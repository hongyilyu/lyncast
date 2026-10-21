//! docs/design/install.md's blocks. The shipped files are its `systemd` and `kdl` blocks, the
//! README holds its `install` and `startup` blocks, its `cli` block runs the install steps in a
//! fresh home, each `argv` step runs a shipped command with the test's build, and the unit's
//! restart lines match the exit codes the docs show.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::slice;

use common::doc::{self, Block};
use common::{Printed, Runtime, cli, lyncast, run_bounded, steps};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const HOME: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/install-home");

fn fixtures() -> Vec<Block> {
    doc::INSTALL
        .read(&doc::INSTALL.text())
        .unwrap_or_else(|error| panic!("install.md: {error}"))
}

fn one<'f>(fixtures: &'f [Block], kind: &str) -> &'f Block {
    let found: Vec<&Block> = fixtures.iter().filter(|block| block.kind == kind).collect();
    let [block] = found[..] else {
        panic!("install.md holds one {kind} block, not {}", found.len());
    };
    block
}

/// A block's lines as the file they stand for.
fn text(block: &Block) -> String {
    block.lines.iter().map(|line| format!("{line}\n")).collect()
}

/// The repository file that a `systemd` or `kdl` block names.
fn shipped(block: &Block) -> &str {
    match &block.args[..] {
        [file] => file,
        _ => panic!("line {}: the block names one file", block.line),
    }
}

#[test]
fn the_install_files_are_the_doc_blocks() {
    let fixtures = fixtures();
    for kind in ["systemd", "kdl"] {
        let block = one(&fixtures, kind);
        let file = shipped(block);
        let read = fs::read_to_string(Path::new(ROOT).join(file))
            .unwrap_or_else(|error| panic!("{file}: {error}"));
        assert_eq!(
            read,
            text(block),
            "{file} differs from install.md's block at line {}",
            block.line
        );
    }
}

#[test]
fn the_readme_holds_the_install_steps() {
    let fixtures = fixtures();
    let readme = fs::read_to_string(Path::new(ROOT).join("README.md")).unwrap();
    for (kind, fence) in [("install", "```sh"), ("startup", "```kdl")] {
        let fenced = format!("{fence}\n{}```\n", text(one(&fixtures, kind)));
        assert!(
            readme.contains(&fenced),
            "README.md holds install.md's {kind} block in a {fence} fence"
        );
    }
}

/// A fresh home for the `cli` block: a checkout in `src` with the shipped files and the install
/// steps as `install.sh`, which read `/etc/niri/` as `~/etc/niri/`, and stubs that print their
/// command line for the build in `src` and for `cargo`, `niri`, and `systemctl` in `bin`.
fn home(fixtures: &[Block]) -> PathBuf {
    let home = PathBuf::from(HOME);
    match fs::remove_dir_all(&home) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{home:?}: {error}"),
        _ => {}
    }
    let src = home.join("src");
    let write = |path: PathBuf, text: &str, mode: u32| {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
    };
    for kind in ["systemd", "kdl"] {
        let block = one(fixtures, kind);
        write(src.join(shipped(block)), &text(block), 0o644);
    }
    let install = text(one(fixtures, "install")).replace("/etc/niri/", "~/etc/niri/");
    write(src.join("install.sh"), &install, 0o644);
    for stub in [
        "bin/cargo",
        "bin/niri",
        "bin/systemctl",
        "src/target/release/lyncast",
    ] {
        write(home.join(stub), "#!/bin/sh\necho \"${0##*/} $*\"\n", 0o755);
    }
    home
}

#[test]
fn every_cli_block_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let home = home(&fixtures);
    let runtime = Runtime::new();
    let out = home.join("out");
    fs::create_dir(&out).unwrap();
    let failures = cli(one(&fixtures, "cli"), &out, |script| {
        let mut command = Command::new("bash");
        command
            .args(["-c", script])
            .current_dir(home.join("src"))
            .env_clear()
            .env("HOME", &home)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", home.join("bin").display()),
            )
            .env("XDG_RUNTIME_DIR", runtime.path())
            .stdin(Stdio::null());
        command
    });
    assert!(
        failures.is_empty(),
        "{} install.md cli commands fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
    fs::remove_dir_all(&home).unwrap();
}

