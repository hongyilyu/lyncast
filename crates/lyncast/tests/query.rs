//! `lyncast query` end to end. The test writes the `desktop`, `executable`, `symlink`, and `fifo`
//! blocks of docs/design/ranking.md under a fresh directory, runs the real binary with only the
//! `env` block's variables, and compares stdout byte for byte with each `cli` block.

#[path = "../../lyncast-core/tests/doc/mod.rs"]
mod doc;

use std::collections::BTreeSet;
use std::ffi::{CString, OsString};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Writes every `desktop <path>` and `executable <path>` block under `tree`, from scratch, makes
/// each `symlink <path> -> <target>` block a link to `target` as written, and makes each
/// `fifo <path>` block a FIFO. A `desktop` block may give `mode=<octal>` after its path.
fn materialize(doc: &str, tree: &Path) {
    match fs::remove_dir_all(tree) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{tree:?}: {error}"),
        _ => {}
    }
    for kind in ["desktop", "executable"] {
        for block in doc::blocks(doc, kind) {
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
    for block in doc::blocks(doc, "symlink") {
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
    for block in doc::blocks(doc, "fifo") {
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
fn raw_bytes(text: &str) -> Vec<u8> {
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

/// The `env` block's `NAME=value` lines, with `$TREE` replaced.
fn environment(doc: &str, tree: &Path) -> Vec<(String, String)> {
    let blocks = doc::blocks(doc, "env");
    let [block] = blocks.as_slice() else {
        panic!("ranking.md needs one env block, not {}", blocks.len());
    };
    let tree = tree.to_str().expect("a UTF-8 target dir");
    block
        .lines
        .iter()
        .map(|line| {
            let (name, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("not NAME=value: {line:?}"));
            (name.to_owned(), value.replace("$TREE", tree))
        })
        .collect()
}

/// One `cli` command line.
struct CommandLine {
    /// The `env -u NAME` variables, removed from the env block.
    unset: Vec<String>,
    /// The `NAME=value` words, which override the env block after the removals.
    vars: Vec<(String, OsString)>,
    args: Vec<String>,
}

/// A `$ [env -u NAME ...] [NAME=value ...] lyncast ...` line. A value may hold `$TREE` and
/// `\x{HH}`, and `""` is an empty argument.
fn command_line(command: &str, tree: &Path) -> CommandLine {
    let mut words: &[&str] = &command
        .strip_prefix("$ ")
        .unwrap_or_else(|| panic!("a cli block starts with `$ `: {command:?}"))
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
        .unwrap_or_else(|| panic!("a cli block runs lyncast: {command:?}"));
    let tree = tree.to_str().expect("a UTF-8 target dir");
    let vars = words[..program]
        .iter()
        .map(|word| {
            let (name, value) = word
                .split_once('=')
                .unwrap_or_else(|| panic!("not NAME=value before lyncast: {command:?}"));
            let value = OsString::from_vec(raw_bytes(&value.replace("$TREE", tree)));
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

/// The `desktop`, `executable`, `symlink`, and `fifo` blocks are the files that the "Each file
/// proves one rule" table lists, so a lost block fails, even an `executable` whose loss only lets
/// its entry hide for a second reason.
#[test]
fn every_tree_file_is_in_the_table() {
    let doc = doc::ranking_doc();
    let table: BTreeSet<String> = doc
        .lines()
        .skip_while(|line| *line != "| File | Proves |")
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cell = line
                .trim_start_matches('|')
                .split('|')
                .next()
                .unwrap_or_default();
            cell.trim().trim_matches('`').to_owned()
        })
        .collect();
    let files: BTreeSet<String> = ["desktop", "executable", "symlink", "fifo"]
        .into_iter()
        .flat_map(|kind| doc::blocks(&doc, kind))
        .map(|block| block.args[0].clone())
        .collect();
    assert_eq!(files, table);
}

/// Each `executable` block is a program that some `desktop` block's `Exec` or `TryExec` names.
#[test]
fn every_executable_is_a_program_some_entry_names() {
    let doc = doc::ranking_doc();
    let named: BTreeSet<String> = doc::blocks(&doc, "desktop")
        .iter()
        .flat_map(|block| &block.lines)
        .filter_map(|line| match line.strip_prefix("Exec=") {
            Some(exec) => exec.split(' ').next(),
            None => line.strip_prefix("TryExec="),
        })
        .map(str::to_owned)
        .collect();
    for block in doc::blocks(&doc, "executable") {
        let program = Path::new(&block.args[0])
            .file_name()
            .and_then(|name| name.to_str());
        assert!(
            program.is_some_and(|program| named.contains(program)),
            "line {}: no desktop block runs {}",
            block.line,
            block.args[0]
        );
    }
}

#[test]
fn query_prints_the_doc_rows() {
    let doc = doc::ranking_doc();
    let tree = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ranking-tree");
    materialize(&doc, &tree);
    // The Notes fixture's `\x{E9}` is the byte 0xE9, so the file it proves its row with is not
    // UTF-8.
    let notes = fs::read(tree.join("home/applications/notes.desktop")).unwrap();
    assert!(
        notes.windows(4).any(|bytes| bytes == b"caf\xE9") && str::from_utf8(&notes).is_err(),
        "home/applications/notes.desktop holds no raw 0xE9"
    );
    // A cli variable's `\x{FF}` is the byte 0xFF, so these blocks prove their rules with a value
    // that is not UTF-8.
    for command in [
        "$ XDG_DATA_DIRS=$TREE/nowhere\\x{FF}:$TREE/usr lyncast query firef --json",
        "$ PATH=$TREE/nowhere\\x{FF}:$TREE/bin lyncast query notes --json",
    ] {
        let line = command_line(command, &tree);
        assert!(
            line.vars[0]
                .1
                .as_bytes()
                .windows(8)
                .any(|bytes| bytes == b"nowhere\xFF"),
            "{command} holds no raw 0xFF"
        );
    }
    let env = environment(&doc, &tree);
    let blocks = doc::blocks(&doc, "cli");
    for command in [
        "$ lyncast query firef --json",
        "$ lyncast query fi --json",
        "$ LC_ALL=C lyncast query firefox --json",
        "$ LC_ALL=C LC_MESSAGES=de_DE.UTF-8 lyncast query firefox --json",
        "$ LC_MESSAGES=de_DE.UTF-8 LANG=C lyncast query firefox --json",
        "$ LANGUAGE=fr:de LC_ALL=C lyncast query firefox --json",
        "$ LANGUAGE=fr lyncast query firefox --json",
        "$ lyncast query \"\" --json",
        "$ lyncast query notes --json",
        "$ lyncast query term --json",
        "$ lyncast query zzz --json",
        "$ XDG_DATA_HOME=home lyncast query notes --json",
        "$ XDG_DATA_DIRS=$TREE/nowhere\\x{FF}:$TREE/usr lyncast query firef --json",
        "$ PATH=$TREE/nowhere\\x{FF}:$TREE/bin lyncast query notes --json",
        "$ env -u PATH lyncast query pathless --json",
        "$ PATH= lyncast query pathless --json",
        "$ env -u PATH PATH= lyncast query pathless --json",
        "$ lyncast query workdir --json",
        "$ PATH= lyncast query workdir --json",
        "$ lyncast query masked --json",
        "$ lyncast query symlinked --json",
        "$ lyncast query check --json",
        "$ lyncast query -5+3 --json",
        "$ lyncast query -h --json",
        "$ lyncast query --json -- --json",
        "$ lyncast query --json -- --",
    ] {
        assert!(
            blocks.iter().any(|block| block.lines[0] == command),
            "ranking.md has no cli block for {command}"
        );
    }
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ranking-out");
    fs::create_dir_all(&out_dir).unwrap();
    let mut failures = Vec::new();
    for block in &blocks {
        let (command, rows) = block
            .lines
            .split_first()
            .expect("a cli block has a command");
        let line = command_line(command, &tree);
        let mut run = Command::new(env!("CARGO_BIN_EXE_lyncast"));
        run.args(line.args)
            .env_clear()
            .envs(env.iter().map(|(name, value)| (name, value)))
            .current_dir(&tree);
        // env(1) unsets before it sets, so `env -u PATH PATH=` runs with an empty `PATH`.
        for name in line.unset {
            run.env_remove(name);
        }
        run.envs(line.vars);
        let Some((stdout, stderr, code)) = run_bounded(run, &out_dir) else {
            failures.push(format!("line {}: {command}\n  ran over 10 s", block.line));
            break;
        };
        let want: String = rows.iter().map(|row| format!("{row}\n")).collect();
        let got = (
            String::from_utf8_lossy(&stdout).into_owned(),
            String::from_utf8_lossy(&stderr).into_owned(),
            code,
        );
        if got != (want.clone(), String::new(), Some(0)) {
            failures.push(format!(
                "line {}: {command}\n  want {want:?}\n  got  {got:?}",
                block.line
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cli blocks fail:\n{}",
        failures.len(),
        blocks.len(),
        failures.join("\n")
    );
}

/// Runs `command` for at most 10 s, so a scan that blocks, as on a FIFO, fails its block and does
/// not hang the test. Its stdout and stderr go to files under `dir`, which never block it. `None`
/// when it ran over.
fn run_bounded(mut command: Command, dir: &Path) -> Option<(Vec<u8>, Vec<u8>, Option<i32>)> {
    let [stdout, stderr] = ["stdout", "stderr"].map(|name| dir.join(name));
    command
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = command.spawn().expect("lyncast runs");
    let deadline = Instant::now() + Duration::from_secs(10);
    let code = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status.code();
        }
        if Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            return None;
        }
        thread::sleep(Duration::from_millis(1));
    };
    Some((fs::read(stdout).unwrap(), fs::read(stderr).unwrap(), code))
}

/// A write that fails exits 1, even when the error message cannot be written either, and a usage
/// error keeps clap's 2.
#[test]
fn a_failed_write_exits_1() {
    let full = || {
        fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("/dev/full opens")
    };
    let run = |args: &[&str], stdout_full: bool, stderr_full: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lyncast"));
        command.args(args).env_clear().stdin(Stdio::null());
        command.stdout(if stdout_full {
            Stdio::from(full())
        } else {
            Stdio::null()
        });
        command.stderr(if stderr_full {
            Stdio::from(full())
        } else {
            Stdio::null()
        });
        command.status().expect("lyncast runs").code()
    };
    assert_eq!(run(&["query", "", "--json"], true, false), Some(1));
    assert_eq!(run(&["query", "", "--json"], true, true), Some(1));
    assert_eq!(run(&["--help"], true, false), Some(1));
    assert_eq!(run(&["--version"], true, false), Some(1));
    assert_eq!(run(&["query", "--help"], true, false), Some(1));
    assert_eq!(run(&["--help"], false, false), Some(0));
    assert_eq!(run(&["query"], false, true), Some(2));
    assert_eq!(run(&["query"], false, false), Some(2));
}
