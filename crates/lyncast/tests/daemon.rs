//! The daemon end to end. The test reads every `env`, `desktop`, `executable`, `session`, and
//! `wire` block of docs/design/daemon.md and runs them as its "The fixtures" section says: each
//! block in a fresh tree, `$TREE`, with a fresh `XDG_RUNTIME_DIR` under `/tmp`, `$RUNTIME`, and
//! only the `env` block's variables.

mod common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use common::doc::{self, Block};
use common::{
    Daemon, Runtime, command_line, environment, materialize, mkfifo, poll, run_bounded, substitute,
};

/// Where each block's tree goes, named by the block's line.
const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/daemon-trees");

/// One block's tree, runtime directory, and environment.
struct World {
    tree: PathBuf,
    runtime: Runtime,
    env: Vec<(String, String)>,
    /// Where each run's stdout and stderr go, beside the tree.
    out: PathBuf,
}

impl World {
    /// A fresh tree at `tree`.
    fn new(fixtures: &[Block], tree: PathBuf) -> World {
        materialize(fixtures, &tree);
        let runtime = Runtime::new();
        let env = environment(fixtures, &[("$TREE", &tree), ("$RUNTIME", runtime.path())]);
        let out = tree.with_extension("out");
        fs::create_dir_all(&out).unwrap();
        World {
            tree,
            runtime,
            env,
            out,
        }
    }

    /// A world on a fresh tree named `name` under `CARGO_TARGET_TMPDIR`, for a test that no
    /// block drives.
    fn scratch(fixtures: &[Block], name: &str) -> World {
        World::new(fixtures, Path::new(env!("CARGO_TARGET_TMPDIR")).join(name))
    }

    /// Removes the tree and the output beside it once the run in them passes, so a run that
    /// fails keeps them to look at.
    fn passed(self) {
        fs::remove_dir_all(&self.out).unwrap();
        fs::remove_dir_all(&self.tree).unwrap();
    }

    /// `text` with `$TREE` and `$RUNTIME` replaced.
    fn substitute(&self, text: &str) -> String {
        substitute(
            text,
            &[("$TREE", &self.tree), ("$RUNTIME", self.runtime.path())],
        )
    }

    /// The `$ ...` line as a command in this world, and the arguments of a line that runs
    /// lyncast. Any other line runs through `/bin/sh`.
    fn command(&self, line: &str) -> (Command, Option<Vec<String>>) {
        let Some(lyncast) = command_line(line, &self.tree) else {
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(&line[2..])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TREE", &self.tree)
                .env("RUNTIME", self.runtime.path())
                .current_dir(&self.tree);
            return (command, None);
        };
        let mut command = lyncast.command(&lyncast.args, &self.env);
        command.current_dir(&self.tree);
        (command, Some(lyncast.args))
    }

    /// The bytes the recorder file holds now. A recorder that is not a file holds none.
    fn recorded(&self) -> Vec<u8> {
        fs::read(self.runtime.runs()).unwrap_or_default()
    }
}

/// What a command printed, each line with its newline, and the lines it added to the recorder.
#[derive(Debug, Default, PartialEq, Eq)]
struct Printed {
    stdout: String,
    stderr: String,
    exit: i32,
    record: String,
}

impl Printed {
    /// The lines after a command in a session block.
    fn expected(lines: &[String]) -> Printed {
        let mut printed = Printed::default();
        for line in lines {
            if let Some(line) = line.strip_prefix("stderr: ") {
                printed.stderr += &format!("{line}\n");
            } else if let Some(code) = line.strip_prefix("exit: ") {
                printed.exit = code.parse().expect("exit: <n>");
            } else if let Some(argv) = line.strip_prefix("record: ") {
                printed.record += &format!("{argv}\n");
            } else {
                printed.stdout += &format!("{line}\n");
            }
        }
        printed
    }
}

