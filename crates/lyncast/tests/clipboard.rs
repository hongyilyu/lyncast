//! Clipboard history end to end. The test reads every `env`, `session`, and `clipboard` block of
//! docs/design/clipboard-and-paste.md and runs each as daemon.md's tests run a session: in a
//! fresh tree, `$TREE`, with a fresh `XDG_RUNTIME_DIR`, `$RUNTIME`, and only the `env` block's
//! variables. A `clipboard` block's runtime directory is that of its own compositor, which
//! `scripts/test-env.sh` starts, and its daemons get the compositor's `WAYLAND_DISPLAY`. Its
//! helper lines run this test binary again as a helper in that environment, so no
//! wl-clipboard-rs call in the test process can reach the developer's own clipboard.

mod common;

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::doc::{self, Block};
use common::{BUDGET, Daemon, Ran, Runtime, World, poll, run_bounded, session, session_with};
use wl_clipboard_rs::copy::{self, MimeSource, MimeType, Source};
use wl_clipboard_rs::paste::Seat;
use wl_clipboard_rs::watch::{self, ClipboardEvent, Watcher};

/// Where each block's tree goes, named by the block's line.
const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/clipboard-trees");

/// The variable that makes this test binary a helper, and names what it does.
const HELPER: &str = "LYNCAST_HELPER";

/// What the helper prints before each line it means, because the test harness prints lines of
/// its own.
const SAID: &str = "helper: ";

/// The doc's blocks, read once per test.
fn fixtures() -> Vec<Block> {
    doc::CLIPBOARD
        .read(&doc::CLIPBOARD.text())
        .unwrap_or_else(|error| panic!("clipboard-and-paste.md: {error}"))
}

/// The blocks of `kind`, which must number `count`, so a lost block fails.
fn blocks<'f>(fixtures: &'f [Block], kind: &str, count: usize) -> Vec<&'f Block> {
    let blocks: Vec<&Block> = fixtures.iter().filter(|block| block.kind == kind).collect();
    assert_eq!(
        blocks.len(),
        count,
        "clipboard-and-paste.md holds {count} {kind} blocks, so a lost block fails"
    );
    blocks
}

/// Removes the trees that a run before this one kept for a block that failed, under `name`.
fn fresh(name: &str) -> PathBuf {
    let trees = Path::new(TREES).join(name);
    match fs::remove_dir_all(&trees) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{trees:?}: {error}"),
        _ => {}
    }
    trees
}

/// Asserts that no line failed.
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
    let failures: Vec<String> = blocks(&fixtures, "session", 6)
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
    let failures: Vec<String> = blocks(&fixtures, "clipboard", 9)
        .into_iter()
        .flat_map(|block| {
            let (runtime, display) = Runtime::compositor();
            let wayland = vec![("WAYLAND_DISPLAY".to_owned(), display)];
            let tree = trees.join(block.line.to_string());
            let world = World::within(&fixtures, tree, runtime, &wayland);
            let mut helpers = Helpers::new(&world);
            let failures = session_with(&world, block, &mut |line| helpers.run(line));
            drop(helpers);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert_passed(&failures);
}

/// How soon after Quit a daemon with a compositor exits: half of Quit's 5 s, as for daemon.md's
/// `wire fifo` blocks, so a Quit that waits out its 5 s for the capture thread fails.
const QUIT_BOUND: Duration = Duration::from_millis(2500);

/// How long after the first stopped seeder's offer the test sends Quit: past the 2 s that its read
/// holds the thread, and before the next read gives up.
const QUIT_AFTER: Duration = Duration::from_millis(2500);

/// Quit cancels the capture thread's `Watcher`, which would otherwise block in its wait for the
/// next offer, and the thread reads no offer that waits behind the one in hand. A block cannot
/// time a Quit, so this test does, with the doc's environment and a compositor of its own. Four
/// seeders stop, so each of their reads holds the thread 2 s. The `Watcher` takes in one go the
/// offers that came while the first read held the thread, so Quit comes [`QUIT_AFTER`] the first
/// offer, while the second is in hand and two more wait. The daemon still exits 0 within
/// [`QUIT_BOUND`], as every block's Quit shows.
#[test]
fn quit_ends_the_capture_thread_at_once() {
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
        for line in [
            &format!("$ seed text/plain;charset=utf-8 {text}"),
            "$ stop-seed",
        ] {
            let ran = helpers.run(line).expect(line);
            assert_eq!(ran, (Vec::new(), Vec::new(), Some(0)), "{line}");
            first.get_or_insert_with(Instant::now);
        }
    }
    let first = first.expect("a seed ran");
    thread::sleep((first + QUIT_AFTER).saturating_duration_since(Instant::now()));
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

/// The helper lines of one `clipboard` block, and the seeders still serving.
struct Helpers<'w> {
    world: &'w World,
    seeders: Vec<Child>,
}

