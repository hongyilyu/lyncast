//! The daemon end to end. The test reads every `env`, `desktop`, `executable`, `session`, and
//! `wire` block of docs/design/daemon.md and runs them as its "The fixtures" section says: each
//! block in a fresh tree, `$TREE`, with a fresh `XDG_RUNTIME_DIR` under `/tmp`, `$RUNTIME`, and
//! only the `env` block's variables.

mod common;

use std::fs;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use common::doc::{self, Block};
use common::{Daemon, Runtime, command_line, environment, materialize, run_bounded, substitute};

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

    /// The `$ ...` line as a command in this world.
    fn command(&self, line: &str) -> (Command, Vec<String>) {
        let line = command_line(line, &self.tree);
        let mut command = line.command(&line.args, &self.env);
        command.current_dir(&self.tree);
        (command, line.args)
    }
}

/// What a command printed, each line with its newline.
#[derive(Debug, Default, PartialEq, Eq)]
struct Printed {
    stdout: String,
    stderr: String,
    exit: i32,
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
        if want.is_empty() && args.first().is_some_and(|arg| arg == "daemon") {
            match Daemon::start(run, &world.runtime) {
                Ok(daemon) => daemons.push(daemon),
                Err(error) => {
                    failures.push(format!("line {}: {command}\n  {error}", block.line));
                    break;
                }
            }
            continue;
        }
        let Some((stdout, stderr, code)) = run_bounded(run, &world.out) else {
            failures.push(format!("line {}: {command}\n  ran over 10 s", block.line));
            break;
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let got = Printed {
            stdout: text(&stdout),
            stderr: text(&stderr),
            exit: code.unwrap_or(-1),
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
/// the reply with the `<` line after it.
fn wire(world: &World, block: &Block) -> Vec<String> {
    let framings: &[Framing] = match block.args.as_slice() {
        [] => &[Framing::Line],
        [mode] if mode == "unterminated" => &[Framing::NoNewline, Framing::Padded],
        [mode] if mode == "full" => &[Framing::Full],
        _ => panic!(
            "line {}: a wire block's mode is unterminated or full",
            block.line
        ),
    };
    let (command, _) = world.command("$ lyncast daemon --headless");
    let _daemon = match Daemon::start(command, &world.runtime) {
        Ok(daemon) => daemon,
        Err(error) => return vec![format!("line {}: {error}", block.line)],
    };
    let mut failures = Vec::new();
    let mut request = None;
    for line in &block.lines {
        if let Some(sent) = line.strip_prefix("> ") {
            request = Some(sent);
            continue;
        }
        let (Some(want), Some(sent)) = (line.strip_prefix("< "), request.take()) else {
            panic!("line {}: {line:?} is not the < after a >", block.line);
        };
        for framing in framings {
            let got = send(&world.runtime.socket(), sent, framing);
            if !got.as_ref().is_ok_and(|got| *got == format!("{want}\n")) {
                failures.push(format!(
                    "> {sent} ({framing:?})\n  want {want:?}\n  got  {got:?}"
                ));
            }
        }
    }
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
        (4, 3),
        "daemon.md holds 4 session blocks and 3 wire blocks, so a lost block fails"
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
