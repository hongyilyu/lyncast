//! TinyCast's comparator as one sort key per shown entry. The derived `Ord` compares fields in
//! declaration order, so the first field that differs is the first rule that separates two
//! entries. A key that should win is the smaller value (`Meets::Yes`, `Reverse(score)`), so an
//! ascending sort puts the best first.

use std::cmp::{Ordering, Reverse};

use super::Field;
use super::frecency::{Frecency, Learned};
use super::score::{Fit, Query};

/// An entry's position in the index. The index stores entries in [`Name`] order, so the
/// position is also the entry's rule-14 rank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Alpha(pub(crate) u32);

/// Rule 13, kind priority: apps before commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum KindRank {
    App,
    Command,
}

/// A yes-or-no rule. `Yes` sorts first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Meets {
    Yes,
    No,
}

fn meets(yes: bool) -> Meets {
    if yes { Meets::Yes } else { Meets::No }
}

/// The strongest past search term, which orders two entries that both meet rule 3: a term
/// equal to the query, then one that starts with it, then one the query runs past.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Strength {
    Exact,
    Prefix,
    Run,
    None,
}

/// The order of one shown entry for one query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RankKey {
    standing: Standing,
    tiebreak: Tiebreak,
}

/// The first of rules 3, 4, and 5 an entry meets, holding the sub-order TinyCast gives two
/// entries that both meet it, or rules 7 to 12 for an entry that meets none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Standing {
    /// Rule 3: the title or alternate title equals the query, and L is more than 3.
    ExactTitle {
        term: Strength,
        frecency: Reverse<Frecency>,
    },
    /// Rule 4: a past search term equals the query.
    ExactTerm { frecency: Reverse<Frecency> },
    /// Rule 5: the subtitle equals the query.
    ExactSubtitle {
        frecency: Reverse<Frecency>,
        title: Reverse<Option<Fit>>,
    },
    Fuzzy {
        /// Rule 7: a past search term starts with the query.
        past_prefix: Meets,
        /// Rule 8: the query starts with a past term of 3 or more characters and runs 1 to 3
        /// characters past it.
        past_run: Meets,
        /// Rule 9: the best fit over the title, alternate title, and subtitle.
        best: Reverse<Option<Fit>>,
        /// Rule 10.
        frecency: Reverse<Frecency>,
        /// Rule 11: the title's own fit.
        title: Reverse<Option<Fit>>,
        /// Rule 12: the title or alternate title starts with the query.
        title_prefix: Meets,
    },
}

/// What the standing leaves equal, and the empty list's whole order: frecency, then kind
/// priority (rule 13), then the name (rule 14). Its frecency never separates two keys of a
/// typed query, because every standing already holds the entry's frecency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Tiebreak {
    frecency: Reverse<Frecency>,
    kind: KindRank,
    alpha: Alpha,
}

impl Tiebreak {
    pub(crate) fn new(frecency: Frecency, kind: KindRank, alpha: Alpha) -> Tiebreak {
        Tiebreak {
            frecency: Reverse(frecency),
            kind,
            alpha,
        }
    }

    pub(crate) fn alpha(&self) -> Alpha {
        self.alpha
    }
}

impl RankKey {
    /// The key for one shown entry from what the query found in its fields and what was
    /// learned about it.
    pub(crate) fn new(
        hits: &Hits,
        learned: Learned<'_>,
        query: &Query,
        tiebreak: Tiebreak,
    ) -> RankKey {
        let terms = TermHits::of(learned.terms, query);
        let frecency = Reverse(learned.frecency);
        let standing = if hits.exact_title && query.len() > 3 {
            Standing::ExactTitle {
                term: terms.strength(),
                frecency,
            }
        } else if terms.exact {
            Standing::ExactTerm { frecency }
        } else if hits.exact_subtitle {
            Standing::ExactSubtitle {
                frecency,
                title: Reverse(hits.title),
            }
        } else {
            Standing::Fuzzy {
                past_prefix: meets(terms.prefix),
                past_run: meets(terms.run),
                best: Reverse(hits.best),
                frecency,
                title: Reverse(hits.title),
                title_prefix: meets(hits.title_prefix),
            }
        };
        RankKey { standing, tiebreak }
    }

