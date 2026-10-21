//! A `clipboard` block's helper lines run this test binary again as a helper in the block's
//! environment, so no wl-clipboard-rs call in the test process can reach the developer's own
//! clipboard. The helper lines are the verbs in `common/clipboard.rs`.

mod common;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::clipboard::{CAPTURE, Helpers, wait_read};
use common::doc::{self, Block};
use common::{BUDGET, Daemon, OK, World, poll, run_bounded, session, session_later};

const TREES: &str = concat!(env!("CARGO_TARGET_TMPDIR"), "/clipboard-trees");

fn fixtures() -> Vec<Block> {
    doc::CLIPBOARD
        .read(&doc::CLIPBOARD.text())
        .unwrap_or_else(|error| panic!("clipboard-and-paste.md: {error}"))
}

fn blocks<'f>(fixtures: &'f [Block], kind: &str, count: usize) -> Vec<&'f Block> {
    let blocks: Vec<&Block> = fixtures.iter().filter(|block| block.kind == kind).collect();
    assert_eq!(
        blocks.len(),
        count,
        "clipboard-and-paste.md holds {count} {kind} blocks, so a lost block fails"
    );
    blocks
}

fn fresh(name: &str) -> PathBuf {
    let trees = Path::new(TREES).join(name);
    match fs::remove_dir_all(&trees) {
        Err(error) if error.kind() != ErrorKind::NotFound => panic!("{trees:?}: {error}"),
        _ => {}
    }
    trees
}

/// A headless daemon in `world`, once it serves.
fn headless(world: &World) -> Daemon {
    let (daemon, _) = world.command("$ lyncast daemon --headless");
    Daemon::start(daemon, &world.runtime).unwrap()
}

