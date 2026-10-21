//! The daemon end to end. The test reads every `env`, `desktop`, `executable`, `session`, and
//! `wire` block of docs/design/daemon.md and runs them as its "The fixtures" section says: each
//! block in a fresh tree, `$TREE`, with a fresh `XDG_RUNTIME_DIR` under `/tmp`, `$RUNTIME`, and
//! only the `env` block's variables.

mod common;

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::doc::{self, Block};
use common::{BUDGET, Daemon, World, fill, mkfifo, poll, run_bounded, session};

/// Where each block's tree goes, named by the block's line.
const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/daemon-trees");

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
    stream.set_read_timeout(Some(BUDGET))?;
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
    /// `record: <line>`: the next line the recorder gets.
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

/// The byte that fills a `wire fifo` block's recorder. No recorded line starts with it.
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
                stream.set_read_timeout(Some(BUDGET)).unwrap();
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
                    fail("the request took no effect within the test's time budget".to_owned());
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
    fill(&mut pipe, FILLER);
    pipe
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
        (28, 5),
        "daemon.md holds 28 session blocks and 5 wire blocks, so a lost block fails"
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
/// shows: each success line, and each request on a line of its own.
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
            let request = format!(
                "{{\"Action\":{{\"Spawn\":{{\"command\":[\"firefox\",\"{argument}\"]}}}}}}"
            );
            (id, request)
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
                .set_read_timeout(Some(BUDGET))
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
        "the recorder got lines of {:?} bytes that are not the two requests, of {:?}",
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

/// How many Lock Screens wait in the runner when Quit comes.
const BACKLOG: usize = 64;

/// How long Quit may take to reply with [`BACKLOG`] results queued: its 5 s, the one result in
/// hand then, and room for a loaded machine. A Quit that takes every queued result takes over
/// two minutes.
const BACKLOG_QUIT: Duration = Duration::from_secs(30);

/// Quit takes no result once its 5 s pass. A `wire fifo` block runs no shell line, so it cannot
/// write the 64 MiB file that makes each result take seconds to record, and this test does what
/// daemon.md's "Quit finishes the work in flight" section says. It uses the first `wire fifo`
/// block's requests and replies and the setup lines of the session with the file of 64 MiB.
#[test]
fn quit_stops_taking_results_at_its_deadline() {
    let fixtures = doc::DAEMON.read(&doc::DAEMON.text()).unwrap();
    let fifo_block = fixtures
        .iter()
        .find(|block| block.kind == "wire" && block.args == ["fifo"])
        .expect("daemon.md has a wire fifo block");
    let steps: Vec<Step> = fifo_block
        .lines
        .iter()
        .map(|line| Step::read(line).unwrap())
        .collect();
    let sent = |label: usize| {
        steps.iter().find_map(|step| match step {
            Step::Send(at, request) if *at == label => Some(*request),
            _ => None,
        })
    };
    let reply = |label: usize| {
        steps.iter().find_map(|step| match step {
            Step::Reply(at, reply) if *at == label => Some(*reply),
            _ => None,
        })
    };
    let record = steps.iter().find_map(|step| match step {
        Step::Record(argv) => Some(*argv),
        _ => None,
    });
    let (Some(lock_screen), Some(ran), Some(quit), Some(quit_ran), Some(quitting), Some(record)) =
        (sent(1), reply(1), sent(2), reply(2), reply(3), record)
    else {
        panic!("the wire fifo block's connections 1 and 2 send and read, and 3 reads");
    };
    let large = fixtures
        .iter()
        .find(|block| block.kind == "session" && block.args == ["large-file"])
        .expect("daemon.md has the large-file session");
    let world = World::scratch(&fixtures, "daemon-backlog");
    for line in large
        .lines
        .iter()
        .take_while(|line| !line.starts_with("$ lyncast daemon"))
        .filter(|line| line.starts_with("$ "))
    {
        let (command, _) = world.command(line);
        let ran = run_bounded(command, &world.out).map(|(_, _, code)| code);
        assert_eq!(ran, Some(Some(0)), "{line}");
    }
    let fifo = world.runtime.runs();
    let mut pipe = full_fifo(&fifo);
    let (command, _) = world.command("$ lyncast daemon --headless");
    let daemon = Daemon::start(command, &world.runtime).unwrap();
    let connect = |request: &str| {
        let mut stream = UnixStream::connect(world.runtime.socket()).unwrap();
        stream.set_read_timeout(Some(BACKLOG_QUIT)).unwrap();
        stream.write_all(format!("{request}\n").as_bytes()).unwrap();
        stream
    };
    let jobs: Vec<UnixStream> = (0..BACKLOG).map(|_| connect(lock_screen)).collect();
    assert!(
        poll(|| (held_open(daemon.id(), &fifo) == BACKLOG).then_some(())).is_some(),
        "{BACKLOG} Lock Screens wait in the runner"
    );
    let quit = connect(quit);
    assert!(
        poll(|| (!world.runtime.socket().exists()).then_some(())).is_some(),
        "Quit removes the socket"
    );
    let mut read = Vec::new();
    for _ in 0..BACKLOG {
        assert_eq!(record_line(&mut pipe, &mut read).as_deref(), Some(record));
    }
    let mut got = String::new();
    let result = BufReader::new(&quit).read_line(&mut got);
    assert_eq!(
        got,
        format!("{quit_ran}\n"),
        "Quit's reply within {BACKLOG_QUIT:?}: {result:?}"
    );
    let mut refused = 0;
    for job in &jobs {
        let mut got = String::new();
        let result = BufReader::new(job).read_line(&mut got);
        match got.strip_suffix('\n') {
            Some(line) if line == ran => {}
            Some(line) if line == quitting => refused += 1,
            _ => panic!("a Lock Screen got {result:?} {got:?}"),
        }
    }
    assert!(refused > 0, "a result still queued after the 5 s fails");
    assert_eq!(daemon.exit(), Some(0));
    world.passed();
}