    pub(crate) fn alpha(&self) -> Alpha {
        self.tiebreak.alpha
    }
}

/// What one query found in one entry's aligned fields. A field that equals or starts with the
/// query aligns, so the fields frizbee did not report hold nothing these rules read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hits {
    /// Some field, keywords included, is exact or passes Medium.
    pub(crate) shows: bool,
    title: Option<Fit>,
    best: Option<Fit>,
    exact_title: bool,
    exact_subtitle: bool,
    title_prefix: bool,
}

impl Hits {
    /// Adds one aligned field from its lowercased fold and frizbee's score. Rules 9 and 11 read
    /// a field's score whether or not that field passes Medium on its own.
    pub(crate) fn add(&mut self, field: Field, lower: &str, score: u16, query: &Query) {
        let scored = query.classify(lower, score);
        self.shows |= scored.shows;
        let exact = scored.fit == Fit::Exact;
        match field {
            Field::Title | Field::AltTitle => {
                if field == Field::Title {
                    self.title = Some(scored.fit);
                }
                self.best = self.best.max(Some(scored.fit));
                self.exact_title |= exact;
                self.title_prefix |= lower.starts_with(query.text());
            }
            Field::Subtitle => {
                self.best = self.best.max(Some(scored.fit));
                self.exact_subtitle |= exact;
            }
            Field::Keyword => {}
        }
    }
}

/// Which past-term rules an entry's steering terms meet for one query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TermHits {
    exact: bool,
    prefix: bool,
    run: bool,
}

impl TermHits {
    fn of(terms: &[Box<str>], query: &Query) -> TermHits {
        let text = query.text();
        let mut hits = TermHits::default();
        for term in terms {
            hits.exact |= &**term == text;
            hits.prefix |= term.starts_with(text);
            let n = term.chars().count();
            hits.run |=
                n >= 3 && text.starts_with(&**term) && (1..=3).contains(&(query.chars() - n));
        }
        hits
    }

    fn strength(self) -> Strength {
        if self.exact {
            Strength::Exact
        } else if self.prefix {
            Strength::Prefix
        } else if self.run {
            Strength::Run
        } else {
            Strength::None
        }
    }
}

/// The alphabetical order, computed once when the index is built: the lowercased folded titles
/// with ASCII digit runs compared by value, then the raw titles, then the keys, so no two
/// entries tie.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Name {
    pub(crate) folded: String,
    pub(crate) raw: String,
    pub(crate) key: String,
}

impl Ord for Name {
    fn cmp(&self, other: &Self) -> Ordering {
        numeric(&self.folded, &other.folded)
            .then_with(|| self.raw.cmp(&other.raw))
            .then_with(|| self.key.cmp(&other.key))
    }
}

impl PartialOrd for Name {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Compares by characters, except that two ASCII digit runs at the same place compare by
/// value, so `app 9` comes before `app 10`. Runs of equal value, such as `7` and `007`,
/// compare equal.
fn numeric(mut a: &str, mut b: &str) -> Ordering {
    loop {
        let (Some(x), Some(y)) = (a.chars().next(), b.chars().next()) else {
            return (!a.is_empty()).cmp(&!b.is_empty());
        };
        if x.is_ascii_digit() && y.is_ascii_digit() {
            let (run_a, rest_a) = digits(a);
            let (run_b, rest_b) = digits(b);
            let (run_a, run_b) = (run_a.trim_start_matches('0'), run_b.trim_start_matches('0'));
            let order = run_a.len().cmp(&run_b.len()).then_with(|| run_a.cmp(run_b));
            if order.is_ne() {
                return order;
            }
            (a, b) = (rest_a, rest_b);
        } else if x != y {
            return x.cmp(&y);
        } else {
            (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
        }
    }
}

/// Splits off the leading run of ASCII digits.
fn digits(text: &str) -> (&str, &str) {
    text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    )
}
