use std::borrow::Cow;
#[cfg(test)]
use std::fmt::Write;
use std::mem;
use std::ops::Range;

use lyncast_core::{ActionItem, Chord, Key, Kind, Row, Section, Subject, calc, fold};

use super::clips::{ClipLine, Day, Kinds};
use super::field::Field;
use crate::daemon::{After, Answer, Screen};
use crate::wire::{Request, Via};

/// What the empty field shows on the root list.
const PLACEHOLDER: &str = "Search for apps and commands";
/// What the empty field shows on the clipboard screen.
const CLIPS_PLACEHOLDER: &str = "Search clipboard history";
/// What the Actions menu's empty field shows.
pub const MENU_PLACEHOLDER: &str = "Search for actions";
/// What an empty root list shows.
const NO_RESULTS: &str = "No results";
/// What an empty clipboard screen shows.
const NO_CLIPS: &str = "No clips";
/// What the Actions menu shows when its filter keeps no item.
pub const NO_ACTIONS: &str = "No actions";

/// The key whose release closes the window after a success.
const ACTIVATING: &str = "enter";

pub struct Model {
    field: Field,
    scope: Scope,
    lines: Vec<Line>,
    selected_row: usize,
    overlay: Option<Overlay>,
    phase: Phase,
    was_active: bool,
}

enum Scope {
    Root(Vec<Row>),
    Clipboard(Clipboard),
}

/// The clipboard screen. `pushed` when it came from the root list, so Escape goes back there
/// instead of closing.
struct Clipboard {
    pushed: bool,
    kinds: Kinds,
    clips: Vec<ClipLine>,
}

impl Scope {
    fn clipboard(pushed: bool) -> Scope {
        Scope::Clipboard(Clipboard {
            pushed,
            kinds: Kinds::All,
            clips: Vec::new(),
        })
    }
}

enum Overlay {
    Confirm {
        key: Key,
        via: Via,
        title: &'static str,
    },
    Menu(Menu),
}

/// The Actions menu of one row: the row's items, default first, and a field that filters
/// them. The keys and text go to it while it is open.
pub struct Menu {
    subject: Subject,
    items: Vec<(Option<Chord>, ActionItem)>,
    field: Field,
    selected_shown: usize,
}

enum MenuAfter {
    Edited,
    Ignored,
    Close,
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

/// A key that the footer or the menu names, as the view draws its cap.
#[derive(Clone, Copy)]
pub enum Cap {
    Enter,
    Escape,
    CtrlK,
    CtrlEnter,
}

impl Cap {
    pub fn of(chord: Chord) -> Cap {
        match chord {
            Chord::Enter => Cap::Enter,
            Chord::CtrlEnter => Cap::CtrlEnter,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Cap::Enter => "↵",
            Cap::Escape => "esc",
            Cap::CtrlK => "Ctrl K",
            Cap::CtrlEnter => "Ctrl ↵",
        }
    }

    /// How a `palette` block spells the key.
    #[cfg(test)]
    pub fn spelling(self) -> &'static str {
        match self {
            Cap::Enter => "enter",
            Cap::Escape => "escape",
            Cap::CtrlK => "ctrl-k",
            Cap::CtrlEnter => "ctrl-enter",
        }
    }
}

impl Footer<'_> {
    /// What the footer names, in order: a label and the key that runs it. The first is what
    /// Enter runs.
    pub fn hints(&self) -> Vec<(&str, Cap)> {
        match self {
            Footer::Hint(label) => vec![(label, Cap::Enter), ("Actions", Cap::CtrlK)],
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
    Header(Heading),
    /// The row at this index in the list that shows, which [`Model::item`] gives.
    Row(usize),
}

/// What a header names: a section of the root list, or a day of the clipboard screen's clips, or
/// its pins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heading {
    Section(&'static str),
    Day(Day),
}

impl Heading {
    pub fn label(self) -> Cow<'static, str> {
        match self {
            Heading::Section(name) => name.into(),
            Heading::Day(day) => day.label().into(),
        }
    }
}

