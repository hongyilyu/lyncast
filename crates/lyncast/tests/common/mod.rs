//! What the binary's end-to-end tests share: the fixture tree, the `$ ...` command lines of the
//! docs, bounded runs, and daemons in runtime directories of their own.

#![allow(dead_code)]

#[path = "../../../lyncast-core/tests/doc/mod.rs"]
pub mod doc;

use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::{self, ErrorKind};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// The binary under test.
pub const LYNCAST: &str = env!("CARGO_BIN_EXE_lyncast");

/// Writes every `desktop <path>` and `executable <path>` block under `tree`, from scratch, makes
/// each `symlink <path> -> <target>` block a link to `target` as written, and makes each
/// `fifo <path>` block a FIFO. A `desktop` block may give `mode=<octal>` after its path.
pub fn materialize(blocks: &[doc::Block], tree: &Path) {
    match fs::remove_dir_all(tree) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{tree:?}: {error}"),
        _ => {}
    }
    fs::create_dir_all(tree).unwrap();
    let of = |kind: &'static str| blocks.iter().filter(move |block| block.kind == kind);
    for kind in ["desktop", "executable"] {
        for block in of(kind) {
            let (relative, mode) = match block.args.as_slice() {
                [relative] => (relative, (kind == "executable").then_some(0o755)),
                [relative, mode] if kind == "desktop" => {
                    let mode = mode
                        .strip_prefix("mode=")
                        .map(|m| u32::from_str_radix(m, 8));
                    let Some(Ok(mode)) = mode else {
                        panic!("line {}: not mode=<octal>", block.line);
                    };
                    (relative, Some(mode))
                }
                _ => panic!("line {}: a {kind} block names one path", block.line),
            };
            let path = tree.join(relative);
            fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
            fs::write(&path, raw_bytes(&format!("{}\n", block.lines.join("\n")))).unwrap();
            if let Some(mode) = mode {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            }
        }
    }
    for block in of("symlink") {
        let [relative, arrow, target] = block.args.as_slice() else {
            panic!(
                "line {}: a symlink block reads <path> -> <target>",
                block.line
            );
        };
        assert!(
            arrow == "->" && block.lines.is_empty(),
            "line {}: a symlink block reads <path> -> <target> and holds no lines",
            block.line
        );
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
        symlink(target, &path).unwrap();
    }
    for block in of("fifo") {
        let [relative] = block.args.as_slice() else {
            panic!("line {}: a fifo block names one path", block.line);
        };
        assert!(
            block.lines.is_empty(),
            "line {}: a fifo block holds no lines",
            block.line
        );
        let path = tree.join(relative);
        fs::create_dir_all(path.parent().expect("a path under the tree")).unwrap();
        let c_path = CString::new(path.into_os_string().into_vec()).unwrap();
        // SAFETY: `c_path` is a NUL-terminated string that lives through the call.
        assert_eq!(
            unsafe { libc::mkfifo(c_path.as_ptr(), 0o644) },
            0,
            "{c_path:?}"
        );
    }
}

/// The text's bytes, with each exact `\x{HH}` written as the byte `HH`, so a desktop file can
/// hold a byte that is not UTF-8.
pub fn raw_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\x{") {
        out.extend_from_slice(&rest.as_bytes()[..at]);
        let after = &rest[at + 3..];
        match after.get(..3).and_then(|hex| {
            let digits = hex.strip_suffix('}')?;
            digits
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
                .then_some(())?;
            u8::from_str_radix(digits, 16).ok()
        }) {
            Some(byte) => {
                out.push(byte);
                rest = &after[3..];
            }
            None => {
                out.extend_from_slice(b"\\x{");
                rest = after;
            }
        }
    }
    out.extend_from_slice(rest.as_bytes());
    out
}

/// The one `env` block's `NAME=value` lines, with each `$NAME` in `paths` replaced.
pub fn environment(blocks: &[doc::Block], paths: &[(&str, &Path)]) -> Vec<(String, String)> {
    let envs: Vec<&doc::Block> = blocks.iter().filter(|block| block.kind == "env").collect();
    let [block] = envs.as_slice() else {
        panic!("the doc needs one env block, not {}", envs.len());
    };
    block
        .lines
        .iter()
        .map(|line| {
            let (name, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("not NAME=value: {line:?}"));
            (name.to_owned(), substitute(value, paths))
        })
        .collect()
}

/// `text` with each `$NAME` in `paths` replaced by its path.
pub fn substitute(text: &str, paths: &[(&str, &Path)]) -> String {
    paths.iter().fold(text.to_owned(), |text, (name, path)| {
        text.replace(name, path.to_str().expect("a UTF-8 test path"))
    })
}

/// One `$ ...` command line of a doc.
pub struct CommandLine {
    /// The `env -u NAME` variables, removed from the env block.
    pub unset: Vec<String>,
    /// The `NAME=value` words, which override the env block after the removals.
    pub vars: Vec<(String, OsString)>,
    pub args: Vec<String>,
}

