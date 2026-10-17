//! Learned ranking: a decaying frecency per entry and its last three distinct search terms.
//! The clock is always an argument. Step 5's daemon owns one `Visits` and persists it.

use std::collections::HashMap;
use std::f64::consts::LN_2;

use super::score::Query;
use crate::Key;

/// Wall-clock milliseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub i64);

const DAY: i64 = 86_400_000;

/// λ = ln 2 per 10 days, per millisecond.
const LAMBDA: f64 = LN_2 / (10 * DAY) as f64;

/// Past terms steer only for an entry opened at most this long ago.
const STEER_FOR: i64 = 17 * DAY;

/// How many distinct search terms a visit keeps.
const TERMS: usize = 3;

/// What the launcher learned about one entry from past picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    /// `score(t) = max(1, e^(λ·(anchor − t)))`. A later anchor is always a higher score, so
    /// ranking compares anchors and nothing changes as time passes.
    anchor: Timestamp,
    last_opened: Timestamp,
    terms: Vec<Box<str>>,
}

impl Visit {
    /// The frecency score at `now`, at least 1.
    pub fn score(&self, now: Timestamp) -> f64 {
        let lead = (self.anchor.0 - now.0) as f64;
        (LAMBDA * lead).exp().max(1.0)
    }

    /// The last distinct search terms in the query's fold, most recent first.
    pub fn terms(&self) -> &[Box<str>] {
        &self.terms
    }

    /// What ranking reads at `now`. Terms steer only while the score is above 1 and the entry
    /// was opened in the last 17 days.
    pub(crate) fn learned(&self, now: Timestamp) -> Learned<'_> {
        let warm = self.anchor > now;
        let recent = now.0 - self.last_opened.0 <= STEER_FOR;
        Learned {
            frecency: Frecency(warm.then_some(self.anchor)),
            terms: if warm && recent { &self.terms } else { &[] },
        }
    }

    /// One pick: the score rises by 100, and a non-empty query moves to the front of the terms.
    fn visit(&mut self, query: &str, now: Timestamp) {
        let lead = (self.score(now) + 100.0).ln() / LAMBDA;
        self.anchor = Timestamp(now.0 + lead.round() as i64);
        self.last_opened = now;
        if let Some(query) = Query::new(query) {
            let term = query.text();
            self.terms.retain(|seen| &**seen != term);
            self.terms.insert(0, term.into());
            self.terms.truncate(TERMS);
        }
    }
}

/// Every entry's visit, by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visits(HashMap<Key, Visit>);

impl Visits {
    /// Records a pick of `key` after typing `query`. A first visit starts from score 1, and the
    /// same pick twice raises the score twice.
    pub fn record(&mut self, key: Key, query: &str, now: Timestamp) {
        self.0
            .entry(key)
            .or_insert_with(|| Visit {
                anchor: now,
                last_opened: now,
                terms: Vec::new(),
            })
            .visit(query, now);
    }

    pub fn get(&self, key: &Key) -> Option<&Visit> {
        self.0.get(key)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&Key, &Visit)> {
        self.0.iter()
    }
}

/// Compares as the frecency score at the instant it was read. `None` scores 1, and a later
/// anchor scores higher, so no exponential is computed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Frecency(Option<Timestamp>);

impl Frecency {
    /// The score is above 1.
    pub(crate) fn is_warm(self) -> bool {
        self.0.is_some()
    }
}

/// One entry's learned ranking for one query.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Learned<'v> {
    pub(crate) frecency: Frecency,
    /// Empty unless the terms steer now.
    pub(crate) terms: &'v [Box<str>],
}
