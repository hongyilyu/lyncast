//! The calculator: one pure function from root query text to an outcome. The steps, the rules,
//! and the fixtures are in docs/design/calculator.md, and `tests/calc.rs` runs those fixtures.

mod catalog;
mod format;
mod lex;
mod ops;
mod parse;
mod quantity;

use crate::clip::Delivery;
use crate::fold::narrow;

/// Everything an answer depends on besides the query text. Currency adds the rate snapshot.
#[derive(Clone, Debug, Default)]
pub struct Env {}

/// What the calculator makes of a root query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A card at index 0.
    Card(Answer),
    /// An error card. The MVP never returns one.
    Error(String),
    /// No card, so the query stays a search.
    Silent,
}

/// One calculator card. Only `evaluate` builds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    expression: String,
    source_badge: &'static str,
    display: String,
    copy: String,
    target_badge: Option<String>,
}

impl Answer {
    /// The query as the engine read it, such as `12 × 34`.
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// What the answer is, named by the root operator, such as `Product`.
    pub fn source_badge(&self) -> &str {
        self.source_badge
    }

    /// The answer rounded once, with grouped digits, such as `1,000,000`.
    pub fn display(&self) -> &str {
        &self.display
    }

    /// What Enter copies: the display's digits without grouping.
    pub fn copy(&self) -> &str {
        &self.copy
    }

    /// The answer in words, such as `Four Hundred Eight`, when it is a whole number that spells
    /// in 40 characters or fewer.
    pub fn target_badge(&self) -> Option<&str> {
        self.target_badge.as_deref()
    }

    /// What a copy of the card hands over: the answer as [`Answer::copy`] spells it.
    pub fn delivery(&self) -> Delivery {
        Delivery::Answer(CalcAnswer(self.copy.clone()))
    }
}

/// The text a copy of a card hands over. Only the calculator builds one, from its answer, so a
/// delivery of an answer always holds what the card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalcAnswer(String);

impl CalcAnswer {
    pub fn text(&self) -> &str {
        &self.0
    }
}

/// Longer text is a paste, not a sum, and the cap bounds the work per keystroke.
const MAX_BYTES: usize = 1024;

/// The calculator's answer to a root query. In the MVP every failure is silent.
pub fn evaluate(text: &str, env: &Env) -> Outcome {
    answer(text, env).map_or(Outcome::Silent, Outcome::Card)
}

fn answer(text: &str, env: &Env) -> Option<Answer> {
    let text = text.trim();
    // Narrowing shrinks text by at most 3 to 1, so longer raw text would be over the cap anyway.
    if text.len() > 3 * MAX_BYTES {
        return None;
    }
    let text = narrow(text);
    if text.len() > MAX_BYTES || is_search(&text) {
        return None;
    }
    let mut tokens = lex::tokens(&text)?;
    let stripped = parse::strip_trailing_operator(&mut tokens);
    let expr = parse::parse(&tokens)?;
    if expr.is_bare_number() && !stripped {
        return None;
    }
    let shown = format::show(expr.eval(env).ok()?.into_scalar()?);
    Some(Answer {
        expression: expr.to_string(),
        source_badge: expr.badge(),
        display: shown.display,
        copy: shown.copy,
        target_badge: shown.words,
    })
}

/// Empty text and one ASCII word are searches, as in TinyCast. A date shape is a date the MVP
/// cannot read yet.
fn is_search(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_alphabetic()) || has_date(text)
}

/// Whether `text` holds `dddd-dd-dd` or `dd/dd/dddd` anywhere. A minus sign counts as a hyphen,
/// because `−` is a spelling of subtraction.
fn has_date(text: &str) -> bool {
    const SHAPES: [&str; 2] = ["dddd-dd-dd", "dd/dd/dddd"];
    let fits = |c: char, shape: char| match shape {
        'd' => c.is_ascii_digit(),
        '-' => c == '-' || c == '−',
        _ => c == shape,
    };
    text.char_indices().any(|(i, _)| {
        SHAPES.iter().any(|shape| {
            let mut rest = text[i..].chars();
            shape
                .chars()
                .all(|s| rest.next().is_some_and(|c| fits(c, s)))
        })
    })
}
