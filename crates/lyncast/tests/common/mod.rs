//! What the binary's end-to-end tests share: the fixture tree, the `$ ...` command lines of the
//! docs, bounded runs, and daemons in runtime directories of their own.

#![allow(dead_code)]

mod connections;
#[path = "../../../lyncast-core/tests/doc/mod.rs"]
pub mod doc;
mod pipe;

pub use pipe::fill;

use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::mem;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::ptr;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use connections::Connections;

/// The binary under test.
pub const LYNCAST: &str = env!("CARGO_BIN_EXE_lyncast");

/// Writes every `desktop <path>`, `executable <path>`, and `file <path>` block under `tree`, from
/// scratch, makes each `symlink <path> -> <target>` block a link to `target` as written, and makes
/// each `fifo <path>` block a FIFO. A `desktop` block may give `mode=<octal>` after its path.
pub fn materialize(blocks: &[doc::Block], tree: &Path) {
    match fs::remove_dir_all(tree) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{tree:?}: {error}"),
        _ => {}
    }
    fs::create_dir_all(tree).unwrap();
    let of = |kind: &'static str| blocks.iter().filter(move |block| block.kind == kind);
    for kind in ["desktop", "executable", "file"] {
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
        mkfifo(&path, 0o644);
    }
}

/// Makes a FIFO at `path` with `mode`.
pub fn mkfifo(path: &Path, mode: libc::mode_t) {
    let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: `c_path` is a NUL-terminated string that lives through the call.
    let made = unsafe { libc::mkfifo(c_path.as_ptr(), mode) };
    assert_eq!(made, 0, "mkfifo {path:?}: {}", io::Error::last_os_error());
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
    pub fn var(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(set, _)| set == name)
            .map(|(_, value)| value.as_os_str())
    }

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
        umask(&mut command);
        // env(1) unsets before it sets, so `env -u PATH PATH=` runs with an empty `PATH`.
        for name in &self.unset {
            command.env_remove(name);
        }
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
    }
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

