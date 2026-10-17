//! The root list. Every root feature evaluates the query, and the rows merge in [`Section`]
//! order, so the calculator's card comes before ranking's rows.

use crate::calc::{self, Answer, Outcome};
use crate::rank::{Index, Timestamp, Visits};
use crate::{Row, Section, Subject};

/// The calculator's answer to `query`, when the query is math.
pub fn answer(query: &str) -> Option<Answer> {
    match calc::evaluate(query, &calc::Env::default()) {
        Outcome::Card(answer) => Some(answer),
        // The MVP never returns an error card, and a Silent query stays a search.
        Outcome::Error(_) | Outcome::Silent => None,
    }
}

/// The root list for `query` at `now`. When the query is math, the calculator's card is the
/// one Answer row, at index 0, keyed by the raw query. Ranking's rows follow.
pub fn rows(index: &Index, visits: &Visits, query: &str, now: Timestamp) -> Vec<Row> {
    let card = answer(query).map(|answer| Row {
        subject: Subject::Calc {
            query: query.to_owned(),
        },
        section: Section::Answer,
        title: answer.display().to_owned(),
        card: Some(answer),
    });
    let mut rows: Vec<Row> = card
        .into_iter()
        .chain(index.rows(query, visits, now))
        .collect();
    rows.sort_by_key(|row| row.section);
    rows
}