/// A row of the list that shows.
pub enum Item<'m> {
    Row(&'m Row),
    Clip(&'m ClipLine),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// The first press of Enter or Ctrl+Enter.
    Activate(Chord),
    /// A held key's repeat of Enter, Escape, Ctrl+K, Tab, or Ctrl+T, which the palette takes
    /// and drops.
    Ignore,
    /// Ctrl+K: opens the selected row's Actions menu, or closes it.
    Menu,
    /// Tab: the root list or the clipboard screen, whichever does not show.
    Tab,
    /// Ctrl+T: the clipboard screen's next type filter.
    CycleKinds,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    /// `held` for a held key's repeat, which deletes but never closes the menu.
    Backspace {
        held: bool,
    },
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
    /// The clipboard rows for the field's text and the type filter.
    Clips(Vec<ClipLine>),
    /// The answer to the activation that is out.
    Answer(Answer),
    /// Clipboard History from a client, while the window is open.
    Show,
}

/// The one thing the view does next.
pub enum Effect {
    /// Ask the daemon for the root list of this text.
    Query(String),
    /// Read the clipboard rows for this filter and these kinds from the daemon's history.
    Clips {
        filter: String,
        kinds: Kinds,
    },
    /// Send the daemon this activation.
    Ask(Request),
    Close,
}

impl Model {
    /// The model of a summon that has just opened on `screen`, and the list it asks for. A
    /// summon that opens on the clipboard screen goes nowhere back: Escape closes it.
    pub fn new(screen: Screen) -> (Model, Effect) {
        let scope = match screen {
            Screen::Root => Scope::Root(Vec::new()),
            Screen::Clipboard => Scope::clipboard(false),
        };
        let model = Model {
            field: Field::default(),
            scope,
            lines: Vec::new(),
            selected_row: 0,
            overlay: None,
            phase: Phase::Idle,
            was_active: false,
        };
        let list = model.list();
        (model, list)
    }

    /// A key press, typed text, or an input method's text. While an activation is out or the
    /// window is closing, only Escape does something: it closes the window, and the answer
    /// goes nowhere.
    pub fn input(&mut self, input: Input) -> Option<Effect> {
        if let Phase::Pending { .. } | Phase::Closing { .. } = self.phase {
            return matches!(input, Input::Press(Press::Escape)).then_some(Effect::Close);
        }
        match input {
            Input::Press(Press::Activate(chord)) => return self.activate(Via::Chord(chord)),
            Input::Press(Press::Ignore) => return None,
            Input::Press(Press::Escape) => return self.escape(),
            Input::Press(Press::Menu) => {
                self.toggle_menu();
                return None;
            }
            Input::Unmark => {
                self.focused_mut().unmark();
                return None;
            }
            _ => {}
        }
        if let Some(Overlay::Menu(menu)) = &mut self.overlay {
            match menu.edit(input) {
                MenuAfter::Edited => self.phase = Phase::Idle,
                MenuAfter::Ignored => {}
                MenuAfter::Close => self.overlay = None,
            }
            return None;
        }
        self.overlay = None;
        match input {
            Input::Press(Press::Tab) => self.tab(),
            Input::Press(Press::CycleKinds) => self.cycle_kinds(),
            Input::Press(Press::Backspace { held: false }) if self.field.is_empty() => self.back(),
            input => {
                self.phase = Phase::Idle;
                self.edit(input).then(|| self.list())
            }
        }
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
                if let Scope::Root(held) = &mut self.scope {
                    self.lines = lines(&rows);
                    *held = rows;
                    self.selected_row = 0;
                }
                None
            }
            Landed::Clips(clips) => {
                let Scope::Clipboard(clipboard) = &mut self.scope else {
                    return None;
                };
                self.lines = day_lines(&clips);
                self.selected_row = clips.iter().position(|clip| !clip.pinned).unwrap_or(0);
                clipboard.clips = clips;
                None
            }
            Landed::Answer(answer) => self.answered(answer, mods),
            Landed::Show => self.show_clipboard(),
        }
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The clipboard screen's type filter, or `None` on the root list.
    pub fn kinds(&self) -> Option<Kinds> {
        match &self.scope {
            Scope::Clipboard(clipboard) => Some(clipboard.kinds),
            Scope::Root(_) => None,
        }
    }

    /// What the field shows while it is empty.
    pub fn placeholder(&self) -> &'static str {
        match self.scope {
            Scope::Root(_) => PLACEHOLDER,
            Scope::Clipboard(_) => CLIPS_PLACEHOLDER,
        }
    }

    /// What the list shows while it is empty.
    pub fn empty_text(&self) -> &'static str {
        match self.scope {
            Scope::Root(_) => NO_RESULTS,
            Scope::Clipboard(_) => NO_CLIPS,
        }
    }

    /// The field that has the caret and gets the window's text: the menu's while it is open.
    pub fn focused(&self) -> &Field {
        match &self.overlay {
            Some(Overlay::Menu(menu)) => &menu.field,
            _ => &self.field,
        }
    }

    fn focused_mut(&mut self) -> &mut Field {
        match &mut self.overlay {
            Some(Overlay::Menu(menu)) => &mut menu.field,
            _ => &mut self.field,
        }
    }

    pub fn menu(&self) -> Option<&Menu> {
        match &self.overlay {
            Some(Overlay::Menu(menu)) => Some(menu),
            _ => None,
        }
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The row of the list that shows at `index`.
    pub fn item(&self, index: usize) -> Item<'_> {
        match &self.scope {
            Scope::Root(rows) => Item::Row(&rows[index]),
            Scope::Clipboard(clipboard) => Item::Clip(&clipboard.clips[index]),
        }
    }

    pub fn is_selected(&self, index: usize) -> bool {
        index == self.selected_row
    }

    /// The selected row's line, if the list has a row.
    pub fn selected_line(&self) -> Option<usize> {
        let line = Line::Row(self.selected_row);
        self.lines.iter().position(|&other| other == line)
    }

    pub fn footer(&self) -> Option<Footer<'_>> {
        Some(match &self.phase {
            Phase::Pending { .. } => Footer::Pending,
            Phase::Closing { .. } => Footer::Closing,
            Phase::Failed(reason) => Footer::Failed(reason),
            Phase::Idle => match &self.overlay {
                Some(Overlay::Confirm { title, .. }) => Footer::Confirm(title),
                Some(Overlay::Menu(menu)) => Footer::Hint(menu.picked()?.label().to_owned()),
                None => {
                    let subject = self.subject()?;
                    Footer::Hint(subject.actions().on(Chord::Enter).label().to_owned())
                }
            },
        })
    }

    /// The text of palette.md's `palette` blocks: the field, the lines with the selected row
    /// marked, the overlay, and the footer.
    #[cfg(test)]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        let field = if self.field.shows_placeholder() {
            format!("({})", self.placeholder())
        } else {
            format!("{:?}", self.field.text())
        };
        let _ = match self.kinds() {
            None => writeln!(out, "root {field}"),
            Some(kinds) => writeln!(out, "clipboard {field} {}", kinds.label()),
        };
        for line in &self.lines {
            let _ = match *line {
                Line::Header(heading) => writeln!(out, "== {}", heading.label()),
                Line::Row(index) => {
                    let mark = if self.is_selected(index) { ">" } else { " " };
                    match self.item(index) {
                        Item::Row(row) => writeln!(out, "{mark} {}", row_text(row)),
                        Item::Clip(clip) => {
                            writeln!(out, "{mark} {}: {}", clip.content.spelling(), clip.title)
                        }
                    }
                }
            };
        }
        if self.lines.is_empty() {
            let _ = writeln!(out, "{}", self.empty_text());
        }
        match &self.overlay {
            Some(Overlay::Confirm { title, .. }) => {
                let _ = writeln!(out, "confirm {}", question(title));
            }
            Some(Overlay::Menu(menu)) => {
                if menu.field.shows_placeholder() {
                    let _ = writeln!(out, "menu ({MENU_PLACEHOLDER})");
                } else {
                    let _ = writeln!(out, "menu {:?}", menu.field.text());
                }
                for (at, (chord, item)) in menu.shown().enumerate() {
                    let mark = if at == menu.selected_shown { ">" } else { " " };
                    let _ = match chord {
                        Some(chord) => {
                            writeln!(
                                out,
                                "{mark} {}  ({})",
                                item.label(),
                                Cap::of(*chord).spelling()
                            )
                        }
                        None => writeln!(out, "{mark} {}", item.label()),
                    };
                }
                if menu.shown().next().is_none() {
                    let _ = writeln!(out, "{NO_ACTIONS}");
                }
            }
            None => {}
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

    fn subject(&self) -> Option<Subject> {
        match &self.scope {
            Scope::Root(rows) => rows.get(self.selected_row).map(|row| row.subject.clone()),
            Scope::Clipboard(clipboard) => clipboard
                .clips
                .get(self.selected_row)
                .map(ClipLine::subject),
        }
    }

    fn len(&self) -> usize {
        match &self.scope {
            Scope::Root(rows) => rows.len(),
            Scope::Clipboard(clipboard) => clipboard.clips.len(),
        }
    }

    fn list(&self) -> Effect {
        let text = self.field.text().to_owned();
        match &self.scope {
            Scope::Root(_) => Effect::Query(text),
            Scope::Clipboard(clipboard) => Effect::Clips {
                filter: text,
                kinds: clipboard.kinds,
            },
        }
    }

    fn show(&mut self, scope: Scope) -> Option<Effect> {
        self.scope = scope;
        self.lines.clear();
        self.selected_row = 0;
        Some(self.list())
    }

    fn edit(&mut self, input: Input) -> bool {
        match input {
            Input::Press(Press::Up) => {
                self.selected_row = self.selected_row.saturating_sub(1);
                false
            }
            Input::Press(Press::Down) => {
                self.selected_row = (self.selected_row + 1).min(self.len().saturating_sub(1));
                false
            }
            input => edit(&mut self.field, input),
        }
    }

    fn tab(&mut self) -> Option<Effect> {
        match self.scope {
            Scope::Root(_) => self.show(Scope::clipboard(true)),
            Scope::Clipboard(_) => self.show(Scope::Root(Vec::new())),
        }
    }

    fn cycle_kinds(&mut self) -> Option<Effect> {
        let Scope::Clipboard(clipboard) = &mut self.scope else {
            return None;
        };
        clipboard.kinds = clipboard.kinds.next();
        Some(self.list())
    }

    fn back(&mut self) -> Option<Effect> {
        match self.scope {
            Scope::Clipboard(Clipboard { pushed: true, .. }) => self.show(Scope::Root(Vec::new())),
            _ => None,
        }
    }

    fn show_clipboard(&mut self) -> Option<Effect> {
        let pushed = match &self.scope {
            Scope::Root(_) => true,
            Scope::Clipboard(clipboard) => clipboard.pushed,
        };
        self.overlay = None;
        self.field.clear();
        self.show(Scope::clipboard(pushed))
    }

    fn toggle_menu(&mut self) {
        self.field.unmark();
        self.overlay = match self.overlay.take() {
            Some(Overlay::Menu(_)) => None,
            _ => self
                .subject()
                .map(|subject| Overlay::Menu(Menu::new(subject))),
        };
    }

    fn activate(&mut self, via: Via) -> Option<Effect> {
        let (key, via, confirmed) = match &self.overlay {
            Some(Overlay::Confirm { key, via, .. }) => (key.clone(), via.clone(), true),
            Some(Overlay::Menu(menu)) => {
                (menu.subject.key(), Via::Menu(menu.picked()?.verb()), false)
            }
            None => (self.subject()?.key(), via, false),
        };
        self.overlay = None;
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
            Answer::Done {
                after: After::ShowClipboard,
                ..
            } => return self.show_clipboard(),
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
        if self.overlay.take().is_some() {
            return None;
        }
        if self.field.clear() {
            self.phase = Phase::Idle;
            return Some(self.list());
        }
        self.back().or(Some(Effect::Close))
    }
}

