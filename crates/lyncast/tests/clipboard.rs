//! A `clipboard` block's helper lines run this test binary again as a helper in the block's
//! environment, so no wl-clipboard-rs call in the test process can reach the developer's own
//! clipboard.

mod common;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::doc::{self, Block};
use common::{
    BUDGET, Daemon, Ran, Runtime, World, named_thread, poll, run_bounded, session, session_with,
};
use wl_clipboard_rs::copy::{self, MimeSource, MimeType, Source};
use wl_clipboard_rs::paste::Seat;
use wl_clipboard_rs::watch::{self, ClipboardEvent, Watcher};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/clipboard-trees");

const HELPER: &str = "LYNCAST_HELPER";

/// The name of the daemon's capture thread.
const CAPTURE: &str = "capture";

/// What the helper prints before each line it means, because the test harness prints lines of
/// its own.
const SAID: &str = "helper: ";

fn fixtures() -> Vec<Block> {
    doc::CLIPBOARD
        .read(&doc::CLIPBOARD.text())
        .unwrap_or_else(|error| panic!("clipboard-and-paste.md: {error}"))
}

fn blocks<'f>(fixtures: &'f [Block], kind: &str, count: usize) -> Vec<&'f Block> {
    let blocks: Vec<&Block> = fixtures.iter().filter(|block| block.kind == kind).collect();
    assert_eq!(
        blocks.len(),
        count,
        "clipboard-and-paste.md holds {count} {kind} blocks, so a lost block fails"
    );
    blocks
}

fn fresh(name: &str) -> PathBuf {
    let trees = Path::new(TREES).join(name);
    match fs::remove_dir_all(&trees) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{trees:?}: {error}"),
        _ => {}
    }
    trees
}

fn assert_passed(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} clipboard-and-paste.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_session_block_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let trees = fresh("session");
    let failures: Vec<String> = blocks(&fixtures, "session", 12)
        .into_iter()
        .flat_map(|block| {
            let world = World::new(&fixtures, trees.join(block.line.to_string()));
            let failures = session(&world, block);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert_passed(&failures);
}

#[test]
fn every_clipboard_block_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let trees = fresh("clipboard");
    let failures: Vec<String> = blocks(&fixtures, "clipboard", 16)
        .into_iter()
        .flat_map(|block| {
            let (runtime, display) = Runtime::compositor();
            let wayland = vec![("WAYLAND_DISPLAY".to_owned(), display)];
            let tree = trees.join(block.line.to_string());
            let world = World::within(&fixtures, tree, runtime, &wayland);
            let mut helpers = Helpers::new(&world);
            let failures =
                session_with(&world, block, &mut |line, newest| helpers.run(line, newest));
            drop(helpers);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert_passed(&failures);
}

const QUIT_BOUND: Duration = Duration::from_millis(2500);

const QUIT_AFTER: Duration = Duration::from_millis(2500);

#[test]
fn quit_reads_no_offer_queued_behind_the_one_in_hand() {
    let fixtures = fixtures();
    let (runtime, display) = Runtime::compositor();
    let wayland = [("WAYLAND_DISPLAY".to_owned(), display)];
    let tree = fresh("quit").join("tree");
    let world = World::within(&fixtures, tree, runtime, &wayland);
    let (daemon, _) = world.command("$ lyncast daemon --headless");
    let mut daemon = Daemon::start(daemon, &world.runtime).unwrap();
    let mut helpers = Helpers::new(&world);
    let mut first = None;
    for text in ["one", "two", "three", "four"] {
        let frozen = common::Frozen::new(daemon.id(), CAPTURE).unwrap();
        for line in [
            &format!("$ seed text/plain;charset=utf-8 {text}"),
            "$ stop-seed",
        ] {
            let ran = helpers.run(line, Some(daemon.id())).expect(line);
            assert_eq!(ran, (Vec::new(), Vec::new(), Some(0)), "{line}");
        }
        drop(frozen);
        first.get_or_insert_with(Instant::now);
    }
    let first = first.expect("a seed ran");
    thread::sleep((first + QUIT_AFTER).saturating_duration_since(Instant::now()));
    let (list, _) = world.command("$ lyncast clip list --json");
    let (rows, _, _) = run_bounded(list, &world.out).expect("the list replies");
    assert_eq!(String::from_utf8_lossy(&rows), "", "a seed was read");
    let (quit, _) = world.command("$ lyncast activate cmd:quit");
    let sent = Instant::now();
    let quit = run_bounded(quit, &world.out).expect("Quit replies");
    let finished = daemon.finish();
    let took = sent.elapsed();
    assert_eq!(quit, (b"ran cmd:quit\n".to_vec(), Vec::new(), Some(0)));
    assert_eq!(finished, Some((Vec::new(), Some(0))));
    assert!(took < QUIT_BOUND, "the daemon exited {took:?} after Quit");
    drop(helpers);
    world.passed();
}

struct Helpers<'w> {
    world: &'w World,
    seeders: Vec<Child>,
    compositors: Vec<Child>,
    /// Joined before the seeders are reaped, so no SIGCONT reaches a pid used again.
    continuing: Vec<thread::JoinHandle<()>>,
}

impl Drop for Helpers<'_> {
    fn drop(&mut self) {
        for continuing in self.continuing.drain(..) {
            let _ = continuing.join();
        }
        for helper in self.seeders.iter_mut().chain(&mut self.compositors) {
            let _ = helper.kill();
            let _ = helper.wait();
        }
    }
}

