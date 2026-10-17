//! The JSON views that `lyncast query --json` and `lyncast clip list --json` print: one compact
//! object per row and line, with the fields in declaration order. The daemon sends the same
//! objects in its `rows` and `clips` replies, and the client prints them as they came. The wire
//! spellings of keys, kinds, sections, verbs, and chords live here, and serde stays out of
//! lyncast-core.

use std::io::{self, Write};

use lyncast_core::calc::Answer;
use lyncast_core::{Actions, Chord, Key, KeyError, Kind, Row, Section, Verb};
use serde::{Deserialize, Serialize};

use crate::clip;

/// A value that the CLI and the wire spell as `query --json` prints it.
pub trait Spelled: Sized {
    fn spelling(&self) -> String;
    /// The value `text` spells, or why it spells none.
    fn read(text: &str) -> Result<Self, String>;
}

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

/// One clip as `clip list --json` prints it.
#[derive(Serialize, Deserialize)]
pub struct ClipView {
    id: i64,
    kind: String,
    pinned: bool,
    mime: String,
    bytes: u64,
    /// `null` for an image.
    preview: Option<String>,
}

/// The view of each clip, in the order given.
pub fn clips(rows: &[&clip::Row]) -> Vec<ClipView> {
    rows.iter()
        .map(|row| ClipView {
            id: row.id.0,
            kind: row.content.spelling().to_owned(),
            pinned: row.pinned,
            mime: row.mime.clone(),
            bytes: row.bytes,
            preview: row.preview.clone(),
        })
        .collect()
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

/// Writes one line per row or clip.
pub fn write_lines<T: Serialize>(out: &mut impl Write, rows: &[T]) -> io::Result<()> {
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
            verb: item.verb().spelling(),
            label: item.label().to_owned(),
            chords: Chord::ALL
                .into_iter()
                .filter(|&chord| actions.on(chord) == item)
                .map(|chord| chord.spelling())
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

impl Spelled for Key {
    fn spelling(&self) -> String {
        self.to_string()
    }

    fn read(text: &str) -> Result<Key, String> {
        text.parse().map_err(|error: KeyError| error.to_string())
    }
}

impl Spelled for Verb {
    fn spelling(&self) -> String {
        match self {
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

    fn read(text: &str) -> Result<Verb, String> {
        if let Some(id) = text
            .strip_prefix("launch-action:")
            .filter(|id| !id.is_empty())
        {
            return Ok(Verb::LaunchAction(id.to_owned()));
        }
        // Every verb without an argument.
        let verbs = [
            Verb::Launch,
            Verb::CopyPath,
            Verb::Copy,
            Verb::Pin,
            Verb::Unpin,
            Verb::Delete,
            Verb::Run,
        ];
        verbs
            .into_iter()
            .find(|verb| verb.spelling() == text)
            .ok_or_else(|| "unknown verb".to_owned())
    }
}

impl Spelled for Chord {
    fn spelling(&self) -> String {
        match self {
            Chord::Enter => "enter",
            Chord::CtrlEnter => "ctrl-enter",
        }
        .to_owned()
    }

    fn read(text: &str) -> Result<Chord, String> {
        Chord::ALL
            .into_iter()
            .find(|chord| chord.spelling() == text)
            .ok_or_else(|| "unknown chord".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use lyncast_core::{AppId, ClipId, Command, DesktopAction, Subject};

    use super::*;

    /// `--chord`, `--verb`, and the wire read back every spelling that `query --json` prints,
    /// for every item of a row of each kind. docs/design/daemon.md holds the spellings they
    /// refuse.
    #[test]
    fn each_chord_and_verb_reads_back_from_its_spelling() {
        // A new kind fails to compile here until it has subjects to check.
        let subjects: Vec<Subject> = Kind::ALL
            .into_iter()
            .flat_map(|kind| match kind {
                Kind::App => vec![Subject::App {
                    id: AppId("firefox.desktop".to_owned()),
                    actions: vec![DesktopAction {
                        id: "new-window".to_owned(),
                        name: "New Window".to_owned(),
                    }],
                }],
                Kind::Command => Vec::from(Command::ALL.map(Subject::Command)),
                Kind::Clip => Vec::from([false, true].map(|pinned| Subject::Clip {
                    id: ClipId(1),
                    pinned,
                })),
                Kind::Calculator => vec![Subject::Calc {
                    query: "1+1".to_owned(),
                }],
            })
            .collect();
        for subject in &subjects {
            for (_, item) in subject.actions().iter() {
                let verb = item.verb();
                assert_eq!(Verb::read(&verb.spelling()), Ok(verb), "{item:?}");
            }
        }
        for chord in Chord::ALL {
            assert_eq!(Chord::read(&chord.spelling()), Ok(chord));
        }
    }
}
