mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use common::clipboard::Helpers;
use common::doc::{self, Block};
use common::{OK, Ran, World, poll, session_with};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/palette-trees");

#[test]
fn every_summon_block_runs_as_the_doc_says() {
    let fixtures = doc::PALETTE
        .read(&doc::PALETTE.text())
        .unwrap_or_else(|error| panic!("palette.md: {error}"));
    let blocks: Vec<&Block> = fixtures
        .iter()
        .filter(|block| block.kind == "summon")
        .collect();
    assert_eq!(
        blocks.len(),
        8,
        "palette.md holds 8 summon blocks, so a lost block fails"
    );
    let trees = Path::new(TREES);
    match fs::remove_dir_all(trees) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{trees:?}: {error}"),
        _ => {}
    }
    let failures: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| run(&fixtures, block, trees.join(block.line.to_string())))
        .collect();
    assert!(
        failures.is_empty(),
        "{} palette.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn run(fixtures: &[Block], block: &Block, tree: PathBuf) -> Vec<String> {
    let world = World::compositor(fixtures, tree);
    let mut helpers = Helpers::new(&world);
    let failures = session_with(&world, block, &mut |line, newest| {
        trace(&world, line).or_else(|| helpers.run(line, newest))
    });
    drop(helpers);
    if failures.is_empty() {
        world.passed();
    }
    failures
}

fn trace(world: &World, line: &str) -> Option<Ran> {
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    let stdout = world.runtime.path().join("daemon.stdout");
    let read = || fs::read_to_string(&stdout).unwrap_or_default();
    Some(match words.as_slice() {
        ["wait-trace", event, seq] => {
            let want = format!("PROBE seq={seq} ev={event} ");
            match poll(|| {
                read()
                    .lines()
                    .any(|held| held.starts_with(&want))
                    .then_some(())
            }) {
                Some(()) => OK,
                None => (Vec::new(), read().into_bytes(), Some(1)),
            }
        }
        ["count-trace", event] => {
            let want = format!("ev={event}");
            let count = read()
                .lines()
                .filter(|held| held.split(' ').nth(2) == Some(want.as_str()))
                .count();
            (format!("{count}\n").into_bytes(), Vec::new(), Some(0))
        }
        _ => return None,
    })
}

#[test]
#[ignore = "the clipboard verbs run it as a helper"]
fn helper() {
    common::clipboard::helper();
}
