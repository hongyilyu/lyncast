//! docs/design/testing.md's blocks. Each `judge` block runs `bench/summon.py --judge` on a `run`
//! block, the JSON of a bench run, and expects the verdicts that the block lists. Each `cli` block
//! runs its commands with bash in the repository and expects what each prints.

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::iter;
use std::path::Path;
use std::process::{Command, Stdio};

use common::doc::{self, Block};
use common::run_bounded;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const RUNS: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/bench-runs");
const CLI: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/bench-cli");
const SUMMON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/summon.py");

fn said(command: Command, dir: &Path) -> (Vec<String>, Vec<u8>) {
    let (stdout, stderr, code) = run_bounded(command, dir).expect("the command ends");
    let mut got: Vec<String> = String::from_utf8(stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    match code {
        Some(0) => {}
        code => got.push(format!("exit: {}", code.unwrap_or(-1))),
    }
    (got, stderr)
}

#[test]
fn every_judge_block_judges_as_the_doc_says() {
    let fixtures = doc::TESTING
        .read(&doc::TESTING.text())
        .unwrap_or_else(|error| panic!("testing.md: {error}"));
    let of = |kind: &str| -> Vec<&Block> {
        fixtures.iter().filter(|block| block.kind == kind).collect()
    };
    let runs = Path::new(RUNS);
    match fs::remove_dir_all(runs) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{runs:?}: {error}"),
        _ => {}
    }
    fs::create_dir_all(runs).unwrap();
    let named: HashMap<&str, &Block> = of("run")
        .into_iter()
        .map(|block| (block.args[0].as_str(), block))
        .collect();
    for (name, block) in &named {
        fs::write(runs.join(name), block.lines.join("\n")).unwrap();
    }
    let judges = of("judge");
    assert_eq!(
        (named.len(), judges.len()),
        (5, 5),
        "testing.md holds 5 run blocks and 5 judge blocks, so a lost block fails"
    );
    let failures: Vec<String> = judges
        .into_iter()
        .filter_map(|block| {
            let (line, want) = block.lines.split_first().expect("a judge block has a line");
            let name = line
                .strip_prefix("$ summon.py --judge ")
                .unwrap_or_else(|| panic!("line {}: {line:?} is not a judge line", block.line));
            assert!(
                named.contains_key(name),
                "line {}: no run block {name}",
                block.line
            );
            let mut command = Command::new("python3");
            command
                .args(["-I", SUMMON, "--judge"])
                .arg(runs.join(name))
                .env_clear()
                .env("PATH", "/usr/bin:/bin");
            let (got, stderr) = said(command, runs);
            (got != want || !stderr.is_empty()).then(|| {
                format!(
                    "line {}: want {want:#?}\n  got {got:#?}\n  stderr {}",
                    block.line,
                    String::from_utf8_lossy(&stderr)
                )
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} testing.md judge blocks fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_cli_block_runs_as_the_doc_says() {
    let blocks = doc::TESTING.blocks("cli");
    assert_eq!(
        blocks.len(),
        1,
        "testing.md holds 1 cli block, so a lost block fails"
    );
    let dir = Path::new(CLI);
    fs::create_dir_all(dir).unwrap();
    let mut failures = Vec::new();
    for block in &blocks {
        let mut lines = block.lines.iter().peekable();
        while let Some(line) = lines.next() {
            let script = line
                .strip_prefix("$ ")
                .unwrap_or_else(|| panic!("line {}: {line:?} is not a command", block.line));
            let want: Vec<String> =
                iter::from_fn(|| lines.next_if(|next| !next.starts_with("$ ")).cloned()).collect();
            let mut command = Command::new("bash");
            command
                .args(["-c", script])
                .current_dir(ROOT)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .stdin(Stdio::null());
            let (got, stderr) = said(command, dir);
            if got != want || !stderr.is_empty() {
                failures.push(format!(
                    "line {}: {line}\n  want {want:#?}\n  got {got:#?}\n  stderr {}",
                    block.line,
                    String::from_utf8_lossy(&stderr)
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} testing.md cli commands fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
