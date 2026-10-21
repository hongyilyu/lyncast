//! Launching end to end. The test reads every `env`, `desktop`, `executable`, and `session`
//! block of docs/design/launching.md and runs each session block as daemon.md's tests do, in a
//! fresh tree with a fresh `XDG_RUNTIME_DIR`, with its `$ fake-niri` lines.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use common::doc::{self, Block};
use common::{World, session};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/launching-trees");

#[test]
fn every_session_block_runs_as_the_doc_says() {
    let fixtures = doc::LAUNCHING
        .read(&doc::LAUNCHING.text())
        .unwrap_or_else(|error| panic!("launching.md: {error}"));
    let sessions: Vec<&Block> = fixtures
        .iter()
        .filter(|block| block.kind == "session")
        .collect();
    assert_eq!(
        sessions.len(),
        5,
        "launching.md holds 5 session blocks, so a lost block fails"
    );
    match fs::remove_dir_all(TREES) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{TREES}: {error}"),
        _ => {}
    }
    let failures: Vec<String> = sessions
        .into_iter()
        .flat_map(|block| {
            let world = World::new(&fixtures, Path::new(TREES).join(block.line.to_string()));
            let failures = session(&world, block);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} launching.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
