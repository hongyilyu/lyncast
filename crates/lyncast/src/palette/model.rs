#[cfg(test)]
use std::fmt::Write;
use std::mem;
use std::ops::Range;

use lyncast_core::{Chord, Key, Kind, Row, Section, calc};

use super::field::Field;
use crate::daemon::{After, Answer};
use crate::wire::{Request, Via};

/// What the empty field shows.
pub const PLACEHOLDER: &str = "Search for apps and commands";
/// What an empty list shows.
pub const NO_RESULTS: &str = "No results";

/// The key whose release closes the window after a success.
const ACTIVATING: &str = "enter";

pub struct Model {
    field: Field,
    rows: Vec<Row>,
    lines: Vec<Line>,
    selected_row: usize,
    overlay: Option<Overlay>,
    phase: Phase,
    was_active: bool,
}

enum Overlay {
    Confirm {
        key: Key,
        via: Via,
        title: &'static str,
    },
}

enum Phase {
    Idle,
    Pending { key: Key, via: Via, released: bool },
    Closing { released: bool },
    Failed(String),
}

pub enum Footer<'m> {
    /// The selected row's default action, which Enter runs.
    Hint(String),
    /// A command that asks first: Enter runs it, and Escape cancels.
    Confirm(&'static str),
    Pending,
    Closing,
    Failed(&'m str),
}

/// A key that the footer names, as the view draws its cap.
#[derive(Clone, Copy)]
pub enum Cap {
    Enter,
    Escape,
}

impl Cap {
    pub fn label(self) -> &'static str {
        match self {
            Cap::Enter => "↵",
            Cap::Escape => "esc",
        }
    }

    /// How a `palette` block spells the key.
    #[cfg(test)]
    pub fn spelling(self) -> &'static str {
        match self {
            Cap::Enter => "enter",
            Cap::Escape => "escape",
        }
    }
}

impl Footer<'_> {
    /// What the footer names, in order: a label and the key that runs it. The first is what
    /// Enter runs.
    pub fn hints(&self) -> Vec<(&str, Cap)> {
        match self {
            Footer::Hint(label) => vec![(label, Cap::Enter)],
            Footer::Confirm(title) => vec![(title, Cap::Enter), ("Cancel", Cap::Escape)],
            Footer::Pending | Footer::Closing | Footer::Failed(_) => Vec::new(),
        }
    }
}