/// A `$ [env -u NAME ...] [NAME=value ...] lyncast ...` line, or `None` for a line that runs
/// another program. A value may hold each `$NAME` in `paths` and `\x{HH}`, and `""` is an empty
/// argument.
pub fn command_line(command: &str, paths: &[(&str, &Path)]) -> Option<CommandLine> {
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
    let program = words.iter().position(|word| !word.contains('='))?;
    if words[program] != "lyncast" {
        return None;
    }
    let vars = words[..program]
        .iter()
        .map(|word| {
            let (name, value) = word.split_once('=').expect("a word with =");
            let value = OsString::from_vec(raw_bytes(&substitute(value, paths)));
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
    Some(CommandLine { unset, vars, args })
}

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

    fn log(&self, name: &str) -> fs::File {
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

/// One block's tree, runtime directory, and environment.
pub struct World {
    pub tree: PathBuf,
    pub runtime: Runtime,
    pub env: Vec<(String, String)>,
    /// Where each run's stdout and stderr go, beside the tree.
    pub out: PathBuf,
}

impl World {
    /// A fresh tree at `tree`, with a fresh runtime directory.
    pub fn new(fixtures: &[doc::Block], tree: PathBuf) -> World {
        World::within(fixtures, tree, Runtime::new(), &[])
    }

    /// A fresh tree at `tree` with `runtime`, and `more` variables beside the `env` block's.
    pub fn within(
        fixtures: &[doc::Block],
        tree: PathBuf,
        runtime: Runtime,
        more: &[(String, String)],
    ) -> World {
        materialize(fixtures, &tree);
        let mut env = environment(fixtures, &[("$TREE", &tree), ("$RUNTIME", runtime.path())]);
        env.extend_from_slice(more);
        let out = tree.with_extension("out");
        fs::create_dir_all(&out).unwrap();
        World {
            tree,
            runtime,
            env,
            out,
        }
    }

    /// A world on a fresh tree named `name` under `CARGO_TARGET_TMPDIR`, for a test that no
    /// block drives.
    pub fn scratch(fixtures: &[doc::Block], name: &str) -> World {
        World::new(fixtures, Path::new(env!("CARGO_TARGET_TMPDIR")).join(name))
    }

    /// Removes the tree and the output beside it once the run in them passes, so a run that
    /// fails keeps them to look at.
    pub fn passed(self) {
        fs::remove_dir_all(&self.out).unwrap();
        fs::remove_dir_all(&self.tree).unwrap();
    }

    /// `text` with `$TREE` and `$RUNTIME` replaced.
    pub fn substitute(&self, text: &str) -> String {
        substitute(
            text,
            &[("$TREE", &self.tree), ("$RUNTIME", self.runtime.path())],
        )
    }

    /// The `$ ...` line as a command in this world, and the line itself when it runs lyncast.
    /// Any other line runs through `/bin/sh`.
    pub fn command(&self, line: &str) -> (Command, Option<CommandLine>) {
        let paths = [
            ("$TREE", self.tree.as_path()),
            ("$RUNTIME", self.runtime.path()),
        ];
        let Some(lyncast) = command_line(line, &paths) else {
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(&line[2..])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TREE", &self.tree)
                .env("RUNTIME", self.runtime.path())
                .env("LYNCAST", LYNCAST)
                .current_dir(&self.tree);
            umask(&mut command);
            return (command, None);
        };
        let mut command = lyncast.command(&lyncast.args, &self.env);
        command.current_dir(&self.tree);
        (command, Some(lyncast))
    }

    /// The bytes the recorder file holds now. A recorder that is not a file holds none.
    pub fn recorded(&self) -> Vec<u8> {
        fs::read(self.runtime.runs()).unwrap_or_default()
    }
}

/// What a command printed, each line with its newline, and the lines it added to the recorder.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Printed {
    pub stdout: String,
    pub stderr: String,
    pub exit: i32,
    pub record: String,
}

impl Printed {
    /// The lines after a command in a session block.
    pub fn expected(lines: &[String]) -> Printed {
        let mut printed = Printed::default();
        for line in lines {
            if let Some(line) = line.strip_prefix("stderr: ") {
                printed.stderr += &format!("{line}\n");
            } else if let Some(code) = line.strip_prefix("exit: ") {
                printed.exit = code.parse().expect("exit: <n>");
            } else if let Some(argv) = line.strip_prefix("record: ") {
                printed.record += &format!("{argv}\n");
            } else {
                printed.stdout += &format!("{line}\n");
            }
        }
        printed
    }
}

fn hold_line(line: &str, newest: Option<&Daemon>, frozen: &mut Vec<Frozen>) -> Option<Ran> {
    let failed = |why: String| (Vec::new(), format!("{why}\n").into_bytes(), Some(1));
    let done = (Vec::new(), Vec::new(), Some(0));
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    Some(match words.as_slice() {
        ["freeze-thread", name] => match newest.map(|daemon| Frozen::new(daemon.id(), name)) {
            Some(Ok(held)) => {
                frozen.push(held);
                done
            }
            Some(Err(why)) => failed(why),
            None => failed("no daemon to freeze".to_owned()),
        },
        ["thaw-thread", after @ ..] => {
            let after = match after {
                [] => Duration::ZERO,
                [seconds] => seconds
                    .strip_suffix('s')
                    .and_then(|seconds| seconds.parse().ok())
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| panic!("not thaw-thread <n>s: {line}")),
                _ => panic!("not thaw-thread [<n>s]: {line}"),
            };
            match frozen.last() {
                Some(held) => {
                    held.thaw(after);
                    done
                }
                None => failed("no thread is frozen".to_owned()),
            }
        }
        _ => return None,
    })
}

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
    let mut failures = Vec::new();
    // Kept to the end of the block, which kills any still running.
    let mut daemons: Vec<Daemon> = Vec::new();
    let mut stall = false;
    let mut threadless = false;
    let mut frozen: Vec<Frozen> = Vec::new();
    let mut connections = Connections::default();
    let mut rest = block.lines.as_slice();
    while let [command, after @ ..] = rest {
        let number = block.line + 1 + block.lines.len() - rest.len();
        let printed = after
            .iter()
            .take_while(|line| !line.starts_with("$ "))
            .count();
        let newest = daemons.last().map(|daemon| daemon.id().to_string());
        let want: Vec<String> = after[..printed]
            .iter()
            .map(|line| match &newest {
                Some(pid) => world.substitute(line).replace("$DAEMON", pid),
                None => world.substitute(line),
            })
            .collect();
        rest = &after[printed..];
        let before = world.recorded().len();
        let ran = match command.as_str() {
            "$ stall-stderr" => {
                stall = true;
                Some((Vec::new(), Vec::new(), Some(0)))
            }
            "$ no-threads" | "$ deny-threads" if limit_exempt() => {
                // libtest hides what eprintln! prints in a test that passes, and the skip has to
                // show.
                let _ = writeln!(
                    io::stderr(),
                    "line {number}: the rest of the block skipped: RLIMIT_NPROC does not limit this user"
                );
                break;
            }
            "$ no-threads" => {
                threadless = true;
                Some((Vec::new(), Vec::new(), Some(0)))
            }
            "$ drain-stderr" => Some(match daemons.last_mut().map(Daemon::drain) {
                Some(Ok(())) => (Vec::new(), Vec::new(), Some(0)),
                Some(Err(error)) => (Vec::new(), format!("{error}\n").into_bytes(), Some(1)),
                None => (Vec::new(), b"no daemon to drain\n".to_vec(), Some(1)),
            }),
            "$ wait-exit" => {
                let daemon = daemons.last_mut().expect("wait-exit follows a daemon");
                daemon
                    .finish()
                    .map(|(stderr, code)| (Vec::new(), stderr, code))
            }
            _ => match hold_line(command, daemons.last(), &mut frozen)
                .or_else(|| connections.line(command, daemons.last(), &world.runtime))
                .or_else(|| verbs(command, daemons.last().map(Daemon::id)))
            {
                Some(ran) => Some(ran),
                None => {
                    let (mut run, lyncast) = world.command(command);
                    if lyncast.is_none()
                        && let Some(newest) = daemons.last()
                    {
                        run.env("DAEMON", newest.id().to_string());
                    }
                    let daemon = lyncast
                        .as_ref()
                        .filter(|line| line.args.first().is_some_and(|arg| arg == "daemon"));
                    if daemon.is_some() && threadless {
                        threadless = false;
                        no_threads(&mut run);
                    }
                    if let Some(line) = daemon
                        && want.is_empty()
                    {
                        let socket = line.var("XDG_RUNTIME_DIR").map_or_else(
                            || world.runtime.socket(),
                            |dir| Path::new(dir).join("lyncast.sock"),
                        );
                        let started = if stall {
                            Daemon::spawn_stalled(run, &world.runtime)
                        } else {
                            Daemon::spawn(run, &world.runtime)
                        };
                        stall = false;
                        match started.serving(&world.runtime, &socket) {
                            Ok(daemon) => daemons.push(daemon),
                            Err(error) => {
                                failures.push(format!("line {number}: {command}\n  {error}"));
                                break;
                            }
                        }
                        continue;
                    }
                    if daemon.is_some() && stall {
                        stall = false;
                        Daemon::spawn_stalled(run, &world.runtime)
                            .finish()
                            .map(|(stderr, code)| (Vec::new(), stderr, code))
                    } else {
                        if daemon.is_some() {
                            world.runtime.record(&mut run);
                        }
                        run_bounded(run, &world.out)
                    }
                }
            },
        };
        let Some((stdout, stderr, code)) = ran else {
            failures.push(format!(
                "line {number}: {command}\n  ran over the test's time budget"
            ));
            break;
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let got = Printed {
            stdout: text(&stdout),
            stderr: text(&stderr),
            exit: code.unwrap_or(-1),
            record: text(world.recorded().get(before..).unwrap_or_default()),
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
