//! Round once, then render twice: grouped for display and plain for copy. The words come from
//! the same rounded digits, so the badge never disagrees with the number.

use std::fmt::{self, Write};

use super::quantity::Num;

/// A repeating decimal shows this many significant digits, but never fewer than its integer
/// digits.
const SIG: usize = 12;
/// A repeating decimal smaller than 10^-7 prints in scientific notation.
const MIN_PLAIN_EXP: i32 = -7;
/// Longer words make a pill nobody reads, so the badge is dropped (decision log).
const WORDS_MAX: usize = 40;

pub(super) struct Shown {
    pub(super) display: String,
    pub(super) copy: String,
    pub(super) words: Option<String>,
}

pub(super) fn show(num: Num) -> Shown {
    let rounded = rounded(num);
    Shown {
        display: rounded.render(true),
        copy: rounded.render(false),
        words: rounded.words(),
    }
}

/// Writes a literal as the engine reads it: `.5` as `0.5`, `007` as `7`, `5.` as `5`, and
/// `1000.50` as `1,000.50`. Fraction digits stay as typed.
pub(super) fn write_literal(out: &mut impl Write, text: &str) -> fmt::Result {
    let (int, frac) = text.split_once('.').unwrap_or((text, ""));
    let int = int.trim_start_matches('0');
    grouped(if int.is_empty() { "0" } else { int }).try_for_each(|c| out.write_char(c))?;
    if !frac.is_empty() {
        write!(out, ".{frac}")?;
    }
    Ok(())
}

/// ASCII digits with a comma before each group of three from the right.
fn grouped(digits: &str) -> impl Iterator<Item = char> + '_ {
    let len = digits.len();
    digits.chars().enumerate().flat_map(move |(i, c)| {
        let comma = (i > 0 && (len - i).is_multiple_of(3)).then_some(',');
        comma.into_iter().chain([c])
    })
}

struct Rounded {
    neg: bool,
    digits: Vec<u8>,
    /// The power of ten of the first digit.
    exp: i32,
    plain: bool,
}

/// How many significant digits a value with first digit at 10^`exp` keeps.
fn keep(exp: i32) -> usize {
    usize::try_from(exp + 1).map_or(SIG, |int_digits| SIG.max(int_digits))
}

/// Digits by long division. A decimal that terminates keeps every digit and prints plain at any
/// size. A repeating one stops one digit past what it keeps, then rounds, and prints plain
/// unless it is tiny.
fn rounded(r: Num) -> Rounded {
    let (n, d) = (r.n.unsigned_abs(), r.d.unsigned_abs());
    let terminates = {
        let mut d = d >> d.trailing_zeros();
        while d % 5 == 0 {
            d /= 5;
        }
        d == 1
    };
    let int = n / d;
    let mut rem = n % d;
    let mut digits: Vec<u8> = if int > 0 {
        int.to_string().bytes().map(|b| b - b'0').collect()
    } else {
        Vec::new()
    };
    let mut exp = digits.len() as i32 - 1;
    while rem != 0 && (terminates || digits.len() <= keep(exp)) {
        let digit = next_digit(&mut rem, d);
        if digits.is_empty() && digit == 0 {
            exp -= 1;
        } else {
            digits.push(digit);
        }
    }
    if digits.is_empty() {
        digits.push(0);
    }
    let mut rounded = Rounded {
        neg: r.n < 0,
        digits,
        exp,
        plain: true,
    };
    if !terminates {
        rounded.round(keep(rounded.exp));
        rounded.plain = rounded.exp >= MIN_PLAIN_EXP;
    }
    rounded.trim();
    rounded
}

/// The next digit of `rem / d` is `10 * rem / d`, and `10 * rem % d` is left in `rem`. Adding
/// `rem` ten times stays under `2 * d`, which fits in a `u128` because `d` came from an `i128`.
fn next_digit(rem: &mut u128, d: u128) -> u8 {
    let mut acc = 0;
    let mut digit = 0;
    for _ in 0..10 {
        acc += *rem;
        if acc >= d {
            acc -= d;
            digit += 1;
        }
    }
    *rem = acc;
    digit
}

