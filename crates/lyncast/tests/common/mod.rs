//! What the binary's end-to-end tests share: the fixture tree, the `$ ...` command lines of the
//! docs, bounded runs, daemons in runtime directories of their own, and the runner of `session`
//! blocks.

#![allow(dead_code)]

#[path = "../../../lyncast-core/tests/doc/mod.rs"]
pub mod doc;

use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
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
    /// The value this line gives `name`, if it sets one.
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
        // env(1) unsets before it sets, so `env -u PATH PATH=` runs with an empty `PATH`.
        for name in &self.unset {
            command.env_remove(name);
        }
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
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

    /// The file that each test daemon records to.
    pub fn runs(&self) -> PathBuf {
        self.0.join("runs")
    }

    /// `name` in the directory, opened to append, as a daemon's stdout or stderr.
    fn log(&self, name: &str) -> fs::File {
        fs::File::options()
            .create(true)
            .append(true)
            .open(self.path().join(name))
            .unwrap()
    }

    /// Adds `--record` and [`Runtime::runs`] to a `lyncast daemon` command that names no
    /// `--record` of its own, so no test daemon starts a real program, even one that a test
    /// expects to fail.
    pub fn record(&self, daemon: &mut Command) {
        if !daemon.get_args().any(|arg| arg == "--record") {
            daemon.arg("--record").arg(self.runs());
        }
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
pub struct Daemon {
    child: Child,
    /// What the daemon writes to stderr: `daemon.stderr` from where this daemon started writing,
    /// still open once the runtime directory has gone, or the pipe of a stderr that does not
    /// drain.
    stderr: fs::File,
    /// The bytes that fill a stderr that does not drain, ahead of what the daemon writes.
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

    /// [`Daemon::spawn`] with a stderr that does not drain: a pipe that is full before the daemon
    /// starts, so each write the daemon makes to stderr waits until [`Daemon::drain`].
    pub fn spawn_stalled(command: Command, runtime: &Runtime) -> Daemon {
        let (reader, mut writer) = io::pipe().unwrap();
        let filler = fill(&mut writer);
        let reader = fs::File::from(OwnedFd::from(reader));
        Daemon::spawn_with(command, runtime, Stdio::from(writer), reader, filler)
    }

    /// Starts `command` with `stderr`. `command` goes once the daemon runs, and with it the test's
    /// end of a stderr pipe, so the pipe ends when the daemon exits.
    fn spawn_with(
        mut command: Command,
        runtime: &Runtime,
        stderr: Stdio,
        from_here: fs::File,
        filler: usize,
    ) -> Daemon {
        runtime.record(&mut command);
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

    /// [`Daemon::start`] for a daemon whose socket is `socket`, because it has an
    /// `XDG_RUNTIME_DIR` of its own. Its stdout and stderr still go to `runtime`.
    pub fn start_on(command: Command, runtime: &Runtime, socket: &Path) -> Result<Daemon, String> {
        Daemon::spawn(command, runtime).serving(runtime, socket)
    }

    /// Waits, as [`poll`] does, until the daemon's socket, `socket`, accepts. Fails when the
    /// daemon exits first, or when the wait runs out.
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

    /// Reads what fills a stderr that does not drain, so the daemon's writes to it go through.
    pub fn drain(&mut self) -> io::Result<()> {
        let mut filler = vec![0; self.filler];
        self.stderr.read_exact(&mut filler)?;
        self.filler = 0;
        Ok(())
    }

    /// Waits, as [`poll`] does, for the daemon to exit by itself, and gives what it wrote to
    /// stderr and its exit code. `None` when it runs over.
    pub fn finish(&mut self) -> Option<(Vec<u8>, Option<i32>)> {
        let status = poll(|| self.exited())?;
        let mut stderr = Vec::new();
        self.stderr.read_to_end(&mut stderr).unwrap();
        Some((stderr, status.code()))
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

/// Fills `pipe` until it takes no more, and gives how many bytes that took. The pipe's end still
/// blocks afterwards, for the daemon that inherits it.
fn fill(pipe: &mut io::PipeWriter) -> usize {
    let fd = pipe.as_raw_fd();
    // SAFETY: fcntl on a descriptor the pipe owns, which lives through each call.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut filled = 0;
    // Big writes fill the pipe's pages, and single bytes the room that a big write leaves.
    for chunk in [vec![b'x'; 4096], vec![b'x']] {
        loop {
            match pipe.write(&chunk) {
                Ok(written) => filled += written,
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => panic!("filling a pipe: {error}"),
            }
        }
    }
    // SAFETY: as above.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) }, 0);
    filled
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    /// A fresh tree at `tree`.
    pub fn new(fixtures: &[doc::Block], tree: PathBuf) -> World {
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
                .current_dir(&self.tree);
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

/// Runs one session block and gives what failed.
pub fn session(world: &World, block: &doc::Block) -> Vec<String> {
    let mut failures = Vec::new();
    // Kept to the end of the block, which kills any still running.
    let mut daemons: Vec<Daemon> = Vec::new();
    // The next daemon gets a stderr that does not drain.
    let mut stall = false;
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
        let before = world.recorded().len();
        let ran = match command.as_str() {
            "$ stall-stderr" => {
                stall = true;
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
            _ => {
                let (mut run, lyncast) = world.command(command);
                if lyncast.is_none()
                    && let Some(newest) = daemons.last()
                {
                    run.env("DAEMON", newest.id().to_string());
                }
                let daemon = lyncast
                    .as_ref()
                    .filter(|line| line.args.first().is_some_and(|arg| arg == "daemon"));
                if let Some(line) = daemon
                    && want.is_empty()
                {
                    let socket = line.var("XDG_RUNTIME_DIR").map_or_else(
                        || world.runtime.socket(),
                        |dir| Path::new(dir).join("lyncast.sock"),
                    );
                    // Daemon::spawn adds --record itself.
                    let started = if stall {
                        Daemon::spawn_stalled(run, &world.runtime)
                    } else {
                        Daemon::spawn(run, &world.runtime)
                    };
                    stall = false;
                    match started.serving(&world.runtime, &socket) {
                        Ok(daemon) => daemons.push(daemon),
                        Err(error) => {
                            failures.push(format!("line {}: {command}\n  {error}", block.line));
                            break;
                        }
                    }
                    continue;
                }
                if daemon.is_some() {
                    world.runtime.record(&mut run);
                }
                run_bounded(run, &world.out)
            }
        };
        let Some((stdout, stderr, code)) = ran else {
            failures.push(format!(
                "line {}: {command}\n  ran over the test's time budget",
                block.line
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
                "line {}: {command}\n  want {want:?}\n  got  {got:?}",
                block.line
            ));
        }
    }
    failures
}
