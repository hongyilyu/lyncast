//! The calculator's fixtures. Every `fixtures` block in docs/design/calculator.md runs here, so
//! that page is the only copy. Each line reads `input => echo | badge | display | copy | words`.

use std::fs;
use std::path::Path;

use lyncast_core::calc::{Env, Outcome, evaluate};

/// The outcome in the fixture spelling: the card's five columns, or ∅ for no card.
fn card(input: &str) -> String {
    match evaluate(input, &Env::default()) {
        Outcome::Card(a) => format!(
            "{} | {} | {} | {} | {}",
            a.expression(),
            a.source_badge(),
            a.display(),
            a.copy(),
            a.target_badge().unwrap_or("none")
        ),
        Outcome::Error(error) => format!("error card: {error}"),
        Outcome::Silent => "∅".into(),
    }
}

/// Every fixture line in the calculator doc, as (input, expected outcome).
fn doc_fixtures() -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/design/calculator.md");
    let doc = fs::read_to_string(&path).expect("docs/design/calculator.md is readable");
    let mut fixtures = Vec::new();
    let mut in_block = false;
    for line in doc.lines() {
        if !in_block {
            // A misspelled or indented fence would drop its block without a word.
            let fence =
                line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~");
            assert!(
                !fence || !line.contains("fixture") || line == "```fixtures",
                "a fixtures fence must read exactly ```fixtures: {line:?}"
            );
            in_block = line == "```fixtures";
        } else if line == "```" {
            in_block = false;
        } else {
            fixtures.push(fixture(line));
        }
    }
    assert!(!in_block, "a fixtures block in calculator.md never closes");
    fixtures
}

fn fixture(line: &str) -> (String, String) {
    let (input, want) = line
        .split_once(" => ")
        .unwrap_or_else(|| panic!("not a fixture line: {line:?}"));
    assert!(
        want == "∅" || want.split(" | ").count() == 5,
        "a card needs five columns: {line:?}"
    );
    let input = input
        .strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(input);
    (input.to_owned(), want.to_owned())
}

#[test]
fn every_fixture_in_the_doc_holds() {
    let fixtures = doc_fixtures();
    // The plan's done-when inputs, then one input from each later block. Without these the
    // test would pass on a doc that lost a block.
    for input in [
        "12×34",
        "1920x1080",
        "0x10",
        "2026-10-14",
        "123456789012.34+0",
        "4(2+3)",
        ".5+.5",
        "-(2+3)",
    ] {
        assert!(
            fixtures.iter().any(|(i, _)| i == input),
            "calculator.md has no fixture for {input}"
        );
    }
    let failures: Vec<String> = fixtures
        .iter()
        .filter_map(|(input, want)| {
            let got = card(input);
            (got != *want).then(|| format!("{input:?}\n  want {want}\n  got  {got}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} fixtures fail:\n{}",
        failures.len(),
        fixtures.len(),
        failures.join("\n")
    );
}

fn nest(depth: usize, inner: &str) -> String {
    format!("{}{inner}{}", "(".repeat(depth), ")".repeat(depth))
}

/// The inputs too big for a fixture line, from the doc's "Generated inputs" section.
fn generated() -> Vec<(String, String)> {
    let join = |n: usize, term: &str, op: &str| vec![term; n].join(op);
    let prices: Vec<String> = (0..33).map(|i| format!("{}.{i:02}", 10 + i)).collect();
    let prices = prices.join(" + ");
    assert_eq!(prices.len(), 261);
    let card = |echo: String, badge: &str, display: &str, copy: &str, words: &str| {
        format!("{echo} | {badge} | {display} | {copy} | {words}")
    };
    let sum = |echo: String, total: &str, words: &str| card(echo, "Sum", total, total, words);
    vec![
        (format!("{}+1", "9".repeat(100_000)), "∅".into()),
        (nest(100_000, "1+1"), "∅".into()),
        (
            format!("10{}", "+1".repeat(511)),
            sum(
                format!("10 + {}", join(511, "1", " + ")),
                "521",
                "Five Hundred Twenty-One",
            ),
        ),
        (format!("100{}", "+1".repeat(511)), "∅".into()),
        (
            format!("1+1{}", " ".repeat(4000)),
            sum(join(2, "1", " + "), "2", "Two"),
        ),
        (
            format!("{}１", "１＋".repeat(200)),
            sum(join(201, "1", " + "), "201", "Two Hundred One"),
        ),
        (
            format!("{}１１", "１＋".repeat(511)),
            sum(
                format!("{} + 11", join(511, "1", " + ")),
                "522",
                "Five Hundred Twenty-Two",
            ),
        ),
        (
            format!("1{}+1", "\u{3000}".repeat(400)),
            sum(join(2, "1", " + "), "2", "Two"),
        ),
        (nest(32, "1+1"), sum(join(2, "1", " + "), "2", "Two")),
        (nest(33, "1+1"), "∅".into()),
        (format!("{}(1+1)", "-".repeat(32)), "∅".into()),
        (prices.clone(), sum(prices, "863.28", "none")),
        (
            format!("1{}", "*1".repeat(511)),
            card(join(512, "1", " × "), "Product", "1", "1", "One"),
        ),
        (
            "(1)".repeat(341),
            card(
                format!("1{}", "(1)".repeat(340)),
                "Product",
                "1",
                "1",
                "One",
            ),
        ),
        (
            nest(32, &format!("1{}", "+1".repeat(470))),
            sum(join(471, "1", " + "), "471", "Four Hundred Seventy-One"),
        ),
        (
            format!("{}1{}", "1+2×(".repeat(32), ")".repeat(32)),
            // The innermost group is `(1)`, which needs no parentheses.
            card(
                format!("{}1 + 2 × 1{}", "1 + 2 × (".repeat(31), ")".repeat(31)),
                "Sum",
                "8,589,934,591",
                "8589934591",
                "none",
            ),
        ),
        (
            format!("{}(1+1)", "-".repeat(31)),
            card(
                format!("{}1 + 1{}", "−(".repeat(31), ")".repeat(31)),
                "Negation",
                "-2",
                "-2",
                "Minus Two",
            ),
        ),
        (
            format!("{}2{}", "2(".repeat(32), ")".repeat(32)),
            card(
                format!("{}2{}", "2(".repeat(32), ")".repeat(32)),
                "Product",
                "8,589,934,592",
                "8589934592",
                "none",
            ),
        ),
    ]
}

/// Rust gives a new thread 2 MiB. The deepest of these inputs needs 148 KB in a debug build
/// and 44 KB in release, so the bound has room to spare.
const SMALL_STACK: usize = 256 * 1024;

#[test]
fn generated_inputs_hold_on_a_small_stack() {
    // An overflow aborts the test binary, which fails the run.
    std::thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(|| {
            for (input, want) in generated() {
                assert_eq!(card(&input), want, "input of {} bytes", input.len());
            }
        })
        .expect("the thread starts")
        .join()
        .expect("every input gives its outcome");
}