/// Runs one session block and gives what failed.
fn session(world: &World, block: &Block) -> Vec<String> {
    let mut failures = Vec::new();
    // Kept to the end of the block, which kills any still running.
    let mut daemons = Vec::new();
    let mut rest = block.lines.as_slice();
    while let [command, after @ ..] = rest {
        let printed = after
            .iter()
            .take_while(|line| !line.starts_with("$ "))
            .count();
        let want: Vec<String> = after[..printed]
            .iter()
            .map(|line| world.substitute(line))
            .collect();
        rest = &after[printed..];
        let (run, args) = world.command(command);
        let daemon = args.is_some_and(|args| args.first().is_some_and(|arg| arg == "daemon"));
        if daemon && want.is_empty() {
            match Daemon::start(run, &world.runtime) {
                Ok(daemon) => daemons.push(daemon),
                Err(error) => {
                    failures.push(format!("line {}: {command}\n  {error}", block.line));
                    break;
                }
            }
            continue;
        }
        let before = world.recorded().len();
        let Some((stdout, stderr, code)) = run_bounded(run, &world.out) else {
            failures.push(format!("line {}: {command}\n  ran over 10 s", block.line));
            break;
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let got = Printed {
            stdout: text(&stdout),
            stderr: text(&stderr),
            exit: code.unwrap_or(-1),
            record: text(world.recorded().get(before..).unwrap_or_default()),
        };
        let want = Printed::expected(&want);
        if got != want {
            failures.push(format!(
                "line {}: {command}\n  want {want:?}\n  got  {got:?}",
                block.line
            ));
        }
    }
    failures
}

/// How a `wire` block sends each `>` line.
#[derive(Debug)]
enum Framing {
    /// The line and a newline.
    Line,
    /// The line alone, then a shutdown of the write half.
    NoNewline,
    /// The line padded with spaces to 1 MiB, then `not json` and a newline.
    Padded,
    /// The line padded with spaces to 1 MiB with its newline.
    Full,
}

/// Sends each `>` line on its own connection, framed as the block's mode says, and compares
/// the reply with the `<` line after it and the recorder's new lines with the `record:` lines.
fn wire(world: &World, block: &Block) -> Vec<String> {
    let framings: &[Framing] = match block.args.as_slice() {
        [] => &[Framing::Line],
        [mode] if mode == "unterminated" => &[Framing::NoNewline, Framing::Padded],
        [mode] if mode == "full" => &[Framing::Full],
        [mode] if mode == "fifo" => return wire_fifo(world, block),
        _ => panic!(
            "line {}: a wire block's mode is unterminated, full, or fifo",
            block.line
        ),
    };
    let (command, _) = world.command("$ lyncast daemon --headless");
    let _daemon = match Daemon::start(command, &world.runtime) {
        Ok(daemon) => daemon,
        Err(error) => return vec![format!("line {}: {error}", block.line)],
    };
    let mut failures = Vec::new();
    let mut rest = block.lines.as_slice();
    while let [request, reply, after @ ..] = rest {
        let (Some(request), Some(want)) = (request.strip_prefix("> "), reply.strip_prefix("< "))
        else {
            panic!(
                "line {}: {request:?} then {reply:?} are not > and <",
                block.line
            );
        };
        let records = after
            .iter()
            .take_while(|line| line.starts_with("record: "))
            .count();
        let want_record: String = after[..records]
            .iter()
            .map(|line| format!("{}\n", &line["record: ".len()..]))
            .collect();
        rest = &after[records..];
        for framing in framings {
            let before = world.recorded().len();
            let got = send(&world.runtime.socket(), request, framing);
            let record = String::from_utf8_lossy(&world.recorded()[before..]).into_owned();
            if !got.as_ref().is_ok_and(|got| *got == format!("{want}\n")) || record != want_record {
                failures.push(format!(
                    "> {request} ({framing:?})\n  want {want:?} {want_record:?}\n  got  {got:?} {record:?}"
                ));
            }
        }
    }
    assert!(
        rest.is_empty(),
        "line {}: a wire block pairs > and <",
        block.line
    );
    failures
}

/// Sends `request` on a new connection and reads the reply line.
fn send(socket: &Path, request: &str, framing: &Framing) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    match framing {
        Framing::Line => stream.write_all(format!("{request}\n").as_bytes())?,
        Framing::NoNewline => {
            stream.write_all(request.as_bytes())?;
            stream.shutdown(Shutdown::Write)?;
        }
        Framing::Padded => {
            let mut bytes = request.as_bytes().to_vec();
            bytes.resize(1 << 20, b' ');
            bytes.extend_from_slice(b"not json\n");
            let mut writer = stream.try_clone()?;
            // The daemon stops reading at 1 MiB and may close before the rest is written.
            thread::spawn(move || writer.write_all(&bytes));
        }
        Framing::Full => {
            let mut bytes = request.as_bytes().to_vec();
            bytes.resize((1 << 20) - 1, b' ');
            bytes.push(b'\n');
            stream.write_all(&bytes)?;
        }
    }
    let mut got = String::new();
    BufReader::new(stream).read_line(&mut got)?;
    Ok(got)
}