/// The words between double quotes in `line`.
fn quoted(line: &str) -> Vec<String> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// Each shipped command by the name of its `argv` step: the bind's `spawn` arguments, the
/// unit's `ExecStart` with `%h/` read as `~/`, and the `spawn-at-startup` line's arguments.
fn commands(fixtures: &[Block]) -> Vec<(&'static str, Vec<String>)> {
    let after = |kind: &str, marker: &str| -> &str {
        let found: Vec<&str> = one(fixtures, kind)
            .lines
            .iter()
            .filter_map(|line| Some(line.split_once(marker)?.1))
            .collect();
        let [rest] = found[..] else {
            panic!("the {kind} block holds one line with {marker:?}");
        };
        rest
    };
    let exec = after("systemd", "ExecStart=%h/");
    vec![
        ("bind", quoted(after("kdl", "{ spawn "))),
        (
            "unit",
            format!("~/{exec}").split(' ').map(str::to_owned).collect(),
        ),
        ("startup", quoted(after("startup", "spawn-at-startup "))),
    ]
}

#[test]
fn every_argv_step_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let install = one(&fixtures, "install");
    let installed = install
        .lines
        .iter()
        .find_map(|line| line.strip_prefix("install -Dm755 target/release/lyncast "))
        .expect("the install block installs the binary");
    let commands = commands(&fixtures);
    let mut failures = Vec::new();
    let mut ran = Vec::new();
    for step in steps(one(&fixtures, "argv")) {
        let name = step
            .command
            .strip_prefix("$ ")
            .unwrap_or_else(|| panic!("line {}: {:?} is not a step", step.line, step.command));
        let (_, argv) = commands
            .iter()
            .find(|(command, _)| *command == name)
            .unwrap_or_else(|| panic!("line {}: no shipped command {name}", step.line));
        let (program, args) = argv.split_first().expect("a command has a program");
        assert_eq!(
            program, installed,
            "{name} runs another program than the install's"
        );
        let runtime = Runtime::new();
        let mut command = lyncast(&runtime, args);
        if args.first().is_some_and(|arg| arg == "daemon") {
            runtime.record(&mut command);
        }
        let out = runtime.path().join("client");
        fs::create_dir(&out).unwrap();
        let (stdout, stderr, code) = run_bounded(command, &out).expect("lyncast ends");
        let at = runtime.path().to_str().unwrap();
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap().replace(at, "$RUNTIME");
        let got = Printed {
            stdout: text(stdout),
            stderr: text(stderr),
            exit: code.unwrap_or(-1),
            ..Printed::default()
        };
        let want = Printed::expected(step.after);
        if got != want {
            failures.push(format!(
                "line {}: {}\n  want {want:?}\n  got  {got:?}",
                step.line, step.command
            ));
        }
        ran.push(name);
    }
    let shipped: Vec<&str> = commands.iter().map(|(name, _)| *name).collect();
    assert_eq!(ran, shipped, "the argv block runs every shipped command");
    assert!(
        failures.is_empty(),
        "install.md argv steps fail:\n{}",
        failures.join("\n")
    );
}

/// The exit code of each step in `blocks` that is `line` or prints it.
fn exits(blocks: &[Block], line: &str) -> Vec<i32> {
    blocks
        .iter()
        .flat_map(steps)
        .filter(|step| step.command == line || step.after.iter().any(|after| after == line))
        .map(|step| Printed::expected(step.after).exit)
        .collect()
}

#[test]
fn the_unit_restarts_only_a_crash() {
    let fixtures = fixtures();
    let unit = one(&fixtures, "systemd");
    let value = |key: &str| {
        unit.lines
            .iter()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or_else(|| panic!("the unit sets {key}"))
    };
    assert_eq!(value("Restart="), "on-failure");
    let kept: i32 = value("RestartPreventExitStatus=")
        .parse()
        .expect("one exit code");
    let refusals = [
        (
            "install.md's unit step",
            exits(slice::from_ref(one(&fixtures, "argv")), "$ unit"),
        ),
        (
            "daemon.md's second daemon",
            exits(
                &doc::DAEMON.blocks("session"),
                "stderr: lyncast: a daemon is already running",
            ),
        ),
        (
            "palette.md's compositor end",
            exits(
                &doc::PALETTE.blocks("summon"),
                "stderr: lyncast: the Wayland connection closed",
            ),
        ),
    ];
    for (refusal, exits) in refusals {
        assert!(!exits.is_empty(), "the docs show {refusal}");
        assert!(
            exits.iter().all(|&exit| exit == kept),
            "{refusal} exits {exits:?}, and the unit keeps only {kept} from a restart"
        );
    }
}
