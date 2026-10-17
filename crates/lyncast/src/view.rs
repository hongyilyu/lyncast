//! The JSON view that `lyncast query --json` prints: one compact object per row and line, with
//! the fields in declaration order. The wire spellings of kinds, sections, verbs, and chords
//! live here, and serde stays out of lyncast-core.

use std::io::{self, Write};

use lyncast_core::{Actions, Chord, Kind, Row, Section, Verb};
use serde::Serialize;

#[derive(Serialize)]
struct RowView<'a> {
    key: String,
    /// The row's position in the flat list, counted across sections.
    index: usize,
    kind: &'static str,
    section: &'static str,
    title: &'a str,
    actions: Vec<ActionView<'a>>,
}

#[derive(Serialize)]
struct ActionView<'a> {
    verb: String,
    label: &'a str,
    /// Every chord that runs this item, in `Chord::ALL` order.
    chords: Vec<&'static str>,
}

/// Writes one line per row.
pub fn write_rows(out: &mut impl Write, rows: &[Row]) -> io::Result<()> {
    for (index, row) in rows.iter().enumerate() {
        let key = row.subject.key();
        let actions = row.subject.actions();
        let view = RowView {
            key: key.to_string(),
            index,
            kind: kind_name(key.kind()),
            section: section_name(row.section),
            title: &row.title,
            actions: action_views(&actions),
        };
        serde_json::to_writer(&mut *out, &view)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// The Actions menu in order, default first.
fn action_views(actions: &Actions) -> Vec<ActionView<'_>> {
    actions
        .iter()
        .map(|(_, item)| ActionView {
            verb: verb_name(&item.verb()),
            label: item.label(),
            chords: Chord::ALL
                .into_iter()
                .filter(|&chord| actions.on(chord) == item)
                .map(chord_name)
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