/// One line of a `wire fifo` block.
enum Step<'b> {
    /// `N> request`: the request, sent on connection N.
    Send(usize, &'b str),
    /// `N< reply`: the reply that connection N reads.
    Reply(usize, &'b str),
    /// `record: argv`: the next line the recorder gets.
    Record(&'b str),
}

impl Step<'_> {
    fn read(line: &str) -> Option<Step<'_>> {
        if let Some(argv) = line.strip_prefix("record: ") {
            return Some(Step::Record(argv));
        }
        let (head, text) = line.split_once(' ')?;
        let (label, arrow) = head.split_at(head.len().checked_sub(1)?);
        let label = label.parse().ok()?;
        match arrow {
            ">" => Some(Step::Send(label, text)),
            "<" => Some(Step::Reply(label, text)),
            _ => None,
        }
    }
}

/// The byte that fills a `wire fifo` block's recorder. No recorded argv starts with it.
const FILLER: u8 = b'#';

/// How soon after its last `record:` line a `wire fifo` block's daemon exits. It is half of
/// Quit's 5 s, so a Quit that waits out its 5 s fails the block.
const QUIT_BOUND: Duration = Duration::from_millis(2500);

/// Runs a `wire fifo` block. Its recorder is a full FIFO, so a program the daemon starts waits
/// in the runner until a `record:` line reads it.
fn wire_fifo(world: &World, block: &Block) -> Vec<String> {
    let steps: Vec<Step> = block
        .lines
        .iter()
        .map(|line| {
            Step::read(line).unwrap_or_else(|| {
                panic!("line {}: {line:?} is not N>, N<, or record:", block.line)
            })
        })
        .collect();
    let fifo = world.runtime.runs();
    let mut pipe = full_fifo(&fifo);
    let (command, _) = world.command("$ lyncast daemon --headless");
    let daemon = match Daemon::start(command, &world.runtime) {
        Ok(daemon) => daemon,
        Err(error) => return vec![format!("line {}: {error}", block.line)],
    };
    let mut connections = BTreeMap::new();
    for step in &steps {
        if let Step::Send(label, _) | Step::Reply(label, _) = step {
            connections.entry(*label).or_insert_with(|| {
                let stream = UnixStream::connect(world.runtime.socket()).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                stream
            });
        }
    }
    let mut failures = Vec::new();
    let mut read = Vec::new();
    let mut released = None;
    for (at, step) in steps.iter().enumerate() {
        let mut fail = |what: String| {
            failures.push(format!(
                "line {}: {}\n  {what}",
                block.line, block.lines[at]
            ));
        };
        match step {
            Step::Send(label, request) => {
                let stream = &connections[label];
                let held = held_open(daemon.id(), &fifo);
                if let Err(error) = (&*stream).write_all(format!("{request}\n").as_bytes()) {
                    fail(format!("not sent: {error}"));
                    continue;
                }
                let took = poll(|| {
                    let took = replied(stream)
                        || held_open(daemon.id(), &fifo) > held
                        || !world.runtime.socket().exists();
                    took.then_some(())
                });
                if took.is_none() {
                    fail("the request took no effect in 10 s".to_owned());
                }
            }
            Step::Reply(label, want) => {
                let mut got = String::new();
                let result = BufReader::new(&connections[label]).read_line(&mut got);
                if got != format!("{want}\n") {
                    fail(format!("got {result:?} {got:?}"));
                }
            }
            Step::Record(want) => {
                for step in &steps[at..] {
                    if let Step::Reply(label, _) = step
                        && replied(&connections[label])
                    {
                        fail(format!("{label} replied while the work was in flight"));
                    }
                }
                let got = record_line(&mut pipe, &mut read);
                released = Some(Instant::now());
                if got.as_deref() != Some(*want) {
                    fail(format!("the recorder got {got:?}"));
                }
            }
        }
    }
    let code = daemon.exit();
    let took = released.map(|released| released.elapsed());
    if code != Some(0) {
        failures.push(format!(
            "line {}: the daemon exited with {code:?}",
            block.line
        ));
    }
    if let Some(took) = took
        && took > QUIT_BOUND
    {
        failures.push(format!(
            "line {}: the daemon exited {took:?} after the last record: line, over {QUIT_BOUND:?}",
            block.line
        ));
    }
    failures
}