impl Menu {
    fn new(subject: Subject) -> Menu {
        let items = subject
            .actions()
            .iter()
            .map(|(chord, item)| (chord, item.clone()))
            .collect();
        Menu {
            subject,
            items,
            field: Field::default(),
            selected_shown: 0,
        }
    }

    pub fn field(&self) -> &Field {
        &self.field
    }

    /// The items whose folded label holds the folded filter, in menu order.
    pub fn shown(&self) -> impl Iterator<Item = &(Option<Chord>, ActionItem)> {
        let filter = fold(self.field.text());
        self.items
            .iter()
            .filter(move |(_, item)| fold(item.label()).lower().contains(filter.lower()))
    }

    pub fn is_selected(&self, at: usize) -> bool {
        at == self.selected_shown
    }

    fn picked(&self) -> Option<&ActionItem> {
        self.shown().nth(self.selected_shown).map(|(_, item)| item)
    }

    fn edit(&mut self, input: Input) -> MenuAfter {
        match input {
            Input::Press(Press::Up) => self.selected_shown = self.selected_shown.saturating_sub(1),
            Input::Press(Press::Down) => {
                self.selected_shown =
                    (self.selected_shown + 1).min(self.shown().count().saturating_sub(1));
            }
            Input::Press(Press::Backspace { held: false }) if self.field.is_empty() => {
                return MenuAfter::Close;
            }
            Input::Press(Press::Tab | Press::CycleKinds) => return MenuAfter::Ignored,
            input => {
                if edit(&mut self.field, input) {
                    self.selected_shown = 0;
                }
            }
        }
        MenuAfter::Edited
    }
}

