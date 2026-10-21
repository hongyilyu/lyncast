//! The session runner. It runs a block's `$ ...` lines in order and compares what each printed
//! with the lines after it. A `lyncast daemon` line with nothing after it starts a daemon in the
//! background, which later lines reach, and the block's end kills it. Besides lyncast and
//! `/bin/sh` lines, a block may hold the runner's own lines: `stall-stdout`, `stall-stderr`, and
//! `no-threads` shape the next daemon, `drain-stdout`, `drain-stderr`, and `wait-exit` act on the
//! newest one, `freeze-thread` and `thaw-thread` hold one of its threads, [`Connections`] holds
//! connections to it, and `fake-niri` starts a [`FakeNiri`]. A doc's own lines come through
//! [`Verbs`].

use std::io::{self, Write};
use std::mem;
use std::path::Path;

use super::connections::Connections;
use super::frozen::Holds;
use super::niri::{Answer, FakeNiri};
use super::steps::{Printed, Step, steps};
use super::{
    Daemon, NextDaemon, OK, Output, Ran, Threads, World, doc, failed, limit_exempt, run_bounded,
};

/// A command line that a doc's own fixtures define, such as a clipboard block's `seed`, given the
/// newest daemon's process ID, which gives what it printed, or `None` for a line it does not
/// define.
pub type Verbs<'v> = dyn FnMut(&str, Option<u32>) -> Option<Ran> + 'v;

/// Runs one session block and gives what failed.
pub fn session(world: &World, block: &doc::Block) -> Vec<String> {
    session_with(world, block, &mut |_, _| None)
}

/// [`session`] for a block whose lines may also run `verbs`.
pub fn session_with(world: &World, block: &doc::Block, verbs: &mut Verbs<'_>) -> Vec<String> {
    session_later(world, block, verbs, &|_| false)
}

/// [`session_with`] for a block where a line that `later` names may start programs or ask niri
/// after it returns, as keys typed into the palette do. Such a line holds no `record:` or `niri:`
/// lines, and what it starts or asks counts on the next line's.
pub fn session_later(
    world: &World,
    block: &doc::Block,
    verbs: &mut Verbs<'_>,
    later: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut session = Session::default();
    let mut before_later = None;
    for Step {
        line: number,
        command,
        after,
    } in steps(block)
    {
        let newest = session.daemons.last().map(|daemon| daemon.id().to_string());
        let want: Vec<String> = after
            .iter()
            .map(|line| match &newest {
                Some(pid) => world.substitute(line).replace("$DAEMON", pid),
                None => world.substitute(line),
            })
            .collect();
        let before = before_later
            .take()
            .unwrap_or_else(|| (world.recorded().len(), world.niri_read().len()));
        let ran = match session.line(world, command, want.is_empty(), verbs) {
            Line::Ran(Some(ran)) => ran,
            Line::Ran(None) => {
                failures.push(format!(
                    "line {number}: {command}\n  ran over the test's time budget"
                ));
                break;
            }
            Line::Started => continue,
            Line::NotStarted(error) => {
                failures.push(format!("line {number}: {command}\n  {error}"));
                break;
            }
            Line::Skipped(why) => {
                // libtest hides what eprintln! prints in a test that passes, and the skip has to
                // show.
                let _ = writeln!(
                    io::stderr(),
                    "line {number}: the rest of the block skipped: {why}"
                );
                break;
            }
        };
        let (stdout, stderr, code) = ran;
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let (record, niri) = if later(command) {
            before_later = Some(before);
            (String::new(), String::new())
        } else {
            let (recorded, read) = (world.recorded(), world.niri_read());
            (
                text(recorded.get(before.0..).unwrap_or_default()),
                text(read.get(before.1..).unwrap_or_default()),
            )
        };
        let got = Printed {
            stdout: text(&stdout),
            stderr: text(&stderr),
            exit: code.unwrap_or(-1),
            record,
            niri,
        };
        let want = Printed::expected(&want);
        if got != want {
            failures.push(format!(
                "line {number}: {command}\n  want {want:?}\n  got  {got:?}"
            ));
        }
    }
    failures
}