impl Drop for Helpers<'_> {
    fn drop(&mut self) {
        for seeder in &mut self.seeders {
            let _ = seeder.kill();
            let _ = seeder.wait();
        }
    }
}

impl<'w> Helpers<'w> {
    fn new(world: &'w World) -> Helpers<'w> {
        Helpers {
            world,
            seeders: Vec::new(),
        }
    }

    /// Runs a helper line, or gives `None` for any other line.
    fn run(&mut self, line: &str) -> Option<Ran> {
        let words: Vec<&str> = line.strip_prefix("$ ")?.splitn(3, ' ').collect();
        Some(match words.as_slice() {
            ["seed", types, text] => self.seed(types, "LYNCAST_TEXT", text),
            ["seed-file", types, path] => self.seed(types, "LYNCAST_FILE", path),
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
                // SAFETY: kill takes a pid and a signal, and touches no memory.
                let stopped = unsafe { libc::kill(pid, libc::SIGSTOP) };
                (Vec::new(), Vec::new(), Some(stopped))
            }
            ["wait-stderr", ..] => {
                let line = self.world.substitute(&line["$ wait-stderr ".len()..]);
                self.wait_stderr(&line)
            }
            ["stop-compositor"] => {
                let status = self.world.runtime.stop_compositor().unwrap();
                (Vec::new(), Vec::new(), status.code())
            }
            _ => return None,
        })
    }

    /// This test binary as a helper that does `op`, in the block's Wayland environment and
    /// tree, with nothing else from the test's environment.
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

    /// Starts a seeder that offers the text, or the file's bytes, under each of `types` until
    /// another selection replaces it. Waits until the compositor offers them, so a seeder that
    /// fails fails its line with what it printed.
    fn seed(&mut self, types: &str, source: &str, value: &str) -> Ran {
        let mut seeder = self
            .helper("seed")
            .env("LYNCAST_TYPES", types)
            .env(source, value)
            .stdout(Stdio::piped())
            .stderr(fs::File::create(self.world.out.join("seeder.stderr")).unwrap())
            .spawn()
            .unwrap();
        let (lines, said) = mpsc::channel();
        let stdout = seeder.stdout.take().unwrap();
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
                Ok(line) if line == ready => {
                    self.seeders.push(seeder);
                    return (Vec::new(), Vec::new(), Some(0));
                }
                Ok(line) => printed.extend(format!("{line}\n").into_bytes()),
                Err(_) => break,
            }
        }
        let _ = seeder.kill();
        let code = seeder.wait().unwrap().code();
        let stderr = fs::read(self.world.out.join("seeder.stderr")).unwrap();
        (printed, stderr, code)
    }

    /// Waits until `$RUNTIME/daemon.stderr` holds `line`. Fails the line with what it holds
    /// when the wait runs out.
    fn wait_stderr(&self, line: &str) -> Ran {
        let stderr = self.world.runtime.path().join("daemon.stderr");
        let read = || fs::read_to_string(&stderr).unwrap_or_default();
        match poll(|| read().lines().any(|held| held == line).then_some(())) {
            Some(()) => (Vec::new(), Vec::new(), Some(0)),
            None => (Vec::new(), read().into_bytes(), Some(1)),
        }
    }

    /// Waits until `lyncast clip list --json` prints a list that `done` accepts. Fails the line
    /// with the last list when the wait runs out.
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

/// What a helper line prints when what it waits for runs over the test's time budget.
fn over(what: &str) -> Ran {
    let said = format!("{what} ran over the test's time budget\n");
    (Vec::new(), said.into_bytes(), Some(-1))
}

/// Runs only when the test binary runs again as a helper, with `--ignored` and [`HELPER`] set.
/// It does one thing in the environment it was given, prints [`SAID`] before each line it means,
/// and exits.
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
            let bytes = match env::var_os("LYNCAST_FILE") {
                Some(path) => fs::read(path).unwrap(),
                None => var("LYNCAST_TEXT").into_encoded_bytes(),
            };
            let types = var("LYNCAST_TYPES").into_string().unwrap();
            let offered: BTreeSet<String> = types.split(',').map(str::to_owned).collect();
            let sources = offered
                .iter()
                .map(|mime| MimeSource {
                    source: Source::Bytes(bytes.clone().into_boxed_slice()),
                    mime_type: MimeType::Specific(mime.clone()),
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
            options
                .prepare_copy_multi(sources)
                .unwrap()
                .serve()
                .unwrap();
        }
        Some(op) => panic!("the helper does not {op}"),
        None => panic!("the helper does not {op:?}"),
    }
}
