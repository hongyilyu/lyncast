//! Pure launcher logic for lyncast. This crate does no IO and has no UI or
//! Wayland dependencies; the clock and every external input are injected.
//! See `docs/design/architecture.md`.

mod actions;
pub mod calc;
pub mod clip;
mod command;
pub mod desktop;
mod fold;
mod key;
pub mod rank;
pub mod root;

pub use actions::{ActionItem, Actions, Chord, DesktopAction, Subject, Verb};
pub use command::Command;
pub use fold::{Folded, fold};
pub use key::{AppId, ClipId, Key, KeyError, Kind};

/// One row of the root list. Its key, kind, and actions all come from `subject`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub subject: Subject,
    pub section: Section,
    /// An app's `Name` in the session locale, a command's title, or the calculator answer's
    /// display.
    pub title: String,
    /// The calculator card's answer, on the Answer row only.
    pub card: Option<calc::Answer>,
}

/// The root list's sections. Declaration order is the merge order. A typed query
/// fills Answer and Results, and an empty query fills Suggestions and Commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Answer,
    Results,
    Suggestions,
    Commands,
}