/// Makes a FIFO at `path` and fills it, so a program the daemon starts waits in the runner until
/// the test reads the FIFO. Gives an end that reads without waiting.
fn full_fifo(path: &Path) -> File {
    mkfifo(path, 0o600);
    // Open for reading and writing, so neither end waits for the other.
    let mut pipe = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .unwrap();
    fill(&mut pipe);
    pipe
}

/// Writes [`FILLER`] to `pipe` until it is full.
fn fill(pipe: &mut File) {
    for chunk in [4096, 1] {
        loop {
            match pipe.write(&vec![FILLER; chunk]) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => panic!("filling the FIFO: {error}"),
            }
        }
    }
}

/// Reads `pipe` into `read`, as [`poll`] waits, until a whole line follows the filler, and gives
/// that line.
fn record_line(pipe: &mut File, read: &mut Vec<u8>) -> Option<String> {
    poll(|| {
        let mut buffer = [0; 4096];
        match pipe.read(&mut buffer) {
            Ok(count) => read.extend_from_slice(&buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => panic!("reading the FIFO: {error}"),
        }
        let start = read
            .iter()
            .position(|&byte| byte != FILLER)
            .unwrap_or(read.len());
        let end = read[start..].iter().position(|&byte| byte == b'\n')?;
        let line = String::from_utf8_lossy(&read[start..start + end]).into_owned();
        read.drain(..=start + end);
        Some(line)
    })
}

/// How many of the daemon's descriptors are open on `path`.
fn held_open(daemon: u32, path: &Path) -> usize {
    fs::read_dir(format!("/proc/{daemon}/fd"))
        .map(|entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .is_ok_and(|entry| fs::read_link(entry.path()).is_ok_and(|to| to == path))
                })
                .count()
        })
        .unwrap_or(0)
}

/// The connection has a reply to read, or the daemon closed it.
fn replied(stream: &UnixStream) -> bool {
    let mut byte = 0u8;
    // SAFETY: the buffer is one byte that lives through the call.
    let peeked = unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&raw mut byte).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    peeked >= 0
}

/// Runs a session or wire block in a world of its own, which goes once the block passes, so a
/// passing run leaves no file behind.
fn run(fixtures: &[Block], block: &Block) -> Vec<String> {
    let world = World::new(fixtures, Path::new(TREES).join(block.line.to_string()));
    let failures = match block.kind.as_str() {
        "session" => session(&world, block),
        _ => wire(&world, block),
    };
    if failures.is_empty() {
        world.passed();
    }
    failures
}