/// What one block's lines keep from one line to the next.
#[derive(Default)]
struct Session {
    /// Kept to the end of the block, which kills any still running.
    daemons: Vec<Daemon>,
    next: NextDaemon,
    holds: Holds,
    connections: Connections,
    /// After the daemons, so it outlives them.
    fake: Option<FakeNiri>,
}

/// What one line did.
enum Line {
    /// It ran, and printed this, or ran over the test's time budget.
    Ran(Option<Ran>),
    /// It started a daemon in the background, which serves.
    Started,
    /// The daemon it started in the background exited, or never served.
    NotStarted(String),
    /// It cannot run here, nor can the lines after it.
    Skipped(&'static str),
}

impl Session {
    /// Runs the line `command`. A `lyncast daemon` line starts its daemon in the background when
    /// `background`, which is when no lines follow it.
    fn line(
        &mut self,
        world: &World,
        command: &str,
        background: bool,
        verbs: &mut Verbs<'_>,
    ) -> Line {
        if let Some(rest) = command.strip_prefix("$ fake-niri")
            && (rest.is_empty() || rest.starts_with(' '))
        {
            self.fake = None;
            let (socket, log) = (world.runtime.niri_socket(), world.runtime.niri_log());
            match rest.trim_start() {
                "gone" => FakeNiri::gone(&socket),
                rest => self.fake = Some(FakeNiri::start(&socket, &log, Answer::read(rest))),
            }
            return Line::Ran(Some(OK));
        }
        let ran = match command {
            "$ stall-stdout" => {
                self.next.stdout = Output::Stalled;
                OK
            }
            "$ stall-stderr" => {
                self.next.stderr = Output::Stalled;
                OK
            }
            "$ no-threads" | "$ deny-threads" if limit_exempt() => {
                return Line::Skipped("RLIMIT_NPROC does not limit this user");
            }
            "$ no-threads" => {
                self.next.threads = Threads::None;
                OK
            }
            "$ drain-stdout" => match self.daemons.last_mut() {
                Some(daemon) => daemon
                    .drain_stdout(&world.runtime)
                    .map_or_else(failed, |()| OK),
                None => failed("no daemon to drain"),
            },
            "$ drain-stderr" => match self.daemons.last_mut().map(Daemon::drain) {
                Some(Ok(())) => OK,
                Some(Err(error)) => failed(error),
                None => failed("no daemon to drain"),
            },
            "$ wait-exit" => {
                let daemon = self.daemons.last_mut().expect("wait-exit follows a daemon");
                let finished = daemon.finish();
                return Line::Ran(finished.map(|(stderr, code)| (Vec::new(), stderr, code)));
            }
            _ => {
                let newest = self.daemons.last();
                let ran = self
                    .holds
                    .line(command, newest)
                    .or_else(|| self.connections.line(command, newest, &world.runtime))
                    .or_else(|| verbs(command, newest.map(Daemon::id)));
                match ran {
                    Some(ran) => ran,
                    None => return self.run(world, command, background),
                }
            }
        };
        Line::Ran(Some(ran))
    }

    /// Runs a lyncast or `/bin/sh` line. A shell line gets the newest daemon's process ID as
    /// `DAEMON`, and a `lyncast daemon` line takes what the lines before it asked of it.
    fn run(&mut self, world: &World, command: &str, background: bool) -> Line {
        let (mut run, lyncast) = world.command(command);
        let Some(lyncast) = lyncast else {
            if let Some(newest) = self.daemons.last() {
                run.env("DAEMON", newest.id().to_string());
            }
            return Line::Ran(run_bounded(run, &world.out));
        };
        if !lyncast.args.first().is_some_and(|arg| arg == "daemon") {
            return Line::Ran(run_bounded(run, &world.out));
        }
        let next = mem::take(&mut self.next);
        if !background {
            return Line::Ran(next.run(run, &world.runtime, &world.out));
        }
        let socket = lyncast.var("XDG_RUNTIME_DIR").map_or_else(
            || world.runtime.socket(),
            |dir| Path::new(dir).join("lyncast.sock"),
        );
        let started = next.spawn(run, &world.runtime);
        match started.serving(&world.runtime, &socket) {
            Ok(daemon) => {
                self.daemons.push(daemon);
                Line::Started
            }
            Err(error) => Line::NotStarted(error),
        }
    }
}