impl<'w> Helpers<'w> {
    fn new(world: &'w World) -> Helpers<'w> {
        Helpers {
            world,
            seeders: Vec::new(),
            compositors: Vec::new(),
            continuing: Vec::new(),
        }
    }

    fn run(&mut self, line: &str, newest: Option<u32>) -> Option<Ran> {
        let words: Vec<&str> = line.strip_prefix("$ ")?.splitn(3, ' ').collect();
        Some(match words.as_slice() {
            ["seed", types, text] => self.seed(types, "LYNCAST_TEXT", text),
            ["seed-file", types, path] => self.seed(types, "LYNCAST_FILE", path),
            ["seed-read", types, path] => {
                let mut seeder = self.helper("seed");
                seeder
                    .env("LYNCAST_TYPES", types)
                    .env("LYNCAST_FILE", path)
                    .env("LYNCAST_ONCE", "1");
                match run_bounded(seeder, &self.world.out) {
                    Some((_, stderr, code)) => (Vec::new(), stderr, code),
                    None => over("the helper"),
                }
            }
            ["seed-each", ..] => {
                let words: Vec<&str> = line["$ seed-each ".len()..].split(' ').collect();
                let pairs = words.chunks(2);
                let types: Vec<&str> = pairs.clone().map(|pair| pair[0]).collect();
                let files: Vec<&str> = pairs.filter_map(|pair| pair.get(1).copied()).collect();
                assert_eq!(
                    types.len(),
                    files.len(),
                    "seed-each <type> <path> ...: {line}"
                );
                self.seed(&types.join(","), "LYNCAST_FILES", &files.join(","))
            }
            ["wait-clips", count] => {
                let count: usize = count.parse().expect("wait-clips <n>");
                self.wait_list(|list| list.lines().count() == count)
            }
            ["wait-first", id] => {
                let first = format!("{{\"id\":{id},");
                self.wait_list(|list| list.starts_with(&first))
            }
            ["stop-seed"] => {
                let seeder = self.seeders.last().expect("stop-seed follows a seed");
                let pid = libc::pid_t::try_from(seeder.id()).unwrap();
                // The kernel stops each thread of the seeder only once it next runs, and waitpid
                // reports the stop once all of them have stopped.
                // SAFETY: kill and waitpid take a pid, a signal, and flags, and waitpid writes only
                // `status`, which lives through the call.
                let stopped = unsafe {
                    let mut status = 0;
                    libc::kill(pid, libc::SIGSTOP) == 0
                        && libc::waitpid(pid, &mut status, libc::WUNTRACED) == pid
                        && libc::WIFSTOPPED(status)
                };
                if stopped {
                    (Vec::new(), Vec::new(), Some(0))
                } else {
                    let why = format!(
                        "seeder {pid} did not stop: {}\n",
                        io::Error::last_os_error()
                    );
                    (Vec::new(), why.into_bytes(), Some(1))
                }
            }
            ["cont-seed-at-quit"] => {
                let seeder = self
                    .seeders
                    .last()
                    .expect("cont-seed-at-quit follows a seed");
                let pid = libc::pid_t::try_from(seeder.id()).unwrap();
                let socket = self.world.runtime.socket();
                self.continuing.push(thread::spawn(move || {
                    poll(|| (!socket.exists()).then_some(()));
                    // SAFETY: kill takes a pid and a signal, and touches no memory.
                    unsafe { libc::kill(pid, libc::SIGCONT) };
                }));
                (Vec::new(), Vec::new(), Some(0))
            }
            ["wait-read"] => match newest {
                Some(pid) => wait_read(pid),
                None => (Vec::new(), b"no daemon to wait for\n".to_vec(), Some(1)),
            },
            ["wait-stderr", ..] => {
                let line = self.world.substitute(&line["$ wait-stderr ".len()..]);
                self.wait_stderr(&line)
            }
            ["stop-compositor"] => {
                let status = self.world.runtime.stop_compositor().unwrap();
                (Vec::new(), Vec::new(), status.code())
            }
            ["fake-compositor", name, ..] => {
                let mut compositor = self.helper("compositor");
                let globals = words.get(2).copied().unwrap_or_default();
                compositor
                    .env("LYNCAST_SOCKET", name)
                    .env("LYNCAST_GLOBALS", globals);
                match self.ready(compositor) {
                    Ok(compositor) => {
                        self.compositors.push(compositor);
                        (Vec::new(), Vec::new(), Some(0))
                    }
                    Err(failed) => failed,
                }
            }
            _ => return None,
        })
    }

    fn helper(&self, op: &str) -> Command {
        let mut command = Command::new(env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "helper",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
                "--quiet",
            ])
            .env_clear()
            .env("XDG_RUNTIME_DIR", self.world.runtime.path())
            .envs(
                self.world
                    .env
                    .iter()
                    .filter(|(name, _)| name == "WAYLAND_DISPLAY")
                    .map(|(name, value)| (name, value)),
            )
            .env(HELPER, op)
            .current_dir(&self.world.tree)
            .stdin(Stdio::null());
        command
    }

