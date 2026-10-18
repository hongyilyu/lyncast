//! What the binary's end-to-end tests share. This file holds bounded runs and plain `lyncast`
//! commands. [`tree`] writes a doc's fixture tree and reads its env block, [`world`] gives one
//! block its tree, runtime directory, and environment and turns its `$ ...` lines into commands,
//! and [`runtime`] makes the runtime directories, with a Tier 2 compositor in one when a test
//! asks. [`daemon`] starts daemons in the background and says how the next one starts,
//! [`frozen`] holds one of a daemon's threads still, and [`session`] runs a block's lines and
//! compares what each printed. [`clipboard`] holds the clipboard verbs, which run the test binary
//! again as a helper.

#![allow(dead_code)]

pub mod clipboard;
mod connections;
mod daemon;
#[path = "../../../lyncast-core/tests/doc/mod.rs"]
pub mod doc;
mod frozen;
mod pipe;
mod runtime;
mod session;
mod tree;
mod world;

// Each test binary uses some of these, as it does of the items they name.
#[allow(unused_imports)]
pub use self::{
    daemon::{Daemon, NextDaemon, Output, Threads, limit_exempt, no_threads},
    frozen::{Frozen, named_thread},
    pipe::fill,
    runtime::Runtime,
    session::{Printed, Verbs, session, session_later, session_with},
    tree::{environment, materialize, mkfifo, raw_bytes, substitute},
    world::{CommandLine, World, command_line},
};

use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The binary under test.
pub const LYNCAST: &str = env!("CARGO_BIN_EXE_lyncast");

/// How long a test waits for something before it fails. A hang fails at any limit, and 60 s
/// leaves room for the 64 MiB session in daemon.md, whose save takes about 2.3 s in a debug
/// build on an idle box and ran over 10 s at a load average of 30 on 32 cores.
pub const BUDGET: Duration = Duration::from_secs(60);

/// Calls `ready` every millisecond until it gives a value, for up to [`BUDGET`], so a test that
/// waits for something that never comes fails and does not hang.
pub fn poll<T>(mut ready: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + BUDGET;
    loop {
        if let Some(value) = ready() {
            return Some(value);
        }
        if Instant::now() > deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// What a finished run printed and its exit code.
pub type Ran = (Vec<u8>, Vec<u8>, Option<i32>);

/// A run that printed nothing and exited 0.
pub const OK: Ran = (Vec::new(), Vec::new(), Some(0));

/// A run that printed `why` on stderr and exited 1.
pub fn failed(why: impl fmt::Display) -> Ran {
    (Vec::new(), format!("{why}\n").into_bytes(), Some(1))
}

/// Runs `command` until it exits, as [`poll`] waits, and kills it if it runs over. Its stdout
/// and stderr go to files under `dir`, which never block it. `None` when it ran over.
pub fn run_bounded(mut command: Command, dir: &Path) -> Option<Ran> {
    let [stdout, stderr] = ["stdout", "stderr"].map(|name| dir.join(name));
    command
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = command.spawn().expect("lyncast runs");
    let Some(status) = poll(|| child.try_wait().unwrap()) else {
        child.kill().unwrap();
        child.wait().unwrap();
        return None;
    };
    Some((
        fs::read(stdout).unwrap(),
        fs::read(stderr).unwrap(),
        status.code(),
    ))
}

fn umask(command: &mut Command) {
    // SAFETY: umask is async-signal-safe, and the closure touches nothing else.
    unsafe {
        command.pre_exec(|| {
            libc::umask(0o022);
            Ok(())
        });
    }
}

/// `lyncast <args>` with an empty environment but `XDG_RUNTIME_DIR`, and no stdin.
pub fn lyncast<S: AsRef<OsStr>>(runtime: &Runtime, args: impl IntoIterator<Item = S>) -> Command {
    let mut command = Command::new(LYNCAST);
    command
        .args(args)
        .env_clear()
        .env("XDG_RUNTIME_DIR", runtime.path())
        .stdin(Stdio::null());
    umask(&mut command);
    command
}

/// Runs [`lyncast`] to its end, as [`run_bounded`] does, and gives what it printed as text.
pub fn ask(runtime: &Runtime, args: &[&str]) -> (String, String, Option<i32>) {
    let out = runtime.path().join("client");
    fs::create_dir_all(&out).unwrap();
    let (stdout, stderr, code) = run_bounded(lyncast(runtime, args), &out).expect("lyncast ends");
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
        code,
    )
}