const STALLED_RUNS: usize = 5;

#[test]
fn quit_replies_while_stderr_does_not_drain() {
    let fixtures = doc::DAEMON.read(&doc::DAEMON.text()).unwrap();
    let block = fixtures
        .iter()
        .find(|block| block.kind == "session" && block.args == ["stalled"])
        .expect("daemon.md has the stalled session");
    // SAFETY: sched_getcpu reads no memory, and the set lives through sched_setaffinity, whose
    // pid 0 is this thread.
    let pinned = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(usize::try_from(libc::sched_getcpu()).unwrap(), &mut set);
        libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set)
    };
    assert_eq!(
        pinned,
        0,
        "sched_setaffinity: {}",
        io::Error::last_os_error()
    );
    for run in 0..STALLED_RUNS {
        let world = World::scratch(&fixtures, "daemon-stalled");
        let failures = session(&world, block);
        assert!(failures.is_empty(), "run {run}:\n{}", failures.join("\n"));
        world.passed();
    }
}

/// A thread takes its name only once it runs, so `named_thread` waits for the name.
#[test]
fn a_thread_is_found_by_a_name_that_it_takes_late() {
    let (stop, stopped) = mpsc::channel::<()>();
    let late = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        // SAFETY: PR_SET_NAME reads the NUL-terminated name, which lives through the call.
        unsafe { libc::prctl(libc::PR_SET_NAME, c"late".as_ptr()) };
        let _ = stopped.recv();
    });
    let found = common::named_thread(std::process::id(), "late");
    drop(stop);
    late.join().unwrap();
    assert!(found.is_ok(), "{found:?}");
}

/// How many daemons start while ranking.json is swapped. A daemon whose open waited on a FIFO
/// failed to bind in 9 of 40 such starts, as docs/decisions.md records, so all 40 pass by chance
/// about once in 27,000 runs.
const SWAPPED_STARTS: usize = 40;

/// A FIFO swapped in for ranking.json while the daemon starts never keeps it from binding. A
/// session cannot swap the file during a start, so this test keeps exchanging ranking.json, a
/// link to a file, with a link to a FIFO while each daemon starts, and expects each to bind, as
/// a session's daemon does.
#[test]
fn a_fifo_swapped_in_for_ranking_json_never_blocks_a_start() {
    let fixtures = doc::DAEMON.read(&doc::DAEMON.text()).unwrap();
    for start in 0..SWAPPED_STARTS {
        let world = World::scratch(&fixtures, "daemon-swaps");
        let dir = world.tree.join("data/lyncast");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file"), "{}").unwrap();
        mkfifo(&dir.join("fifo"), 0o600);
        symlink("file", dir.join("ranking.json")).unwrap();
        symlink("fifo", dir.join("other")).unwrap();
        let [ranking, other] = ["ranking.json", "other"]
            .map(|name| CString::new(dir.join(name).as_os_str().as_bytes()).unwrap());
        let stop = AtomicBool::new(false);
        let started = thread::scope(|scope| {
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    // SAFETY: both paths are NUL-terminated strings that live through the call.
                    let swapped = unsafe {
                        libc::renameat2(
                            libc::AT_FDCWD,
                            ranking.as_ptr(),
                            libc::AT_FDCWD,
                            other.as_ptr(),
                            libc::RENAME_EXCHANGE,
                        )
                    };
                    // The daemon moved ranking.json aside, so nothing is left to swap.
                    if swapped != 0 {
                        break;
                    }
                }
            });
            let (command, _) = world.command("$ lyncast daemon --headless");
            let started = Daemon::start(command, &world.runtime);
            stop.store(true, Ordering::Relaxed);
            started
        });
        match started {
            Ok(daemon) => drop(daemon),
            Err(error) => panic!("start {start}: {error}"),
        }
        world.passed();
    }
}
