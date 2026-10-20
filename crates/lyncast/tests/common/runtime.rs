//! Runtime directories, each a test's own `XDG_RUNTIME_DIR`, and the Tier 2 compositor that
//! `scripts/test-env.sh` starts in one.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

/// A fresh directory under `/tmp` for one test's `XDG_RUNTIME_DIR`, removed when dropped. A
/// socket path holds at most 107 bytes, which a path under `CARGO_TARGET_TMPDIR` can pass.
pub struct Runtime {
    path: PathBuf,
    compositor: bool,
}

const TEST_ENV: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/test-env.sh");

impl Runtime {
    pub fn new() -> Runtime {
        let mut template = b"/tmp/lyncast-test.XXXXXX\0".to_vec();
        // SAFETY: `template` is NUL-terminated, and mkdtemp rewrites its X's in place.
        let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
        assert!(!made.is_null(), "mkdtemp: {}", io::Error::last_os_error());
        template.pop();
        Runtime {
            path: PathBuf::from(OsString::from_vec(template)),
            compositor: false,
        }
    }

    /// The `XDG_RUNTIME_DIR` of a compositor that `scripts/test-env.sh start` starts, and the
    /// `WAYLAND_DISPLAY` that reaches it. Dropping the runtime stops the compositor.
    pub fn compositor() -> (Runtime, String) {
        let started = Command::new(TEST_ENV)
            .arg("start")
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .output()
            .expect("test-env.sh runs");
        assert!(
            started.status.success(),
            "test-env.sh start: {}",
            started.status
        );
        let printed = String::from_utf8(started.stdout).expect("test-env.sh prints UTF-8");
        let export = |name: &str| {
            printed
                .lines()
                .find_map(|line| line.strip_prefix(&format!("export {name}=")))
                .unwrap_or_else(|| panic!("test-env.sh exports no {name}: {printed:?}"))
                .to_owned()
        };
        let runtime = Runtime {
            path: PathBuf::from(export("XDG_RUNTIME_DIR")),
            compositor: true,
        };
        (runtime, export("WAYLAND_DISPLAY"))
    }

    /// Stops the compositor and removes the directory, as `scripts/test-env.sh stop` does.
    pub fn stop_compositor(&self) -> io::Result<ExitStatus> {
        Command::new(TEST_ENV)
            .arg("stop")
            .arg(&self.path)
            .stdin(Stdio::null())
            .status()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn socket(&self) -> PathBuf {
        self.path.join("lyncast.sock")
    }

    /// The file that each test daemon records to.
    pub fn runs(&self) -> PathBuf {
        self.path.join("runs")
    }

    pub(super) fn log(&self, name: &str) -> fs::File {
        fs::File::options()
            .create(true)
            .append(true)
            .open(self.path().join(name))
            .unwrap()
    }

    /// Adds `--record` and [`Runtime::runs`], so no test daemon starts a real program.
    pub fn record(&self, daemon: &mut Command) {
        if !daemon.get_args().any(|arg| arg == "--record") {
            daemon.arg("--record").arg(self.runs());
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if self.compositor {
            let _ = self.stop_compositor();
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}