impl CommandLine {
    /// `lyncast` with `args` and only `env`'s variables, as this line changes them.
    pub fn command<S: AsRef<OsStr>>(
        &self,
        args: impl IntoIterator<Item = S>,
        env: &[(String, String)],
    ) -> Command {
        let mut command = Command::new(LYNCAST);
        command
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null());
        // env(1) unsets before it sets, so `env -u PATH PATH=` runs with an empty `PATH`.
        for name in &self.unset {
            command.env_remove(name);
        }
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
    }
}

/// A `$ [env -u NAME ...] [NAME=value ...] lyncast ...` line. A value may hold `$TREE` and
/// `\x{HH}`, and `""` is an empty argument.
pub fn command_line(command: &str, tree: &Path) -> CommandLine {
    let mut words: &[&str] = &command
        .strip_prefix("$ ")
        .unwrap_or_else(|| panic!("a command line starts with `$ `: {command:?}"))
        .split(' ')
        .collect::<Vec<_>>();
    let mut unset = Vec::new();
    if let ["env", rest @ ..] = words {
        words = rest;
        while let ["-u", name, rest @ ..] = words {
            unset.push((*name).to_owned());
            words = rest;
        }
    }
    let program = words
        .iter()
        .position(|&word| word == "lyncast")
        .unwrap_or_else(|| panic!("a command line runs lyncast: {command:?}"));
    let vars = words[..program]
        .iter()
        .map(|word| {
            let (name, value) = word
                .split_once('=')
                .unwrap_or_else(|| panic!("not NAME=value before lyncast: {command:?}"));
            let value = OsString::from_vec(raw_bytes(&substitute(value, &[("$TREE", tree)])));
            (name.to_owned(), value)
        })
        .collect();
    let args = words[program + 1..]
        .iter()
        .map(|word| {
            let arg = doc::unquote(word);
            assert!(
                !arg.contains('"'),
                "a quoted argument with a space: {command:?}"
            );
            arg.to_owned()
        })
        .collect();
    CommandLine { unset, vars, args }
}

/// Calls `ready` every millisecond until it gives a value, for up to 10 s, so a test that waits
/// for something that never comes fails and does not hang.
pub fn poll<T>(mut ready: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(10);
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

/// A fresh directory under `/tmp` for one test's `XDG_RUNTIME_DIR`, removed when dropped. A
/// socket path holds at most 107 bytes, which a path under `CARGO_TARGET_TMPDIR` can pass.
pub struct Runtime(PathBuf);

impl Runtime {
    pub fn new() -> Runtime {
        let mut template = b"/tmp/lyncast-test.XXXXXX\0".to_vec();
        // SAFETY: `template` is NUL-terminated, and mkdtemp rewrites its X's in place.
        let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
        assert!(!made.is_null(), "mkdtemp: {}", io::Error::last_os_error());
        template.pop();
        Runtime(PathBuf::from(OsString::from_vec(template)))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn socket(&self) -> PathBuf {
        self.0.join("lyncast.sock")
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

/// A daemon that a test started in the background. Dropping it kills it, so a test that fails
/// leaves no daemon running.
pub struct Daemon(Child);

impl Daemon {
    /// Starts `command`, a `lyncast daemon`, in the background, with its stdout and stderr
    /// appended to `daemon.stdout` and `daemon.stderr` in `runtime`.
    pub fn spawn(mut command: Command, runtime: &Runtime) -> Daemon {
        let log = |name: &str| {
            fs::File::options()
                .create(true)
                .append(true)
                .open(runtime.path().join(name))
                .unwrap()
        };
        command
            .stdin(Stdio::null())
            .stdout(log("daemon.stdout"))
            .stderr(log("daemon.stderr"));
        Daemon(command.spawn().expect("lyncast runs"))
    }

    /// Spawns `command` and waits, as [`poll`] does, until the socket accepts. Fails when the
    /// daemon exits first, or when the wait runs out.
    pub fn start(command: Command, runtime: &Runtime) -> Result<Daemon, String> {
        let mut daemon = Daemon::spawn(command, runtime);
        let accepted = poll(|| {
            if UnixStream::connect(runtime.socket()).is_ok() {
                return Some(Ok(()));
            }
            daemon.exited().map(Err)
        });
        match accepted {
            Some(Ok(())) => Ok(daemon),
            Some(Err(status)) => {
                let stderr = runtime.path().join("daemon.stderr");
                let printed = fs::read_to_string(stderr).unwrap_or_default();
                Err(format!("the daemon exited with {status}: {printed:?}"))
            }
            None => Err("the daemon ran over 10 s before its socket accepted".to_owned()),
        }
    }

    /// How the daemon exited, or `None` while it runs.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.0.try_wait().unwrap()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
