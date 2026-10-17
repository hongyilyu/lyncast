//! A daemon's thread held still with ptrace, and the `freeze-thread` and `thaw-thread` lines that
//! hold one.

use std::fs;
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use super::{Daemon, OK, Ran, failed, poll};

/// Only the thread that seized a thread may let it go, so a thread of the test holds it: it lets
/// it go when told, or reaps it once it has died with the daemon, since a daemon whose held thread
/// is not reaped cannot be waited for.
pub struct Frozen {
    thaw: Option<Sender<Duration>>,
    holder: Option<thread::JoinHandle<()>>,
}

impl Frozen {
    pub fn new(pid: u32, name: &str) -> Result<Frozen, String> {
        let tid = named_thread(pid, name)?;
        let (held, holding) = mpsc::channel();
        let (thaw, thawed) = mpsc::channel();
        let holder = thread::spawn(move || hold(tid, &held, &thawed));
        holding
            .recv()
            .unwrap_or_else(|_| Err("the holder ended".to_owned()))?;
        Ok(Frozen {
            thaw: Some(thaw),
            holder: Some(holder),
        })
    }

    pub fn thaw(&self, after: Duration) {
        if let Some(thaw) = &self.thaw {
            let _ = thaw.send(after);
        }
    }
}

impl Drop for Frozen {
    fn drop(&mut self) {
        drop(self.thaw.take());
        if let Some(holder) = self.holder.take() {
            let _ = holder.join();
        }
    }
}

/// The threads that a block's `freeze-thread` lines hold, the newest last.
#[derive(Default)]
pub(super) struct Holds(Vec<Frozen>);

impl Holds {
    /// Runs a `freeze-thread <name>` line, which holds the newest daemon's thread named `name`, or
    /// a `thaw-thread [<n>s|<n>ms]` line, which lets the newest held thread go that much later,
    /// and gives what it printed, or `None` for any other line.
    pub(super) fn line(&mut self, line: &str, newest: Option<&Daemon>) -> Option<Ran> {
        let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
        Some(match words.as_slice() {
            ["freeze-thread", name] => match newest.map(|daemon| Frozen::new(daemon.id(), name)) {
                Some(Ok(held)) => {
                    self.0.push(held);
                    OK
                }
                Some(Err(why)) => failed(why),
                None => failed("no daemon to freeze"),
            },
            ["thaw-thread", after @ ..] => {
                let after = match after {
                    [] => Duration::ZERO,
                    [after] => match after.strip_suffix("ms") {
                        Some(millis) => millis.parse().ok().map(Duration::from_millis),
                        None => after
                            .strip_suffix('s')
                            .and_then(|seconds| seconds.parse().ok())
                            .map(Duration::from_secs),
                    }
                    .unwrap_or_else(|| panic!("not thaw-thread <n>s or <n>ms: {line}")),
                    _ => panic!("not thaw-thread [<n>s|<n>ms]: {line}"),
                };
                match self.0.last() {
                    Some(held) => {
                        held.thaw(after);
                        OK
                    }
                    None => failed("no thread is frozen"),
                }
            }
            _ => return None,
        })
    }
}

/// The thread of process `pid` that is named `name`. `main` is the main thread, whose ID is the
/// process ID, because a thread that the daemon starts without a name has the main thread's name.
/// Any other thread takes its name only once it runs, after the call that started it has
/// returned, so the lookup waits for the name as [`poll`] does.
pub fn named_thread(pid: u32, name: &str) -> Result<libc::pid_t, String> {
    if name == "main" {
        return libc::pid_t::try_from(pid).map_err(|error| error.to_string());
    }
    let tasks = format!("/proc/{pid}/task");
    poll(|| match fs::read_dir(&tasks) {
        Err(error) => Some(Err(format!("{tasks}: {error}"))),
        Ok(entries) => entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let comm = fs::read_to_string(entry.path().join("comm")).ok()?;
                (comm.trim_end() == name).then(|| entry.file_name().to_str()?.parse().ok())?
            })
            .next()
            .map(Ok),
    })
    .unwrap_or_else(|| Err(format!("no thread of process {pid} is named {name}")))
}

fn hold(tid: libc::pid_t, held: &Sender<Result<(), String>>, thawed: &Receiver<Duration>) {
    // SAFETY: each ptrace and waitpid call takes the thread ID and plain values, and waitpid
    // writes only `status`, which lives through the call.
    let stopped = unsafe {
        let mut status = 0;
        libc::ptrace(libc::PTRACE_SEIZE, tid, 0, 0) == 0
            && libc::ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0) == 0
            && libc::waitpid(tid, &mut status, libc::__WALL) == tid
    };
    if !stopped {
        let _ = held.send(Err(format!(
            "ptrace of thread {tid}: {}",
            io::Error::last_os_error()
        )));
        return;
    }
    let _ = held.send(Ok(()));
    let mut until = None;
    loop {
        let mut status = 0;
        // SAFETY: as above.
        if unsafe { libc::waitpid(tid, &mut status, libc::WNOHANG | libc::__WALL) } == tid
            && (libc::WIFEXITED(status) || libc::WIFSIGNALED(status))
        {
            return;
        }
        if until.is_some_and(|until| Instant::now() >= until) {
            break;
        }
        match thawed.recv_timeout(Duration::from_millis(10)) {
            Ok(after) => until = Some(Instant::now() + after),
            Err(RecvTimeoutError::Disconnected) => until = until.or(Some(Instant::now())),
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
    // SAFETY: as above.
    unsafe { libc::ptrace(libc::PTRACE_DETACH, tid, 0, 0) };
}