/// The question that a command that asks first puts in the footer.
pub fn question(title: &str) -> String {
    format!("{title}?")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    /// The header of a section that has one.
    Header(&'static str),
    /// The row at this index in [`Model::row`].
    Row(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// The first press of Enter or Ctrl+Enter.
    Activate(Chord),
    /// A held key's repeat of Enter or Escape, which the palette takes and drops.
    Ignore,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Backspace,
    Escape,
}

pub enum Input {
    Press(Press),
    /// Typed text or an input method's commit, written over `utf16`, else over the composing
    /// text, else at the caret.
    Replace {
        utf16: Option<Range<usize>>,
        text: String,
    },
    /// Text an input method is composing.
    Mark {
        utf16: Option<Range<usize>>,
        text: String,
        selected: Option<Range<usize>>,
    },
    /// The input method keeps its composing text as typed.
    Unmark,
}

/// What the daemon answered.
pub enum Landed {
    /// The root list for the field's text.
    Rows(Vec<Row>),
    /// The answer to the activation that is out.
    Answer(Answer),
}

/// The one thing the view does next.
pub enum Effect {
    /// Ask the daemon for the root list of this text.
    Query(String),
    /// Send the daemon this activation.
    Ask(Request),
    Close,
}

impl Model {
    /// The model of a summon that has just opened, and the root list it asks for.
    pub fn new() -> (Model, Effect) {
        let model = Model {
            field: Field::default(),
            rows: Vec::new(),
            lines: Vec::new(),
            selected_row: 0,
            overlay: None,
            phase: Phase::Idle,
            was_active: false,
        };
        (model, Effect::Query(String::new()))
    }

    /// A key press, typed text, or an input method's text. While an activation is out or the
    /// window is closing, only Escape does something: it closes the window, and the answer
    /// goes nowhere.
    pub fn input(&mut self, input: Input) -> Option<Effect> {
        if let Phase::Pending { .. } | Phase::Closing { .. } = self.phase {
            return matches!(input, Input::Press(Press::Escape)).then_some(Effect::Close);
        }
        let edited = match input {
            Input::Press(Press::Activate(chord)) => return self.activate(Via::Chord(chord)),
            Input::Press(Press::Ignore) => return None,
            Input::Press(Press::Escape) => return self.escape(),
            Input::Unmark => {
                self.field.unmark();
                return None;
            }
            input => {
                // Any other key or text ends a failure's footer and a confirmation it does
                // not answer.
                self.phase = Phase::Idle;
                self.overlay = None;
                self.edit(input)
            }
        };
        edited.then(|| self.query())
    }

    /// A key's release, by its name. `mods` says whether a modifier is held.
    pub fn released(&mut self, key: &str, mods: bool) -> Option<Effect> {
        if key != ACTIVATING {
            return None;
        }
        match &mut self.phase {
            Phase::Pending { released, .. } => {
                *released = true;
                None
            }
            Phase::Closing { released } => {
                *released = true;
                (!mods).then_some(Effect::Close)
            }
            // A release whose press the palette never saw, such as the one niri's bind leaves
            // as the window maps.
            Phase::Idle | Phase::Failed(_) => None,
        }
    }

    /// The held modifiers changed, and `mods` says whether one is still held.
    pub fn modifiers(&self, mods: bool) -> Option<Effect> {
        let up = matches!(self.phase, Phase::Closing { released: true });
        (up && !mods).then_some(Effect::Close)
    }

    /// The window turned active or inactive. Only a window that has been active closes when
    /// it turns inactive.
    pub fn active(&mut self, active: bool) -> Option<Effect> {
        let lost = self.was_active && !active;
        self.was_active |= active;
        lost.then_some(Effect::Close)
    }

    /// `mods` says whether a modifier is held as the answer lands.
    pub fn land(&mut self, landed: Landed, mods: bool) -> Option<Effect> {
        match landed {
            Landed::Rows(rows) => {
                self.lines = lines(&rows);
                self.rows = rows;
                self.selected_row = 0;
                None
            }
            Landed::Answer(answer) => self.answered(answer, mods),
        }
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The placeholder shows in an empty field while no input method composes.
    pub fn placeholder(&self) -> bool {
        self.field.is_empty() && !self.field.composing()
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn row(&self, index: usize) -> &Row {
        &self.rows[index]
    }

    pub fn is_selected(&self, index: usize) -> bool {
        index == self.selected_row
    }

    /// The selected row's line, if the list has a row.
    pub fn selected_line(&self) -> Option<usize> {
        self.lines
            .iter()
            .position(|&line| line == Line::Row(self.selected_row))
    }

    pub fn footer(&self) -> Option<Footer<'_>> {
        Some(match &self.phase {
            Phase::Pending { .. } => Footer::Pending,
            Phase::Closing { .. } => Footer::Closing,
            Phase::Failed(reason) => Footer::Failed(reason),
            Phase::Idle => match &self.overlay {
                Some(Overlay::Confirm { title, .. }) => Footer::Confirm(title),
                None => {
                    let row = self.rows.get(self.selected_row)?;
                    Footer::Hint(row.subject.actions().on(Chord::Enter).label().to_owned())
                }
            },
        })
    }

    /// The text of palette.md's `palette` blocks: the field, the lines with the selected row
    /// marked, and the footer.
    #[cfg(test)]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        if self.placeholder() {
            let _ = writeln!(out, "root ({PLACEHOLDER})");
        } else {
            let _ = writeln!(out, "root {:?}", self.field.text());
        }
        for line in &self.lines {
            let _ = match *line {
                Line::Header(name) => writeln!(out, "== {name}"),
                Line::Row(index) => {
                    let mark = if self.is_selected(index) { ">" } else { " " };
                    writeln!(out, "{mark} {}", row_text(&self.rows[index]))
                }
            };
        }
        if self.rows.is_empty() {
            let _ = writeln!(out, "{NO_RESULTS}");
        }
        if let Some(Overlay::Confirm { title, .. }) = &self.overlay {
            let _ = writeln!(out, "confirm {}", question(title));
        }
        let _ = match self.footer() {
            Some(Footer::Pending) => writeln!(out, "footer: pending"),
            Some(Footer::Closing) => writeln!(out, "footer: closing"),
            Some(Footer::Failed(reason)) => writeln!(out, "footer: failed: {reason}"),
            Some(footer) => {
                let hints: Vec<String> = (footer.hints().iter())
                    .map(|(label, cap)| format!("{label} ({})", cap.spelling()))
                    .collect();
                writeln!(out, "footer: {}", hints.join(" · "))
            }
            None => Ok(()),
        };
        out
    }

    /// The root list's request for the field's text.
    fn query(&self) -> Effect {
        Effect::Query(self.field.text().to_owned())
    }

    fn edit(&mut self, input: Input) -> bool {
        match input {
            Input::Press(Press::Up) => {
                self.selected_row = self.selected_row.saturating_sub(1);
                false
            }
            Input::Press(Press::Down) => {
                self.selected_row = (self.selected_row + 1).min(self.rows.len().saturating_sub(1));
                false
            }
            Input::Press(Press::Backspace) => self.field.backspace(),
            Input::Press(Press::Delete) => self.field.delete(),
            Input::Press(Press::Left) => {
                self.field.left();
                false
            }
            Input::Press(Press::Right) => {
                self.field.right();
                false
            }
            Input::Press(Press::Home) => {
                self.field.home();
                false
            }
            Input::Press(Press::End) => {
                self.field.end();
                false
            }
            Input::Replace { utf16, text } => self.field.replace(utf16, &text),
            Input::Mark {
                utf16,
                text,
                selected,
            } => self.field.mark(utf16, &text, selected),
            // `input` takes these itself.
            Input::Unmark | Input::Press(Press::Activate(_) | Press::Ignore | Press::Escape) => {
                false
            }
        }
    }

    fn activate(&mut self, via: Via) -> Option<Effect> {
        let (key, via, confirmed) = match self.overlay.take() {
            Some(Overlay::Confirm { key, via, .. }) => (key, via, true),
            None => (self.rows.get(self.selected_row)?.subject.key(), via, false),
        };
        self.phase = Phase::Pending {
            key: key.clone(),
            via: via.clone(),
            released: false,
        };
        let query = self.field.text().to_owned();
        Some(Effect::Ask(Request::Activate {
            key,
            via,
            confirmed,
            query,
        }))
    }

    fn answered(&mut self, answer: Answer, mods: bool) -> Option<Effect> {
        let Phase::Pending { key, via, released } = mem::replace(&mut self.phase, Phase::Idle)
        else {
            return None;
        };
        match answer {
            Answer::Done {
                after: After::Close,
                ..
            } if released && !mods => return Some(Effect::Close),
            Answer::Done {
                after: After::Close,
                ..
            } => self.phase = Phase::Closing { released },
            // The window has no clipboard screen to show.
            Answer::Done {
                after: After::ShowClipboard,
                ..
            } => {}
            Answer::Confirm(command) => {
                let title = command.title();
                self.overlay = Some(Overlay::Confirm { key, via, title });
            }
            Answer::Failed(reason) => self.phase = Phase::Failed(reason),
            Answer::Summon(_) | Answer::Rows(_) | Answer::Clips(_) => {
                self.phase = Phase::Failed("unexpected answer".to_owned());
            }
        }
        None
    }

    fn escape(&mut self) -> Option<Effect> {
        self.phase = Phase::Idle;
        if self.overlay.take().is_some() {
            return None;
        }
        if self.field.clear() {
            return Some(self.query());
        }
        Some(Effect::Close)
    }
}

/// A section's header. The Answer section, which holds only the calculator's card, has none.
fn header(section: Section) -> Option<&'static str> {
    match section {
        Section::Answer => None,
        Section::Results => Some("Results"),
        Section::Suggestions => Some("Suggestions"),
        Section::Commands => Some("Commands"),
    }
}

