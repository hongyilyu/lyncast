mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use common::clipboard::Helpers;
use common::doc::{self, Block};
use common::{OK, Ran, World, poll, run_bounded, session_later};

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
        10,
        "palette.md holds 10 summon blocks, so a lost block fails"
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
    let keys = |line: &str| line.starts_with("$ keys ");
    let failures = session_later(
        &world,
        block,
        &mut |line, newest| {
            trace(&world, line)
                .or_else(|| palette(&world, line))
                .or_else(|| helpers.run(line, newest))
        },
        &keys,
    );
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

const X11: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/x11.py");

fn palette(world: &World, line: &str) -> Option<Ran> {
    let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
    Some(match words.as_slice() {
        ["keys", specs @ ..] => {
            let display = fs::read_to_string(world.runtime.path().join("display"))
                .expect("test-env.sh writes the display");
            let typed = format!(
                "$ python3 -I {X11} {} focus keys {}",
                display.trim(),
                specs.join(" ")
            );
            run_bounded(world.command(&typed).0, &world.out)?
        }
        ["wait-summon", state] => {
            let want = format!("{state}\n");
            let mut last = Vec::new();
            let status = || run_bounded(world.command("$ lyncast status").0, &world.out);
            let waited = poll(|| {
                let (stdout, _, _) = status()?;
                if stdout == want.as_bytes() {
                    return Some(());
                }
                last = stdout;
                None
            });
            match waited {
                Some(()) => OK,
                None => (last, Vec::new(), Some(1)),
            }
        }
        _ => return None,
    })
}

#[test]
#[ignore = "the clipboard verbs run it as a helper"]
fn helper() {
    common::clipboard::helper();
}
