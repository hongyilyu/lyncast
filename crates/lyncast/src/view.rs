//! The JSON view that `lyncast query --json` prints: one compact object per row and line, with
//! the fields in declaration order. The daemon sends the same objects in its `rows` reply, and
//! the client prints them as they came. The wire spellings of kinds, sections, verbs, and chords
//! live here, and serde stays out of lyncast-core.

use std::io::{self, Write};

use lyncast_core::calc::Answer;
use lyncast_core::{Actions, Chord, Kind, Row, Section, Verb};
use serde::{Deserialize, Serialize};

/// One row as `query --json` prints it.
#[derive(Serialize, Deserialize)]
pub struct RowView {
    key: String,
    /// The row's position in the flat list, counted across sections.
    index: usize,
    kind: String,
    section: String,
    title: String,
    /// Only the calculator card's row has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<CardView>,
    actions: Vec<ActionView>,
}

#[derive(Serialize, Deserialize)]
struct CardView {
    expression: String,
    source_badge: String,
    display: String,
    copy: String,
    /// `null` when the answer has no words.
    target_badge: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ActionView {
    verb: String,
    label: String,
    /// Every chord that runs this item, in `Chord::ALL` order.
    chords: Vec<String>,
}

/// The view of each row, numbered in order.
pub fn rows(rows: &[Row]) -> Vec<RowView> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let key = row.subject.key();
            RowView {
                key: key.to_string(),
                index,
                kind: kind_name(key.kind()).to_owned(),
                section: section_name(row.section).to_owned(),
                title: row.title.clone(),
                card: row.card.as_ref().map(card_view),
                actions: action_views(&row.subject.actions()),
            }
        })
        .collect()
}

/// Writes one line per row.
pub fn write_rows(out: &mut impl Write, rows: &[RowView]) -> io::Result<()> {
    for row in rows {
        serde_json::to_writer(&mut *out, row)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

fn card_view(answer: &Answer) -> CardView {
    CardView {
        expression: answer.expression().to_owned(),
        source_badge: answer.source_badge().to_owned(),
        display: answer.display().to_owned(),
        copy: answer.copy().to_owned(),
        target_badge: answer.target_badge().map(str::to_owned),
    }
}

/// The Actions menu in order, default first.
fn action_views(actions: &Actions) -> Vec<ActionView> {
    actions
        .iter()
        .map(|(_, item)| ActionView {
            verb: verb_name(&item.verb()),
            label: item.label().to_owned(),
            chords: Chord::ALL
                .into_iter()
                .filter(|&chord| actions.on(chord) == item)
                .map(|chord| chord_name(chord).to_owned())
                .collect(),
        })
        .collect()
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::App => "app",
        Kind::Command => "command",
        Kind::Clip => "clip",
        Kind::Calculator => "calculator",
    }
}

fn section_name(section: Section) -> &'static str {
    match section {
        Section::Answer => "answer",
        Section::Results => "results",
        Section::Suggestions => "suggestions",
        Section::Commands => "commands",
    }
}

fn verb_name(verb: &Verb) -> String {
    match verb {
        Verb::Launch => "launch".to_owned(),
        Verb::LaunchAction(id) => format!("launch-action:{id}"),
        Verb::CopyPath => "copy-path".to_owned(),
        Verb::Copy => "copy".to_owned(),
        Verb::Pin => "pin".to_owned(),
        Verb::Unpin => "unpin".to_owned(),
        Verb::Delete => "delete".to_owned(),
        Verb::Run => "run".to_owned(),
    }
}

fn chord_name(chord: Chord) -> &'static str {
    match chord {
        Chord::Enter => "enter",
        Chord::CtrlEnter => "ctrl-enter",
    }
}
