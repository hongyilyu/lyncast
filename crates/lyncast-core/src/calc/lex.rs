//! The one tokenizer. Numbers, words, parentheses, and the operator table's spellings each
//! become one token, read once. A word the table does not spell stays a word, for the catalog.

use super::ops::{INFIX, Infix, Operator, PREFIX, Prefix};

#[derive(Clone, Copy, Debug)]
pub(super) enum Tok<'a> {
    /// ASCII digits with at most one point.
    Num(&'a str),
    /// A spelling from the operator table, with each row it can mean. `-` is both `SUB` and
    /// `NEG`, and the parser picks by position. `symbol` is false for a word such as `x`.
    Op {
        infix: Option<&'static Infix>,
        prefix: Option<&'static Prefix>,
        symbol: bool,
    },
    Word(&'a str),
    Open,
    Close,
    /// `0x`, `0b`, or `0o` and the letters and digits after it. Base conversion will read it,
    /// and the MVP has no reading for it.
    Radix,
}

/// The tokens of `text`, or `None` at a character that starts no token.
pub(super) fn tokens(text: &str) -> Option<Vec<Tok<'_>>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let (tok, len) = match c {
            c if c.is_whitespace() => {
                rest = &rest[c.len_utf8()..];
                continue;
            }
            '0'..='9' | '.' => number(rest)?,
            '(' => (Tok::Open, 1),
            ')' => (Tok::Close, 1),
            c => spelled(rest).or_else(|| c.is_alphabetic().then(|| word(rest)))?,
        };
        out.push(tok);
        rest = &rest[len..];
    }
    Some(out)
}

/// A number or a radix token at the start of `rest`. `1.2.3` is a version, not a number.
fn number(rest: &str) -> Option<(Tok<'_>, usize)> {
    let b = rest.as_bytes();
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    if b.starts_with(b"0")
        && matches!(b.get(1), Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O'))
        && b.get(2).is_some_and(u8::is_ascii_alphanumeric)
    {
        let len = 2 + b[2..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        return Some((Tok::Radix, len));
    }
    let int = digits(0);
    let mut len = int;
    let mut frac = 0;
    if b.get(len) == Some(&b'.') {
        frac = digits(len + 1);
        len += 1 + frac;
    }
    if int + frac == 0 || b.get(len) == Some(&b'.') {
        return None;
    }
    Some((Tok::Num(&rest[..len]), len))
}

fn word(rest: &str) -> (Tok<'_>, usize) {
    let len = word_len(rest);
    (Tok::Word(&rest[..len]), len)
}

fn word_len(s: &str) -> usize {
    s.find(|c: char| !c.is_alphabetic()).unwrap_or(s.len())
}

/// The longest operator spelling at the start of `rest`, as a token and its length in bytes.
fn spelled(rest: &str) -> Option<(Tok<'static>, usize)> {
    let infix = longest(&INFIX, rest);
    let prefix = longest(&PREFIX, rest);
    let len = infix.map_or(0, |m| m.len).max(prefix.map_or(0, |m| m.len));
    if len == 0 {
        return None;
    }
    let tok = Tok::Op {
        infix: infix.filter(|m| m.len == len).map(|m| m.op),
        prefix: prefix.filter(|m| m.len == len).map(|m| m.op),
        symbol: !starts_with_ascii_letter(&rest[..len]),
    };
    Some((tok, len))
}

#[derive(Clone, Copy)]
struct Spelled<F: 'static> {
    op: &'static Operator<F>,
    len: usize,
}

/// The row with the longest spelling at the start of `rest`.
fn longest<F>(rows: &[&'static Operator<F>], rest: &str) -> Option<Spelled<F>> {
    rows.iter()
        .flat_map(|&op| op.spellings.iter().map(move |spelling| (op, spelling)))
        .filter_map(|(op, spelling)| {
            Some(Spelled {
                op,
                len: spelling_len(rest, spelling)?,
            })
        })
        .max_by_key(|m| m.len)
}

/// Word spellings are ASCII, so one byte tells a word from a symbol without a Unicode lookup.
fn starts_with_ascii_letter(s: &str) -> bool {
    s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
}

/// How many bytes at the start of `rest` spell `spelling`. A word spelling matches a whole word
/// without case.
fn spelling_len(rest: &str, spelling: &str) -> Option<usize> {
    if !starts_with_ascii_letter(spelling) {
        return rest.starts_with(spelling).then_some(spelling.len());
    }
    if !starts_with_ascii_letter(rest) {
        return None;
    }
    let word = word_len(rest);
    rest[..word].eq_ignore_ascii_case(spelling).then_some(word)
}
