//! The calculator's cost per keystroke. Run it with
//! `cargo run --release -p lyncast-core --example calc_bench`.
//!
//! It times the 26 shared fixture inputs the way notes/08 timed the prototypes, 10,000
//! evaluations cycling the list, and then each of the worst inputs under the caps.

use std::hint::black_box;
use std::time::Instant;

use lyncast_core::calc::{Env, Outcome, evaluate};

/// The 26 shared inputs of docs/design/calculator.md, in the order notes/08 timed them.
const MIX: [&str; 26] = [
    "12×34",
    "12*34",
    "12x34",
    "12 x 34",
    "2+3*4",
    "(2+3)*4",
    "10/4",
    "7-10",
    "-5+3",
    "0.1+0.2",
    "1/3*3",
    "sqrt(16)",
    "√16",
    "√(9+16)",
    "sqrt 2",
    "1000*1000",
    "12×34+",
    "(2+3",
    "12",
    "firefox",
    "sqrt",
    "5 3",
    "1.2.3+1",
    "10/0",
    "sqrt(-1)",
    "",
];

/// Timed runs per measurement. The report gives their median and range.
const RUNS: usize = 7;

/// The longest and slowest inputs the caps allow, and the two pastes over them.
fn worst() -> Vec<(&'static str, String)> {
    let nest = |n: usize, inner: &str| format!("{}{inner}{}", "(".repeat(n), ")".repeat(n));
    // Adjacent Fibonacci numbers make Euclid's gcd take its most steps.
    let fib = "127127879743834334146972278486287885163/78569350599398894027251472817058687522";
    // Prices joined by ` + ` up to the 1024-byte cap.
    let mut prices = String::from("10.00");
    for i in 1.. {
        let term = format!(" + {}.{:02}", 10 + i % 90, i % 100);
        if prices.len() + term.len() > 1024 {
            break;
        }
        prices.push_str(&term);
    }
    vec![
        ("100,000-digit paste", format!("{}+1", "9".repeat(100_000))),
        ("100,000 nested parentheses", nest(100_000, "1+1")),
        ("512-term sum", format!("1{}", "+1".repeat(511))),
        ("gcd-heavy sum", format!("{fib}{}", "+0".repeat(473))),
        ("long column of prices", prices),
        (
            "32 groups around 471 terms",
            nest(32, &format!("1{}", "+1".repeat(470))),
        ),
        ("341 juxtaposed groups", "(1)".repeat(341)),
        (
            "2(2(…)) 32 deep",
            format!("{}2{}", "2(".repeat(32), ")".repeat(32)),
        ),
        (
            "1 ÷ 2^100, 100 decimals",
            "1/1267650600228229401496703205376".into(),
        ),
        (
            "full-width 512-term sum",
            format!("{}１", "１＋".repeat(511)),
        ),
    ]
}

fn main() {
    let env = Env::default();
    // A slip that silenced every input would time the gate alone.
    let cards = MIX
        .iter()
        .filter(|q| matches!(evaluate(q, &env), Outcome::Card(_)))
        .count();
    // The MVP has no square root, so the four root inputs that gave cards in notes/08 are silent.
    assert_eq!(cards, 13, "the shared mix gives 13 cards");
    let n = 10_000;
    let mix = measure(n, || {
        for q in MIX.iter().cycle().take(n) {
            black_box(evaluate(black_box(q), &env));
        }
    });
    report("26 shared inputs, per evaluation", &mix);
    for (name, input) in worst() {
        let reps = 200;
        let times = measure(reps, || {
            for _ in 0..reps {
                black_box(evaluate(black_box(&input), &env));
            }
        });
        let card = matches!(evaluate(&input, &env), Outcome::Card(_));
        let label = format!(
            "{name} ({} bytes, {})",
            input.len(),
            if card { "card" } else { "∅" }
        );
        report(&label, &times);
    }
}

/// Microseconds per call for each of `RUNS` timed runs of `work`, which makes `calls` calls.
fn measure(calls: usize, mut work: impl FnMut()) -> Vec<f64> {
    work();
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            work();
            start.elapsed().as_secs_f64() * 1e6 / calls as f64
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times
}

fn report(label: &str, sorted: &[f64]) {
    println!(
        "{label:<52} median {:>9.3} µs   range {:.3} to {:.3}",
        sorted[sorted.len() / 2],
        sorted[0],
        sorted[sorted.len() - 1]
    );
}
