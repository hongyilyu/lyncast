//! The scorer: frizbee 0.13.0 with TinyCast's points as its weights, TinyCast's Medium
//! threshold, and exactness decided on folded text.

use frizbee::{CaseMatching, Config, Matcher, Scoring, SortStrategy};

use crate::fold;

/// How well a query fits one field. `Exact` beats every score, so the derived order puts the
/// best fit last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Fit {
    Score(u16),
    /// The field's lowercased fold equals the query's.
    Exact,
}

/// One field scored against a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scored {
    pub fit: Fit,
    /// The field alone makes its entry show: it is exact, or it passes Medium.
    pub shows: bool,
}

/// Scores one text against a query as the index scores a field. `None` when the query does
/// not align, or has nothing left to match once its separators are gone.
pub fn score(query: &str, text: &str) -> Option<Scored> {
    let query = Query::new(query)?;
    let field = fold(text);
    let end = field_end(field.kept());
    let found = query.matcher()?.match_one(&field.kept()[..end], 0)?;
    Some(query.classify(field.lower().trim(), found.score))
}

/// frizbee scores a match window over 1,024 bytes with a greedy match that can miss the best
/// alignment. The window lies inside one field, so frizbee scores a field's first 1,024 bytes.
/// Exactness and prefixes still compare the whole field.
const FIELD_BYTES: usize = 1024;

/// Where frizbee's copy of a folded field ends: at most [`FIELD_BYTES`], on a character
/// boundary.
pub(crate) fn field_end(kept: &str) -> usize {
    kept.floor_char_boundary(FIELD_BYTES)
}

/// TinyCast's separators. A query loses them before frizbee sees it.
const SEPARATORS: [char; 10] = [' ', '\t', '\n', '-', '.', '/', '(', ')', '[', ']'];

/// A typed query in the fold, built once per keystroke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Query {
    /// Folded, lowercased, and trimmed. Exactness, prefixes, and past terms compare it.
    text: Box<str>,
    /// `text`'s length in characters, for rule 8.
    chars: usize,
    /// `text` without separators. frizbee matches it.
    needle: Box<str>,
    len: usize,
}

impl Query {
    /// `None` for text that is empty or only whitespace once folded, which is the empty query.
    pub(crate) fn new(text: &str) -> Option<Query> {
        let folded = fold(text);
        let text = folded.lower().trim();
        if text.is_empty() {
            return None;
        }
        let needle: String = text.chars().filter(|c| !SEPARATORS.contains(c)).collect();
        Some(Query {
            chars: text.chars().count(),
            len: needle.chars().count(),
            text: text.into(),
            needle: needle.into(),
        })
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn chars(&self) -> usize {
        self.chars
    }

    /// L, the query's length without its separators. Medium and rule 3 read it.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// A matcher for the needle, or `None` when the query is only separators.
    pub(crate) fn matcher(&self) -> Option<Matcher> {
        (!self.needle.is_empty()).then(|| Matcher::new(&*self.needle, &config()))
    }

    /// Classifies one aligned field from its lowercased fold, trimmed as the query is, and
    /// frizbee's score on its case-kept fold. frizbee's own `exact` is case-sensitive, so
    /// exactness is decided here.
    pub(crate) fn classify(&self, lower: &str, score: u16) -> Scored {
        if lower == &*self.text {
            Scored {
                fit: Fit::Exact,
                shows: true,
            }
        } else {
            Scored {
                fit: Fit::Score(score),
                shows: passes(score, self.len),
            }
        }
    }
}

/// TinyCast's Medium sensitivity, `score ≥ 1.5·(L−2)+4`, in integers.
fn passes(score: u16, len: usize) -> bool {
    2 * u64::from(score) >= 3 * len as u64 + 2
}

/// TinyCast's points as frizbee weights: the first character on the text's first 4 (2 plus the
/// prefix bonus 2), a word start 3, any other 2, and −1 per gap. Case is ignored, because a
/// folded needle still holds a capital with no lowercase form, such as `ℝ`, and frizbee's
/// default would then match case-sensitively. `IndexAsc` returns matches in haystack order, so
/// the index reads each entry's matches as one run.
fn config() -> Config {
    Config::default()
        .max_typos(Some(0))
        .casing(CaseMatching::Ignore)
        .sort(SortStrategy::IndexAsc)
        .scoring(Scoring {
            match_score: 2,
            prefix_bonus: 2,
            delimiter_bonus: 1,
            capitalization_bonus: 1,
            gap_open_penalty: 1,
            gap_extend_penalty: 0,
            mismatch_penalty: 4,
            matching_case_bonus: 0,
            exact_match_bonus: 0,
        })
}
