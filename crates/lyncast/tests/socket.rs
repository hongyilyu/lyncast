//! Who owns the socket: a daemon after one that died, and two daemons that start at once. A
//! session cannot kill a daemon or start two together, so these check only exit codes and the
//! socket. docs/design/daemon.md's "One daemon owns the socket" session holds what they print.

mod common;

use std::process::Command;

use common::{Daemon, Runtime, ask, lyncast, poll};

/// `lyncast daemon --headless` against `runtime`, with no apps to scan.
fn daemon(runtime: &Runtime) -> Command {
    let mut command = lyncast(runtime, ["daemon", "--headless"]);
    command.env("XDG_DATA_DIRS", runtime.path());
    command
}

/// The exit code of `lyncast status`.
fn status(runtime: &Runtime) -> Option<i32> {
    ask(runtime, &["status"]).2
}

/// SIGKILL leaves the socket behind. A client then finds no daemon, and the next daemon takes
/// the lock the kernel released, removes the old socket, and serves.
#[test]
fn a_daemon_starts_after_one_killed_with_sigkill() {
    let runtime = Runtime::new();
    let first = Daemon::start(daemon(&runtime), &runtime).unwrap();
    assert_eq!(status(&runtime), Some(0), "the first daemon serves");
    drop(first);
    assert!(
        runtime.socket().exists(),
        "the killed daemon left its socket"
    );
    assert_eq!(status(&runtime), Some(1), "a client finds no daemon");
    let _second = Daemon::start(daemon(&runtime), &runtime).unwrap();
    assert_eq!(status(&runtime), Some(0), "the second daemon serves");
}

/// Exactly one of two daemons started together takes the lock and serves. The other exits 1.
#[test]
fn of_two_daemons_started_at_once_one_serves() {
    let runtime = Runtime::new();
    let mut daemons = [
        Daemon::spawn(daemon(&runtime), &runtime),
        Daemon::spawn(daemon(&runtime), &runtime),
    ];
    let exited = poll(|| {
        let exited: Vec<_> = daemons.iter_mut().filter_map(Daemon::exited).collect();
        (!exited.is_empty()).then_some(exited)
    })
    .unwrap_or_default();
    let [loser] = exited.as_slice() else {
        panic!("one daemon exits, not {exited:?}");
    };
    assert_eq!(loser.code(), Some(1), "the loser exits 1");
    // The loser exits as soon as the lock is taken, before the winner has scanned and bound.
    assert!(
        poll(|| (status(&runtime) == Some(0)).then_some(())).is_some(),
        "the winner serves"
    );
    assert_eq!(
        daemons.iter_mut().filter_map(Daemon::exited).count(),
        1,
        "the winner still runs"
    );
}
