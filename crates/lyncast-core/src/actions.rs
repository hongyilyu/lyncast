use crate::{AppId, ClipId, Command, Key};

/// One `[Desktop Action <id>]` group of a desktop entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
}

/// What a row is about, with exactly the facts the chord table reads. A feature builds one
/// for each row, and the daemon builds it again from current state at every activation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    App {
        id: AppId,
        /// In the entry's `Actions=` order.
        actions: Vec<DesktopAction>,
    },
    Command(Command),
    Clip {
        id: ClipId,
        pinned: bool,
    },
    Calc {
        /// The raw query, so activation evaluates the same text again.
        query: String,
    },
}

impl Subject {
    /// The row's key. It ignores the facts, so a clip has one key pinned or not.
    pub fn key(&self) -> Key {
        match self {
            Subject::App { id, .. } => Key::App(id.clone()),
            Subject::Command(command) => Key::Command(*command),
            Subject::Clip { id, .. } => Key::Clip(*id),
            Subject::Calc { query } => Key::Calc(query.clone()),
        }
    }

    /// The chord table: the only list of what a row can do.
    pub fn actions(&self) -> Actions {
        match self {
            Subject::App { id, actions } => Actions {
                enter: ActionItem::Launch(id.clone()),
                more: actions
                    .iter()
                    .map(|action| ActionItem::LaunchAction(id.clone(), action.clone()))
                    .chain([ActionItem::CopyPath(id.clone())])
                    .collect(),
            },
            Subject::Command(command) => Actions {
                enter: ActionItem::Run(*command),
                more: Vec::new(),
            },
            Subject::Clip { id, pinned } => Actions {
                enter: ActionItem::CopyClip(*id),
                more: vec![
                    if *pinned {
                        ActionItem::Unpin(*id)
                    } else {
                        ActionItem::Pin(*id)
                    },
                    ActionItem::Delete(*id),
                ],
            },
            Subject::Calc { query } => Actions {
                enter: ActionItem::CopyAnswer(query.clone()),
                more: Vec::new(),
            },
        }
    }
}

/// The keys that activate a row. The palette reports the chord and leaves its meaning to the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chord {
    Enter,
    CtrlEnter,
}

impl Chord {
    /// Every chord, in declaration order.
    pub const ALL: [Chord; 2] = [Chord::Enter, Chord::CtrlEnter];
}

/// A menu item's name without its row's ids. A menu pick sends one, and [`Actions::pick`] checks it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Verb {
    Launch,
    LaunchAction(String),
    CopyPath,
    Copy,
    Pin,
    Unpin,
    Delete,
    Run,
}

/// One thing a row can do, bound to the row's ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionItem {
    Launch(AppId),
    LaunchAction(AppId, DesktopAction),
    CopyPath(AppId),
    CopyClip(ClipId),
    CopyAnswer(String),
    Pin(ClipId),
    Unpin(ClipId),
    Delete(ClipId),
    Run(Command),
}

impl ActionItem {
    /// The name a menu pick sends for this item.
    pub fn verb(&self) -> Verb {
        match self {
            ActionItem::Launch(_) => Verb::Launch,
            ActionItem::LaunchAction(_, action) => Verb::LaunchAction(action.id.clone()),
            ActionItem::CopyPath(_) => Verb::CopyPath,
            ActionItem::CopyClip(_) | ActionItem::CopyAnswer(_) => Verb::Copy,
            ActionItem::Pin(_) => Verb::Pin,
            ActionItem::Unpin(_) => Verb::Unpin,
            ActionItem::Delete(_) => Verb::Delete,
            ActionItem::Run(_) => Verb::Run,
        }
    }

    /// The text the footer and the Actions menu show.
    pub fn label(&self) -> &str {
        match self {
            ActionItem::Launch(_) => "Open",
            ActionItem::LaunchAction(_, action) => &action.name,
            ActionItem::CopyPath(_) => "Copy Path to .desktop",
            ActionItem::CopyClip(_) => "Copy",
            ActionItem::CopyAnswer(_) => "Copy Answer",
            ActionItem::Pin(_) => "Pin",
            ActionItem::Unpin(_) => "Unpin",
            ActionItem::Delete(_) => "Delete",
            ActionItem::Run(_) => "Run",
        }
    }
}

/// A row's actions in menu order. Only [`Subject::actions`] builds one, so it is never
/// empty and Enter always has an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actions {
    enter: ActionItem,
    more: Vec<ActionItem>,
}

impl Actions {
    /// The item a chord runs. Both chords run the default in the MVP. Paste will give
    /// Ctrl+Enter its own item on clips and the calculator.
    pub fn on(&self, chord: Chord) -> &ActionItem {
        match chord {
            Chord::Enter | Chord::CtrlEnter => &self.enter,
        }
    }

    /// The item a menu pick names, if the row offers it now.
    pub fn pick(&self, verb: &Verb) -> Option<&ActionItem> {
        self.iter()
            .map(|(_, item)| item)
            .find(|item| item.verb() == *verb)
    }

    /// Menu order, default first, with the key each item shows.
    pub fn iter(&self) -> impl Iterator<Item = (Option<Chord>, &ActionItem)> {
        std::iter::once((Some(Chord::Enter), &self.enter))
            .chain(self.more.iter().map(|item| (None, item)))
    }
}
