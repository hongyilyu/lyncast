//! Clipboard history end to end. The test reads every `env` and `session` block of
//! docs/design/clipboard-and-paste.md and runs each session as daemon.md's tests do: in a fresh
//! tree, `$TREE`, with a fresh `XDG_RUNTIME_DIR` under `/tmp`, `$RUNTIME`, and only the `env`
//! block's variables.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use common::doc::{self, Block};
use common::{World, session};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/clipboard-trees");

#[test]
fn every_session_block_runs_as_the_doc_says() {
    let text = doc::CLIPBOARD.text();
    let fixtures = doc::CLIPBOARD
        .read(&text)
        .unwrap_or_else(|error| panic!("clipboard-and-paste.md: {error}"));
    let sessions: Vec<&Block> = fixtures
        .iter()
        .filter(|block| block.kind == "session")
        .collect();
    assert_eq!(
        sessions.len(),
        11,
        "clipboard-and-paste.md holds 11 session blocks, so a lost block fails"
    );
    // A failed block keeps its tree, so the trees of the last run go first.
    match fs::remove_dir_all(TREES) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{TREES}: {error}"),
        _ => {}
    }
    let failures: Vec<String> = sessions
        .iter()
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
        "{} clipboard-and-paste.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
