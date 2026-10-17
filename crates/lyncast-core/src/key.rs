use std::fmt;
use std::str::FromStr;

use crate::Command;

/// A desktop entry id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppId(pub String);

/// A Clips row id, the sqlite rowid type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipId(pub i64);

/// Names a row without daemon state: `app:<id>`, `cmd:<id>`, `clip:<digits>`, `calc:<raw query>`.
/// Parsing splits at the first `:`, so a calculator query keeps its colons.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    App(AppId),
    Command(Command),
    Clip(ClipId),
    Calc(String),
}

/// The kind of row a key names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    App,
    Command,
    Clip,
    Calculator,
}

/// Why a key string did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyError {
    UnknownKind,
    Empty,
    UnknownCommand,
    BadClipId,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KeyError::UnknownKind => "unknown kind",
            KeyError::Empty => "nothing after the colon",
            KeyError::UnknownCommand => "unknown command",
            KeyError::BadClipId => "bad clip id",
        })
    }
}

impl Kind {
    /// Every kind, in declaration order.
    pub const ALL: [Kind; 4] = [Kind::App, Kind::Command, Kind::Clip, Kind::Calculator];

    fn prefix(self) -> &'static str {
        match self {
            Kind::App => "app",
            Kind::Command => "cmd",
            Kind::Clip => "clip",
            Kind::Calculator => "calc",
        }
    }
}

impl Key {
    pub fn kind(&self) -> Kind {
        match self {
            Key::App(_) => Kind::App,
            Key::Command(_) => Kind::Command,
            Key::Clip(_) => Kind::Clip,
            Key::Calc(_) => Kind::Calculator,
        }
    }
}

impl FromStr for Key {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Key, KeyError> {
        let (prefix, rest) = s.split_once(':').ok_or(KeyError::UnknownKind)?;
        let kind = Kind::ALL
            .into_iter()
            .find(|kind| kind.prefix() == prefix)
            .ok_or(KeyError::UnknownKind)?;
        match kind {
            Kind::App => non_empty(rest).map(|id| Key::App(AppId(id.to_owned()))),
            Kind::Command => Command::ALL
                .into_iter()
                .find(|command| command.id() == rest)
                .map(Key::Command)
                .ok_or(KeyError::UnknownCommand),
            Kind::Clip => clip_id(rest).map(Key::Clip),
            Kind::Calculator => non_empty(rest).map(|query| Key::Calc(query.to_owned())),
        }
    }
}

fn non_empty(rest: &str) -> Result<&str, KeyError> {
    if rest.is_empty() {
        Err(KeyError::Empty)
    } else {
        Ok(rest)
    }
}

/// `i64::from_str` also takes a sign, so the digits are checked first.
fn clip_id(rest: &str) -> Result<ClipId, KeyError> {
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return Err(KeyError::BadClipId);
    }
    rest.parse().map(ClipId).map_err(|_| KeyError::BadClipId)
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.kind().prefix())?;
        match self {
            Key::App(AppId(id)) => f.write_str(id),
            Key::Command(command) => f.write_str(command.id()),
            Key::Clip(ClipId(id)) => write!(f, "{id}"),
            Key::Calc(query) => f.write_str(query),
        }
    }
}