impl Rounded {
    /// Rounds half up to `keep` significant digits. `9.99` becomes `10`.
    fn round(&mut self, keep: usize) {
        if self.digits.len() <= keep {
            return;
        }
        let up = self.digits[keep] >= 5;
        self.digits.truncate(keep);
        if up {
            for d in self.digits.iter_mut().rev() {
                if *d < 9 {
                    *d += 1;
                    return;
                }
                *d = 0;
            }
            self.digits.insert(0, 1);
            self.exp += 1;
        }
    }

    /// Drops trailing zeros, and puts zero's one digit in the units place.
    fn trim(&mut self) {
        while self.digits.len() > 1 && self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits == [0] {
            self.exp = 0;
        }
    }

    fn render(&self, group: bool) -> String {
        let mut ds: String = self.digits.iter().map(|&d| char::from(b'0' + d)).collect();
        let mut out = String::with_capacity(ds.len() + 8);
        if self.neg {
            out.push('-');
        }
        if !self.plain {
            let (first, rest) = ds.split_at(1);
            out.push_str(first);
            if !rest.is_empty() {
                out.push('.');
                out.push_str(rest);
            }
            out.push('e');
            out.push_str(&self.exp.to_string());
        } else if let Ok(exp) = usize::try_from(self.exp) {
            let int_len = exp + 1;
            while ds.len() < int_len {
                ds.push('0');
            }
            let (int, frac) = ds.split_at(int_len);
            if group {
                out.extend(grouped(int));
            } else {
                out.push_str(int);
            }
            if !frac.is_empty() {
                out.push('.');
                out.push_str(frac);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-self.exp - 1) as usize));
            out.push_str(&ds);
        }
        out
    }

    /// Whole numbers in title-case short scale, with `Minus` for negatives. None for fractions or
    /// past the length cap.
    fn words(&self) -> Option<String> {
        let int_len = usize::try_from(self.exp + 1).ok()?;
        if self.digits.len() > int_len {
            return None;
        }
        // A whole number is at most 2^127, so it fits in a u128.
        let scale = 10u128.pow((int_len - self.digits.len()) as u32);
        let n = self
            .digits
            .iter()
            .fold(0u128, |n, &d| n * 10 + u128::from(d))
            * scale;
        let words = spell(n);
        let words = if self.neg {
            format!("Minus {words}")
        } else {
            words
        };
        (words.len() <= WORDS_MAX).then_some(words)
    }
}

const ONES: [&str; 20] = [
    "Zero",
    "One",
    "Two",
    "Three",
    "Four",
    "Five",
    "Six",
    "Seven",
    "Eight",
    "Nine",
    "Ten",
    "Eleven",
    "Twelve",
    "Thirteen",
    "Fourteen",
    "Fifteen",
    "Sixteen",
    "Seventeen",
    "Eighteen",
    "Nineteen",
];
const TENS: [&str; 10] = [
    "", "", "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety",
];
/// Up to 10^36, so every `u128` spells.
const SCALES: [&str; 13] = [
    "",
    "Thousand",
    "Million",
    "Billion",
    "Trillion",
    "Quadrillion",
    "Quintillion",
    "Sextillion",
    "Septillion",
    "Octillion",
    "Nonillion",
    "Decillion",
    "Undecillion",
];

fn spell(mut n: u128) -> String {
    if n == 0 {
        return ONES[0].into();
    }
    let mut groups = Vec::new();
    for scale in SCALES {
        let g = (n % 1000) as usize;
        if g > 0 {
            let mut w = Vec::new();
            if g >= 100 {
                w.push(format!("{} Hundred", ONES[g / 100]));
            }
            match g % 100 {
                0 => {}
                r @ 1..20 => w.push(ONES[r].into()),
                r if r.is_multiple_of(10) => w.push(TENS[r / 10].into()),
                r => w.push(format!("{}-{}", TENS[r / 10], ONES[r % 10])),
            }
            if !scale.is_empty() {
                w.push(scale.into());
            }
            groups.push(w.join(" "));
        }
        n /= 1000;
    }
    groups.reverse();
    groups.join(" ")
}