    fn seed(&mut self, types: &str, source: &str, value: &str) -> Ran {
        let mut seeder = self.helper("seed");
        seeder.env("LYNCAST_TYPES", types).env(source, value);
        match self.ready(seeder) {
            Ok(seeder) => {
                self.seeders.push(seeder);
                (Vec::new(), Vec::new(), Some(0))
            }
            Err(failed) => failed,
        }
    }

    fn ready(&self, mut helper: Command) -> Result<Child, Ran> {
        let stderr = self.world.out.join("helper.stderr");
        let mut child = helper
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let (lines, said) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = format!("{SAID}ready");
        let deadline = Instant::now() + BUDGET;
        let mut printed = Vec::new();
        loop {
            let wait = deadline.saturating_duration_since(Instant::now());
            match said.recv_timeout(wait) {
                Ok(line) if line == ready => return Ok(child),
                Ok(line) => printed.extend(format!("{line}\n").into_bytes()),
                Err(_) => break,
            }
        }
        let _ = child.kill();
        let code = child.wait().unwrap().code();
        Err((printed, fs::read(stderr).unwrap(), code))
    }

    fn wait_stderr(&self, line: &str) -> Ran {
        let stderr = self.world.runtime.path().join("daemon.stderr");
        let read = || fs::read_to_string(&stderr).unwrap_or_default();
        match poll(|| read().lines().any(|held| held == line).then_some(())) {
            Some(()) => (Vec::new(), Vec::new(), Some(0)),
            None => (Vec::new(), read().into_bytes(), Some(1)),
        }
    }

