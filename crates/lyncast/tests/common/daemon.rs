//! Daemons that a test starts, in the background or until they exit, and what a block asks of
//! the next one it starts.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::mem;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::ptr;

use super::{Ran, Runtime, fill, poll, run_bounded, umask};

/// A daemon that a test started in the background. Dropping it kills it, so a test that fails
/// leaves no daemon running.
pub struct Daemon {
    child: Child,
    stderr: fs::File,
    filler: usize,
}

impl Daemon {
    /// Starts `command`, a `lyncast daemon`, in the background with [`Runtime::record`]. Its
    /// stdout and stderr are appended to `daemon.stdout` and `daemon.stderr` in `runtime`.
    pub fn spawn(command: Command, runtime: &Runtime) -> Daemon {
        let stderr = runtime.log("daemon.stderr");
        let mut from_here = fs::File::open(runtime.path().join("daemon.stderr")).unwrap();
        from_here.seek(SeekFrom::End(0)).unwrap();
        Daemon::spawn_with(command, runtime, Stdio::from(stderr), from_here, 0)
    }

    /// [`Daemon::spawn`] with a stderr that does not drain: a pipe that the test fills before the
    /// daemon starts.
    pub fn spawn_stalled(command: Command, runtime: &Runtime) -> Daemon {
        let (reader, mut writer) = io::pipe().unwrap();
        let filler = fill(&mut writer, b'x');
        let reader = fs::File::from(OwnedFd::from(reader));
        Daemon::spawn_with(command, runtime, Stdio::from(writer), reader, filler)
    }

    /// `command` goes once the daemon runs, and with it the test's end of a stderr pipe, so the
    /// pipe ends when the daemon exits.
    fn spawn_with(
        mut command: Command,
        runtime: &Runtime,
        stderr: Stdio,
        from_here: fs::File,
        filler: usize,
    ) -> Daemon {
        runtime.record(&mut command);
        umask(&mut command);
        command
            .stdin(Stdio::null())
            .stdout(runtime.log("daemon.stdout"))
            .stderr(stderr);
        Daemon {
            child: command.spawn().expect("lyncast runs"),
            stderr: from_here,
            filler,
        }
    }

    /// Spawns `command` and waits, as [`poll`] does, until the socket accepts. Fails when the
    /// daemon exits first, or when the wait runs out.
    pub fn start(command: Command, runtime: &Runtime) -> Result<Daemon, String> {
        Daemon::spawn(command, runtime).serving(runtime, &runtime.socket())
    }

    pub fn serving(mut self, runtime: &Runtime, socket: &Path) -> Result<Daemon, String> {
        let accepted = poll(|| {
            if UnixStream::connect(socket).is_ok() {
                return Some(Ok(()));
            }
            self.exited().map(Err)
        });
        match accepted {
            Some(Ok(())) => Ok(self),
            Some(Err(status)) => {
                let stderr = runtime.path().join("daemon.stderr");
                let printed = fs::read_to_string(stderr).unwrap_or_default();
                Err(format!("the daemon exited with {status}: {printed:?}"))
            }
            None => Err(
                "the daemon ran over the test's time budget before its socket accepted".to_owned(),
            ),
        }
    }

    /// How the daemon exited, or `None` while it runs.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().unwrap()
    }

    /// Waits, as [`poll`] does, for the daemon to exit by itself, and gives its exit code.
    pub fn exit(mut self) -> Option<i32> {
        poll(|| self.exited()).and_then(|status| status.code())
    }

    pub fn drain(&mut self) -> io::Result<()> {
        let mut filler = vec![0; self.filler];
        self.stderr.read_exact(&mut filler)?;
        self.filler = 0;
        Ok(())
    }

    /// Waits, as [`poll`] does, for the daemon to exit by itself, and gives what it wrote to
    /// stderr and its exit code. A stderr that did not drain gives what the daemon wrote after
    /// what filled it. `None` when it runs over.
    pub fn finish(&mut self) -> Option<(Vec<u8>, Option<i32>)> {
        let status = poll(|| self.exited())?;
        self.drain().unwrap();
        let mut stderr = Vec::new();
        self.stderr.read_to_end(&mut stderr).unwrap();
        Some((stderr, status.code()))
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Where a daemon's stderr goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Output {
    /// Appended to its log in the runtime directory, as [`Daemon::spawn`] does.
    #[default]
    Logged,
    /// A pipe that the test fills before the daemon starts, as [`Daemon::spawn_stalled`] does.
    Stalled,
}

/// Whether a daemon may start threads and processes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Threads {
    #[default]
    Allowed,
    /// None, as [`no_threads`] gives.
    None,
}

/// What a block's lines ask of its next `lyncast daemon` line. That line takes it, so no later
/// daemon gets it.
#[derive(Debug, Default)]
pub struct NextDaemon {
    pub stderr: Output,
    pub threads: Threads,
}

impl NextDaemon {
    /// Starts `command`, a `lyncast daemon`, in the background, as [`Daemon::spawn`] does.
    pub fn spawn(self, mut command: Command, runtime: &Runtime) -> Daemon {
        self.limit(&mut command);
        match self.stderr {
            Output::Logged => Daemon::spawn(command, runtime),
            Output::Stalled => Daemon::spawn_stalled(command, runtime),
        }
    }

    /// Runs `command`, a `lyncast daemon`, until it exits, with [`Runtime::record`], and gives
    /// what it printed, or `None` when it runs over. A stalled stderr gives what the daemon wrote
    /// after what filled it, and its stdout goes to `daemon.stdout`. Otherwise the daemon runs as
    /// [`run_bounded`] runs a command.
    pub fn run(self, mut command: Command, runtime: &Runtime, out: &Path) -> Option<Ran> {
        if self.stderr == Output::Stalled {
            let mut daemon = self.spawn(command, runtime);
            return daemon
                .finish()
                .map(|(stderr, code)| (Vec::new(), stderr, code));
        }
        self.limit(&mut command);
        runtime.record(&mut command);
        run_bounded(command, out)
    }

    fn limit(&self, command: &mut Command) {
        if self.threads == Threads::None {
            no_threads(command);
        }
    }
}

/// Gives the process that `command` starts an `RLIMIT_NPROC` of 0, so it can start no thread and
/// no process, unless the kernel exempts its user, as it exempts root. The kernel checks the limit
/// at each thread or process that a process starts, and not at its exec. The process starts with
/// SIGALRM blocked too, as a launcher that takes its signals through a signalfd leaves it, and std
/// clears the signal mask before it runs the closure.
pub fn no_threads(command: &mut Command) -> &mut Command {
    // SAFETY: setrlimit, sigemptyset, sigaddset, and sigprocmask are async-signal-safe, and the
    // closure touches only its own locals.
    unsafe {
        command.pre_exec(|| {
            let none = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            let mut alarm: libc::sigset_t = mem::zeroed();
            libc::sigemptyset(&mut alarm);
            libc::sigaddset(&mut alarm, libc::SIGALRM);
            if libc::setrlimit(libc::RLIMIT_NPROC, &none) != 0
                || libc::sigprocmask(libc::SIG_BLOCK, &alarm, ptr::null_mut()) != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    }
}

/// Whether a process under [`no_threads`] can still start one. `timeout` starts its command in a
/// process of its own, and it fails at once when it cannot.
pub fn limit_exempt() -> bool {
    let mut timeout = Command::new("timeout");
    timeout
        .args(["10", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    no_threads(&mut timeout).status().unwrap().success()
}
