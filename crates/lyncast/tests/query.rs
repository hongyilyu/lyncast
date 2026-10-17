//! `lyncast query` end to end. The test writes the `desktop`, `executable`, `symlink`, and `fifo`
//! blocks of docs/design/ranking.md under a fresh directory. For each `cli` block it starts a
//! daemon with only the `env` block's variables, as the block's command line changes them, and
//! compares the client's stdout byte for byte with the block.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use common::{Daemon, Runtime, command_line, doc, environment, lyncast, materialize, run_bounded};

/// The `desktop`, `executable`, `symlink`, and `fifo` blocks are the files that the "Each file
/// proves one rule" table lists, so a lost block fails, even an `executable` whose loss only lets
/// its entry hide for a second reason.
#[test]
fn every_tree_file_is_in_the_table() {
    let doc = doc::RANKING.text();
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
        .flat_map(|kind| doc::RANKING.blocks(kind))
        .map(|block| block.args[0].clone())
        .collect();
    assert_eq!(files, table);
}

/// Each `executable` block is a program that some `desktop` block's `Exec` or `TryExec` names.
#[test]
fn every_executable_is_a_program_some_entry_names() {
    let named: BTreeSet<String> = doc::RANKING
        .blocks("desktop")
        .iter()
        .flat_map(|block| &block.lines)
        .filter_map(|line| match line.strip_prefix("Exec=") {
            Some(exec) => exec.split(' ').next(),
            None => line.strip_prefix("TryExec="),
        })
        .map(str::to_owned)
        .collect();
    for block in doc::RANKING.blocks("executable") {
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
    let doc = doc::RANKING.text();
    let fixtures = doc::RANKING
        .read(&doc)
        .unwrap_or_else(|error| panic!("ranking.md: {error}"));
    let tree = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ranking-tree");
    materialize(&fixtures, &tree);
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
        let line = command_line(command, &[("$TREE", &tree)]).expect("a cli line runs lyncast");
        assert!(
            line.vars[0]
                .1
                .as_bytes()
                .windows(8)
                .any(|bytes| bytes == b"nowhere\xFF"),
            "{command} holds no raw 0xFF"
        );
    }
    let env = environment(&fixtures, &[("$TREE", &tree)]);
    let blocks = doc::RANKING.blocks("cli");
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
        let line = command_line(command, &[("$TREE", &tree)]).expect("a cli line runs lyncast");
        let runtime = Runtime::new();
        let mut daemon = line.command(["daemon", "--headless"], &env);
        daemon
            .current_dir(&tree)
            .env("XDG_RUNTIME_DIR", runtime.path());
        let daemon = match Daemon::start(daemon, &runtime) {
            Ok(daemon) => daemon,
            Err(error) => {
                failures.push(format!("line {}: {command}\n  {error}", block.line));
                break;
            }
        };
        // The client reads nothing from its environment but the socket path.
        let Some((stdout, stderr, code)) = run_bounded(lyncast(&runtime, &line.args), &out_dir)
        else {
            failures.push(format!(
                "line {}: {command}\n  ran over the test's time budget",
                block.line
            ));
            break;
        };
        drop(daemon);
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
    fs::remove_dir_all(&tree).unwrap();
    fs::remove_dir_all(&out_dir).unwrap();
}

/// A write that fails exits 1, even when the error message cannot be written either, and a usage
/// error keeps clap's 2. The query reaches a daemon, so only the write can fail.
#[test]
fn a_failed_write_exits_1() {
    let runtime = Runtime::new();
    let mut daemon = lyncast(&runtime, ["daemon", "--headless"]);
    daemon.env("XDG_DATA_DIRS", runtime.path());
    let _daemon = Daemon::start(daemon, &runtime).unwrap();
    let full = || {
        fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .expect("/dev/full opens")
    };
    let run = |args: &[&str], stdout_full: bool, stderr_full: bool| {
        let mut command = lyncast(&runtime, args);
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
    assert_eq!(
        run(&["query", "", "--json"], false, false),
        Some(0),
        "the query reaches the daemon"
    );
    assert_eq!(run(&["query", "", "--json"], true, false), Some(1));
    assert_eq!(run(&["query", "", "--json"], true, true), Some(1));
    assert_eq!(run(&["status"], true, false), Some(1));
    assert_eq!(run(&["--help"], true, false), Some(1));
    assert_eq!(run(&["--version"], true, false), Some(1));
    assert_eq!(run(&["query", "--help"], true, false), Some(1));
    assert_eq!(run(&["--help"], false, false), Some(0));
    assert_eq!(run(&["query"], false, true), Some(2));
    assert_eq!(run(&["query"], false, false), Some(2));
}