fn edit(field: &mut Field, input: Input) -> bool {
    match input {
        Input::Press(Press::Backspace { .. }) => field.backspace(),
        Input::Press(Press::Delete) => field.delete(),
        Input::Press(Press::Left) => {
            field.left();
            false
        }
        Input::Press(Press::Right) => {
            field.right();
            false
        }
        Input::Press(Press::Home) => {
            field.home();
            false
        }
        Input::Press(Press::End) => {
            field.end();
            false
        }
        Input::Replace { utf16, text } => field.replace(utf16, &text),
        Input::Mark {
            utf16,
            text,
            selected,
        } => field.mark(utf16, &text, selected),
        Input::Unmark
        | Input::Press(
            Press::Activate(_)
            | Press::Ignore
            | Press::Menu
            | Press::Tab
            | Press::CycleKinds
            | Press::Up
            | Press::Down
            | Press::Escape,
        ) => false,
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
            lines.extend(header(row.section).map(|name| Line::Header(Heading::Section(name))));
        }
        lines.push(Line::Row(index));
    }
    lines
}

fn day_lines(clips: &[ClipLine]) -> Vec<Line> {
    let mut lines = Vec::with_capacity(clips.len() + 4);
    let mut day = None;
    for (index, clip) in clips.iter().enumerate() {
        if day != Some(clip.day) {
            day = Some(clip.day);
            lines.push(Line::Header(Heading::Day(clip.day)));
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