#[test]
fn every_session_and_wire_block_runs_as_the_doc_says() {
    let text = doc::DAEMON.text();
    let fixtures = doc::DAEMON
        .read(&text)
        .unwrap_or_else(|error| panic!("daemon.md: {error}"));
    let of = |kind| -> Vec<&Block> { fixtures.iter().filter(|block| block.kind == kind).collect() };
    let (sessions, wires) = (of("session"), of("wire"));
    assert_eq!(
        (sessions.len(), wires.len()),
        (13, 4),
        "daemon.md holds 13 session blocks and 4 wire blocks, so a lost block fails"
    );
    // A failed block keeps its tree, which a doc edit can leave under a line that no block has
    // any more, so the trees of the last run go first.
    match fs::remove_dir_all(TREES) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{TREES}: {error}"),
        _ => {}
    }
    let failures: Vec<String> = sessions
        .iter()
        .chain(&wires)
        .flat_map(|block| run(&fixtures, block))
        .collect();
    assert!(
        failures.is_empty(),
        "{} daemon.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Two launches at once, each of an argv longer than the 4 KiB that a FIFO takes in one write,
/// record one whole line each. A session runs one command at a time, so this test starts the
/// two itself, with two apps added to the doc's tree, and expects what the launch session
/// shows: each success line, and each argv as one compact JSON array on a line of its own.
#[test]
fn launches_at_once_record_whole_lines() {
    let fixtures = doc::DAEMON.read(&doc::DAEMON.text()).unwrap();
    let world = World::scratch(&fixtures, "daemon-launches");
    let apps: Vec<(String, String)> = ["x", "y"]
        .into_iter()
        .map(|letter| {
            let id = format!("{letter}.desktop");
            let argument = letter.repeat(18_000);
            let entry = format!(
                "[Desktop Entry]\nType=Application\nName={letter}\nExec=firefox {argument}\n"
            );
            fs::write(world.tree.join("share/applications").join(&id), entry).unwrap();
            (id, format!("[\"firefox\",\"{argument}\"]"))
        })
        .collect();
    let fifo = world.runtime.runs();
    let mut pipe = full_fifo(&fifo);
    let (command, _) = world.command("$ lyncast daemon --headless");
    let daemon = Daemon::start(command, &world.runtime).unwrap();
    let connections: Vec<UnixStream> = apps
        .iter()
        .map(|(id, _)| {
            let mut stream = UnixStream::connect(world.runtime.socket()).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let request = format!(
                "{{\"request\":\"activate\",\"key\":\"app:{id}\",\"via\":{{\"chord\":\"enter\"}},\"confirmed\":false,\"query\":\"\"}}\n"
            );
            stream.write_all(request.as_bytes()).unwrap();
            stream
        })
        .collect();
    assert!(
        poll(|| (held_open(daemon.id(), &fifo) == 2).then_some(())).is_some(),
        "both launches wait in the runner"
    );
    let mut read = Vec::new();
    let mut got: Vec<Option<String>> = (0..2).map(|_| record_line(&mut pipe, &mut read)).collect();
    got.sort();
    let want: Vec<Option<String>> = apps.iter().map(|(_, argv)| Some(argv.clone())).collect();
    // Each line holds 18,000 bytes, so the message gives only their lengths.
    let lengths = |lines: &[Option<String>]| -> Vec<Option<usize>> {
        lines
            .iter()
            .map(|line| line.as_ref().map(String::len))
            .collect()
    };
    assert!(
        got == want,
        "the recorder got lines of {:?} bytes that are not the two argvs, of {:?}",
        lengths(&got),
        lengths(&want)
    );
    for ((id, _), stream) in apps.iter().zip(&connections) {
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        assert_eq!(
            reply,
            format!("{{\"reply\":\"done\",\"message\":\"launched app:{id}\"}}\n")
        );
    }
    drop(daemon);
    world.passed();
}