    fn wait_list(&self, done: impl Fn(&str) -> bool) -> Ran {
        let deadline = Instant::now() + BUDGET;
        loop {
            let (command, _) = self.world.command("$ lyncast clip list --json");
            let Some((stdout, stderr, code)) = run_bounded(command, &self.world.out) else {
                return over("lyncast clip list");
            };
            let list = String::from_utf8_lossy(&stdout);
            if code == Some(0) && done(&list) {
                return (Vec::new(), Vec::new(), Some(0));
            }
            if Instant::now() > deadline {
                let failed = match code {
                    Some(0) => 1,
                    code => code.unwrap_or(-1),
                };
                return (stdout, stderr, Some(failed));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Waits, as [`poll`] does, until the capture thread of process `pid` waits in `poll` or `ppoll`
/// on one descriptor, as it does only while it reads an offer. `/proc/<pid>/task/<tid>/syscall`
/// gives the number of the call a thread waits in and then its arguments, the second of which
/// counts the descriptors.
fn wait_read(pid: u32) -> Ran {
    let failed = |why: String| (Vec::new(), format!("{why}\n").into_bytes(), Some(1));
    let tid = match named_thread(pid, CAPTURE) {
        Ok(tid) => tid,
        Err(why) => return failed(why),
    };
    let syscall = format!("/proc/{pid}/task/{tid}/syscall");
    let reading = || {
        let call = fs::read_to_string(&syscall).unwrap_or_default();
        let mut words = call.split(' ');
        let number = words
            .next()
            .and_then(|number| number.parse::<libc::c_long>().ok());
        matches!(number, Some(libc::SYS_poll | libc::SYS_ppoll)) && words.nth(1) == Some("0x1")
    };
    match poll(|| reading().then_some(())) {
        Some(()) => (Vec::new(), Vec::new(), Some(0)),
        None => failed(format!(
            "the capture thread of process {pid} reads no offer"
        )),
    }
}

fn over(what: &str) -> Ran {
    let said = format!("{what} ran over the test's time budget\n");
    (Vec::new(), said.into_bytes(), Some(-1))
}

#[test]
#[ignore = "the clipboard tests run it as a helper"]
fn helper() {
    let Some(op) = env::var_os(HELPER) else {
        return;
    };
    let var = |name: &str| env::var_os(name).unwrap_or_else(|| panic!("the helper needs {name}"));
    let say = |line: &str| {
        let mut out = io::stdout().lock();
        writeln!(out, "{SAID}{line}")
            .and_then(|()| out.flush())
            .unwrap();
    };
    match op.to_str() {
        Some("seed") => {
            let types = var("LYNCAST_TYPES").into_string().unwrap();
            let types: Vec<&str> = types.split(',').collect();
            let bytes: Vec<Vec<u8>> = match env::var_os("LYNCAST_FILES") {
                Some(paths) => paths
                    .to_str()
                    .unwrap()
                    .split(',')
                    .map(|path| fs::read(path).unwrap())
                    .collect(),
                None => {
                    let bytes = match env::var_os("LYNCAST_FILE") {
                        Some(path) => fs::read(path).unwrap(),
                        None => var("LYNCAST_TEXT").into_encoded_bytes(),
                    };
                    vec![bytes; types.len()]
                }
            };
            let offered: BTreeSet<String> = types.iter().map(|&mime| mime.to_owned()).collect();
            let sources = types
                .iter()
                .zip(bytes)
                .map(|(mime, bytes)| MimeSource {
                    source: Source::Bytes(bytes.into_boxed_slice()),
                    mime_type: MimeType::Specific((*mime).to_owned()),
                })
                .collect();
            // wl-clipboard-rs sends the selection only once it serves, so a watcher of its own
            // says when the compositor has taken it: the first change after the selection that
            // was there.
            let (watching, watched) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let mut watcher =
                    Watcher::new(watch::ClipboardType::Regular, Seat::Unspecified).unwrap();
                watcher.next_event().unwrap();
                watching.send(()).unwrap();
                loop {
                    if let Some(ClipboardEvent::Changed { mime_types, .. }) =
                        watcher.next_event().unwrap()
                        && mime_types.into_iter().collect::<BTreeSet<_>>() == offered
                    {
                        say("ready");
                        return;
                    }
                }
            });
            watched.recv().unwrap();
            let mut options = copy::Options::new();
            options
                .foreground(true)
                .omit_additional_text_mime_types(true);
            if env::var_os("LYNCAST_ONCE").is_some() {
                options.serve_requests(copy::ServeRequests::Only(1));
            }
            options
                .prepare_copy_multi(sources)
                .unwrap()
                .serve()
                .unwrap();
        }
        Some("compositor") => {
            let socket = Path::new(&var("XDG_RUNTIME_DIR")).join(var("LYNCAST_SOCKET"));
            let globals = var("LYNCAST_GLOBALS").into_string().unwrap();
            let words: Vec<&str> = globals.split_whitespace().collect();
            let globals: Vec<(&str, u32)> = words
                .chunks(2)
                .map(|pair| (pair[0], pair[1].parse().unwrap()))
                .collect();
            let listener = UnixListener::bind(socket).unwrap();
            say("ready");
            for stream in listener.incoming() {
                serve_globals(stream.unwrap(), &globals);
            }
        }
        Some(op) => panic!("the helper does not {op}"),
        None => panic!("the helper does not {op:?}"),
    }
}

fn serve_globals(mut stream: UnixStream, globals: &[(&str, u32)]) {
    const DISPLAY: u32 = 1;
    const SYNC: u32 = 0;
    const GET_REGISTRY: u32 = 1;
    const DONE: u32 = 0;
    const DELETE_ID: u32 = 1;
    const GLOBAL: u32 = 0;
    let message = |object: u32, opcode: u32, body: &[u8]| {
        let size = u32::try_from(8 + body.len()).unwrap();
        let mut bytes = [object.to_ne_bytes(), (size << 16 | opcode).to_ne_bytes()].concat();
        bytes.extend_from_slice(body);
        bytes
    };
    let mut header = [0; 8];
    while stream.read_exact(&mut header).is_ok() {
        let word = |at: usize| u32::from_ne_bytes(header[at..at + 4].try_into().unwrap());
        let (object, size, opcode) = (word(0), word(4) >> 16, word(4) & 0xffff);
        let mut body = vec![0; size as usize - 8];
        stream.read_exact(&mut body).unwrap();
        let new_id = || u32::from_ne_bytes(body[..4].try_into().unwrap());
        let reply = match (object, opcode) {
            (DISPLAY, SYNC) => [
                message(new_id(), DONE, &0u32.to_ne_bytes()),
                message(DISPLAY, DELETE_ID, &new_id().to_ne_bytes()),
            ]
            .concat(),
            (DISPLAY, GET_REGISTRY) => (1..)
                .zip(globals)
                .flat_map(|(name, (interface, version))| {
                    let mut text = interface.as_bytes().to_vec();
                    text.push(0);
                    let length = u32::try_from(text.len()).unwrap();
                    text.resize(text.len().next_multiple_of(4), 0);
                    let body = [
                        &u32::to_ne_bytes(name)[..],
                        &length.to_ne_bytes(),
                        &text,
                        &version.to_ne_bytes(),
                    ]
                    .concat();
                    message(new_id(), GLOBAL, &body)
                })
                .collect(),
            _ => Vec::new(),
        };
        stream.write_all(&reply).unwrap();
    }
}
