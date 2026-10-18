#[cfg(test)]
use std::fmt::Write;
use std::ops::Range;

use lyncast_core::{Chord, Kind, Row, Section, calc};

use super::field::Field;

/// What the empty field shows.
pub const PLACEHOLDER: &str = "Search for apps and commands";
/// What an empty list shows.
pub const NO_RESULTS: &str = "No results";

pub struct Model {
    field: Field,
    rows: Vec<Row>,
    lines: Vec<Line>,
    selected_row: usize,
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
}

/// The one thing the view does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    /// Ask the daemon for the root list of this text.
    Query(String),
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
        };
        (model, Effect::Query(String::new()))
    }

    pub fn input(&mut self, input: Input) -> Option<Effect> {
        let edited = match input {
            Input::Press(Press::Up) => {
                self.selected_row = self.selected_row.saturating_sub(1);
                false
            }
            Input::Press(Press::Down) => {
                self.selected_row = (self.selected_row + 1).min(self.rows.len().saturating_sub(1));
                false
            }
            Input::Press(Press::Escape) => return self.escape(),
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
            Input::Unmark => {
                self.field.unmark();
                false
            }
        };
        edited.then(|| self.query())
    }

    pub fn land(&mut self, landed: Landed) -> Option<Effect> {
        match landed {
            Landed::Rows(rows) => {
                self.lines = lines(&rows);
                self.rows = rows;
                self.selected_row = 0;
            }
        }
        None
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

    /// What the footer shows: the label of the selected row's default action.
    pub fn footer(&self) -> Option<String> {
        let row = self.rows.get(self.selected_row)?;
        Some(row.subject.actions().on(Chord::Enter).label().to_owned())
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
        if let Some(footer) = self.footer() {
            let _ = writeln!(out, "footer: {footer}");
        }
        out
    }

    /// The root list's request for the field's text.
    fn query(&self) -> Effect {
        Effect::Query(self.field.text().to_owned())
    }

    fn escape(&mut self) -> Option<Effect> {
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