fn assert_passed(failures: &[String]) {
    assert!(
        failures.is_empty(),
        "{} clipboard-and-paste.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_session_block_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let trees = fresh("session");
    let failures: Vec<String> = blocks(&fixtures, "session", 12)
        .into_iter()
        .flat_map(|block| {
            let world = World::new(&fixtures, trees.join(block.line.to_string()));
            let failures = session(&world, block);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert_passed(&failures);
}

#[test]
fn every_clipboard_block_runs_as_the_doc_says() {
    let fixtures = fixtures();
    let trees = fresh("clipboard");
    // The capture thread asks niri about an offer before or after the seed's line returns.
    let seeds = |line: &str| line.starts_with("$ seed");
    let failures: Vec<String> = blocks(&fixtures, "clipboard", 34)
        .into_iter()
        .flat_map(|block| {
            let world = World::compositor(&fixtures, trees.join(block.line.to_string()));
            let mut helpers = Helpers::new(&world);
            let mut verbs = |line: &str, newest| helpers.run(line, newest);
            let failures = session_later(&world, block, &mut verbs, &seeds);
            drop(helpers);
            if failures.is_empty() {
                world.passed();
            }
            failures
        })
        .collect();
    assert_passed(&failures);
}

const QUIT_BOUND: Duration = Duration::from_millis(2500);

const QUIT_AFTER: Duration = Duration::from_millis(2500);

#[test]
fn quit_reads_no_offer_queued_behind_the_one_in_hand() {
    let world = World::compositor(&fixtures(), fresh("quit").join("tree"));
    let mut daemon = headless(&world);
    let mut helpers = Helpers::new(&world);
    let mut first = None;
    for text in ["one", "two", "three", "four"] {
        let frozen = common::Frozen::new(daemon.id(), CAPTURE).unwrap();
        for line in [
            &format!("$ seed text/plain;charset=utf-8 {text}"),
            "$ stop-seed",
        ] {
            let ran = helpers.run(line, Some(daemon.id())).expect(line);
            assert_eq!(ran, OK, "{line}");
        }
        drop(frozen);
        first.get_or_insert_with(Instant::now);
    }
    let first = first.expect("a seed ran");
    thread::sleep((first + QUIT_AFTER).saturating_duration_since(Instant::now()));
    let (list, _) = world.command("$ lyncast clip list --json");
    let (rows, _, _) = run_bounded(list, &world.out).expect("the list replies");
    assert_eq!(String::from_utf8_lossy(&rows), "", "a seed was read");
    let (quit, _) = world.command("$ lyncast activate cmd:quit");
    let sent = Instant::now();
    let quit = run_bounded(quit, &world.out).expect("Quit replies");
    let finished = daemon.finish();
    let took = sent.elapsed();
    assert_eq!(quit, (b"ran cmd:quit\n".to_vec(), Vec::new(), Some(0)));
    assert_eq!(finished, Some((Vec::new(), Some(0))));
    assert!(took < QUIT_BOUND, "the daemon exited {took:?} after Quit");
    drop(helpers);
    world.passed();
}

const ROUNDS: usize = 10;

#[test]
fn copies_at_once_record_in_the_order_the_compositor_took_them() {
    let world = World::compositor(&fixtures(), fresh("at-once").join("tree"));
    let daemon = headless(&world);
    let mut helpers = Helpers::new(&world);
    let mut run = |line: &str| helpers.run(line, Some(daemon.id())).expect(line);
    for line in [
        "$ seed text/plain;charset=utf-8 alpha",
        "$ wait-clips 1",
        "$ seed text/plain;charset=utf-8 beta",
        "$ wait-clips 2",
    ] {
        assert_eq!(run(line), OK, "{line}");
    }
    for round in 0..ROUNDS {
        let copies: Vec<(Child, PathBuf)> = [1, 2]
            .into_iter()
            .map(|id| {
                let (mut copy, _) = world.command(&format!("$ lyncast activate clip:{id}"));
                let out = world.out.join(format!("copy-{id}"));
                let stdout = fs::File::create(&out).unwrap();
                (copy.stdout(stdout).spawn().unwrap(), out)
            })
            .collect();
        for ((mut copy, out), id) in copies.into_iter().zip([1, 2]) {
            if poll(|| copy.try_wait().unwrap()).is_none() {
                let _ = copy.kill();
                panic!("round {round}: the copy of clip {id} ran over the test's time budget");
            }
            assert_eq!(
                fs::read_to_string(out).unwrap(),
                format!("copied clip {id}\n"),
                "round {round}"
            );
        }
        let (list, _) = world.command("$ lyncast clip list --json");
        let (list, _, _) = run_bounded(list, &world.out).unwrap();
        let first = String::from_utf8(list).unwrap();
        let first = first.lines().next().unwrap().to_owned();
        assert_eq!(run("$ paste text/plain;charset=utf-8 got"), OK);
        let pasted = fs::read_to_string(world.tree.join("got")).unwrap();
        assert!(
            first.ends_with(&format!("\"preview\":\"{pasted}\",\"source\":null}}")),
            "round {round}: the clipboard holds {pasted:?}, and the first row is {first}"
        );
    }
    drop(helpers);
    drop(daemon);
    world.passed();
}

#[test]
fn wait_read_waits_past_a_served_copy() {
    let world = World::compositor(&fixtures(), fresh("wait-read").join("tree"));
    let daemon = headless(&world);
    let pid = daemon.id();
    let mut helpers = Helpers::new(&world);
    for line in ["$ seed text/plain;charset=utf-8 alpha", "$ wait-clips 1"] {
        assert_eq!(helpers.run(line, Some(pid)).expect(line), OK, "{line}");
    }
    let (copy, _) = world.command("$ lyncast activate clip:1");
    let copied = run_bounded(copy, &world.out).expect("the copy replies");
    assert_eq!(copied, (b"copied clip 1\n".to_vec(), Vec::new(), Some(0)));
    let (done, waited) = mpsc::channel();
    thread::spawn(move || done.send(wait_read(pid)));
    assert!(
        waited.recv_timeout(Duration::from_secs(1)).is_err(),
        "wait-read returned while no offer was read"
    );
    let frozen = common::Frozen::new(pid, CAPTURE).unwrap();
    for line in ["$ seed text/plain;charset=utf-8 late", "$ stop-seed"] {
        assert_eq!(helpers.run(line, Some(pid)).expect(line), OK, "{line}");
    }
    drop(frozen);
    assert_eq!(waited.recv_timeout(BUDGET).expect("wait-read returns"), OK);
    drop(helpers);
    drop(daemon);
    world.passed();
}

#[test]
#[ignore = "the clipboard verbs run it as a helper"]
fn helper() {
    common::clipboard::helper();
}
