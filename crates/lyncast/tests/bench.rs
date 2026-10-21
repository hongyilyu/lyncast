//! docs/design/testing.md's blocks. Each `judge` block runs `bench/summon.py --judge` on a `run`
//! block, the JSON of a bench run, and expects the verdicts that the block lists. Each `cli` block
//! runs its commands with bash in the repository and expects what each prints.

mod common;

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::process::{Command, Stdio};

use common::cli;
use common::doc::{self, Block};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const RUNS: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/bench-runs");
const CLI: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/bench-cli");
const SUMMON: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/summon.py");

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
        .flat_map(|block| {
            cli(block, runs, |line| {
                let name = line
                    .strip_prefix("summon.py --judge ")
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
                command
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
    let failures: Vec<String> = blocks
        .iter()
        .flat_map(|block| {
            cli(block, dir, |script| {
                let mut command = Command::new("bash");
                command
                    .args(["-c", script])
                    .current_dir(ROOT)
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .stdin(Stdio::null());
                command
            })
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} testing.md cli commands fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
