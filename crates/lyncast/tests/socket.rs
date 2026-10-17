//! Who owns the socket: a daemon after one that died, two daemons that start at once, and a
//! daemon that cannot start a thread. A session cannot kill a daemon, start two together, or
//! limit a daemon's threads, so these check only exit codes, the `lyncast: ` line, and the
//! socket. docs/design/daemon.md's "One daemon owns the socket" session holds what they print.

mod common;

use std::fs;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

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

/// Gives the process that `command` starts an `RLIMIT_NPROC` of 0, so it can start no thread and
/// no process, unless the kernel exempts its user, as it exempts root.
fn no_threads(command: &mut Command) -> &mut Command {
    // SAFETY: setrlimit is async-signal-safe, and the closure touches only its own locals.
    unsafe {
        command.pre_exec(|| {
            let none = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            match libc::setrlimit(libc::RLIMIT_NPROC, &none) {
                0 => Ok(()),
                _ => Err(io::Error::last_os_error()),
            }
        })
    }
}

/// Whether a process under [`no_threads`] can still start one. `timeout` starts its command in a
/// process of its own, and it fails at once when it cannot.
fn limit_exempt() -> bool {
    let mut timeout = Command::new("timeout");
    timeout
        .args(["10", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_threads(&mut timeout).status().unwrap().success()
}

/// A daemon that cannot start a thread exits 1 with one `lyncast: ` line. Without a data home
/// the accept thread is the first to start, and with one the writer of the visits is. Under an
/// `RLIMIT_NPROC` of 0 no thread starts, unless the kernel exempts the user, as it exempts root,
/// and then the test says it skipped.
#[test]
fn a_daemon_that_cannot_start_a_thread_exits_1() {
    if limit_exempt() {
        // libtest hides what eprintln! prints in a test that passes, and the skip has to show.
        let _ = writeln!(
            io::stderr(),
            "a_daemon_that_cannot_start_a_thread_exits_1 skipped: RLIMIT_NPROC does not limit this user"
        );
        return;
    }
    for data_home in [false, true] {
        let runtime = Runtime::new();
        let mut command = daemon(&runtime);
        if data_home {
            command.env("XDG_DATA_HOME", runtime.path().join("data"));
        }
        no_threads(&mut command);
        let mut daemon = Daemon::spawn(command, &runtime);
        let code = poll(|| daemon.exited()).and_then(|status| status.code());
        let stderr = fs::read_to_string(runtime.path().join("daemon.stderr")).unwrap();
        assert_eq!(code, Some(1), "data home {data_home}: {stderr:?}");
        assert!(
            stderr.starts_with("lyncast: ") && stderr.lines().count() == 1,
            "data home {data_home}: {stderr:?}"
        );
    }
}