/// The calculator card's halves as the list draws them, each a text and its badge: the
/// expression, then the answer.
pub fn card_halves(card: &calc::Answer) -> [(&str, Option<&str>); 2] {
    [
        (card.expression(), Some(card.source_badge())),
        (card.display(), card.target_badge()),
    ]
}

/// The kind a row shows on its right. The card and clip rows show none.
pub fn kind_label(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::App => Some("Application"),
        Kind::Command => Some("Command"),
        Kind::Clip | Kind::Calculator => None,
    }
}

fn lines(rows: &[Row]) -> Vec<Line> {
    let mut lines = Vec::with_capacity(rows.len() + 3);
    let mut section = None;
    for (index, row) in rows.iter().enumerate() {
        if section != Some(row.section) {
            section = Some(row.section);
            lines.extend(header(row.section).map(Line::Header));
        }
        lines.push(Line::Row(index));
    }
    lines
}

#[cfg(test)]
fn row_text(row: &Row) -> String {
    if let Some(card) = &row.card {
        let [(expression, source), (display, target)] = card_halves(card);
        let badges: Vec<&str> = [Some(expression), source, target]
            .into_iter()
            .flatten()
            .collect();
        return format!("= {display}  ({})", badges.join(" · "));
    }
    match kind_label(row.subject.key().kind()) {
        Some(kind) => format!("{}  ({kind})", row.title),
        None => row.title.clone(),
    }
}
