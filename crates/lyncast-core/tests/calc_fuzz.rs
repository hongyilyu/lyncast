//! `evaluate` never panics. A seeded run of random queries in the test profile, which keeps
//! overflow checks on, with no fuzzing crate.

use std::panic;

use lyncast_core::calc::{Env, Outcome, evaluate};

/// splitmix64, so a failing run replays from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Pieces that reach every branch of the lexer: each operator spelling, multibyte symbols,
/// full-width forms, radix and date shapes, words, square roots (which spell nothing), and
/// characters no token starts with.
const PIECES: &[&str] = &[
    "+",
    "-",
    "−",
    "*",
    "×",
    "·",
    "x",
    "X",
    "/",
    "÷",
    "√",
    "sqrt",
    "square root of",
    "(",
    ")",
    " ",
    ".",
    "0",
    "7",
    "12",
    "0x",
    "0b1",
    "2026-10-14",
    "10/14/2026",
    "1e5",
    ",",
    "=",
    "%",
    "（",
    "）",
    "＋",
    "１",
    "\u{3000}",
    "é",
    "日",
    "🙂",
    "\u{301}",
    "\t",
    "of",
    "km",
    "SQRT",
];

fn digits(rng: &mut Rng, n: usize) -> String {
    (0..n)
        .map(|_| char::from(b'0' + rng.below(10) as u8))
        .collect()
}

/// Mostly short, sometimes up to 45 digits so literals and products pass `i128`, and sometimes
/// with a fraction.
fn number(rng: &mut Rng) -> String {
    let max = if rng.below(8) == 0 { 45 } else { 4 };
    let len = 1 + rng.below(max);
    let int = digits(rng, len);
    match rng.below(16) {
        0 => {
            let len = 1 + rng.below(20);
            format!("{int}.{}", digits(rng, len))
        }
        1..4 => {
            let len = 1 + rng.below(3);
            format!("{int}.{}", digits(rng, len))
        }
        _ => int,
    }
}

/// An expression from the grammar, so many of these give cards.
fn expression(rng: &mut Rng, depth: u32) -> String {
    match rng.below(if depth > 3 { 2 } else { 8 }) {
        0 | 1 => number(rng),
        2 => format!("({})", expression(rng, depth + 1)),
        3 => {
            let prefix = rng.pick(&["-", "−"]);
            format!("{prefix}{}", expression(rng, depth + 1))
        }
        4 => format!("{}({})", number(rng), expression(rng, depth + 1)),
        _ => {
            let lhs = expression(rng, depth + 1);
            let op = rng.pick(&["+", " - ", "−", "*", "x", " × ", "/", "÷", "·"]);
            format!("{lhs}{op}{}", expression(rng, depth + 1))
        }
    }
}

/// Pieces in a random order, which is mostly noise.
fn soup(rng: &mut Rng) -> String {
    let len = 1 + rng.below(30);
    (0..len)
        .map(|_| {
            if rng.below(3) == 0 {
                number(rng)
            } else {
                rng.pick(PIECES).to_owned()
            }
        })
        .collect()
}

fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// 200,000 inputs in all, as the notes/08 judge ran, split so the test threads share them.
const RUNS: usize = 50_000;

/// Runs `RUNS` inputs from `seed`, half from the grammar and half noise, and checks every card.
fn run(seed: u64) {
    let mut rng = Rng(seed);
    let env = Env::default();
    let (mut cards, mut words) = (0, 0);
    for run in 0..RUNS {
        let input = if run % 2 == 0 {
            expression(&mut rng, 0)
        } else {
            soup(&mut rng)
        };
        let outcome = panic::catch_unwind(|| evaluate(&input, &env))
            .unwrap_or_else(|_| panic!("evaluate panicked on {input:?} (seed {seed:#x})"));
        let Outcome::Card(card) = outcome else {
            continue;
        };
        cards += 1;
        assert_eq!(card.display().replace(',', ""), card.copy(), "{input:?}");
        assert!(
            card.copy().parse::<f64>().is_ok_and(f64::is_finite),
            "{input:?} copies {}",
            card.copy()
        );
        if card.target_badge().is_some() {
            words += 1;
            assert!(is_integer(card.copy()), "{input:?} spells {}", card.copy());
        }
    }
    // Without these the run would pass if evaluate always returned Silent.
    assert!(cards > RUNS / 5, "only {cards} of {RUNS} inputs gave cards");
    assert!(words > RUNS / 100, "only {words} of {RUNS} cards had words");
}

#[test]
fn never_panics_seed_1() {
    run(1);
}

#[test]
fn never_panics_seed_2() {
    run(2);
}

#[test]
fn never_panics_seed_3() {
    run(3);
}

#[test]
fn never_panics_seed_4() {
    run(4);
}
