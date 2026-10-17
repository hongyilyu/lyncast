//! The socket's wire, as docs/design/daemon.md spells it: one compact JSON request line from
//! the client and one reply line from the daemon per connection. Keys, chords, and verbs travel
//! as the strings `query --json` prints, and the socket parses them.

use std::env;
use std::fmt;
use std::path::PathBuf;

use lyncast_core::{Chord, Key, Verb};
use serde::{Deserialize, Serialize};

use crate::view::{RowView, Spelled};

/// The socket's file name in `$XDG_RUNTIME_DIR`.
pub const SOCKET: &str = "lyncast.sock";

/// What a client asks the daemon.
#[derive(Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "lowercase")]
pub enum Request {
    Toggle,
    Status,
    /// The root list for the query as typed.
    Query {
        text: String,
    },
    /// Runs one item of the row that `key` names. `query` is the search text the row was found
    /// with, which frecency keeps as a search term.
    Activate {
        #[serde(with = "spelled")]
        key: Key,
        via: Via,
        confirmed: bool,
        query: String,
    },
}

/// How an activation picks its item: the keys pressed, or a verb from the Actions menu.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Chord(#[serde(with = "spelled")] Chord),
    #[serde(rename = "verb")]
    Menu(#[serde(with = "spelled")] Verb),
}

/// What the daemon answers.
#[derive(Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "lowercase")]
pub enum Reply {
    Summon {
        state: Summon,
    },
    Rows {
        rows: Vec<RowView>,
    },
    /// The success line, such as `ran cmd:reboot`.
    Done {
        message: String,
    },
    /// The command asks first. Nothing ran.
    Confirm {
        title: String,
    },
    /// The reason, without the `lyncast: ` prefix.
    Failed {
        reason: String,
    },
}

/// Without GPUI the summon is a state. From plan step 7 an open summon is a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Summon {
    Closed,
    Open,
}

impl fmt::Display for Summon {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Summon::Closed => "closed",
            Summon::Open => "open",
        })
    }
}

/// `$XDG_RUNTIME_DIR`, which holds the socket and its lock. An empty or relative value counts
/// as unset, as the XDG Base Directory spec asks.
pub fn runtime_dir() -> Result<PathBuf, String> {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_owned())
}

/// `#[serde(with)]` for a [`Spelled`] field. A spelling that reads as nothing fails the request
/// with the reason and the text, as in `bad clip id in clip:x`.
mod spelled {
    use serde::de::Error;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::Spelled;

    pub fn serialize<T: Spelled, S: Serializer>(value: &T, out: S) -> Result<S::Ok, S::Error> {
        out.serialize_str(&value.spelling())
    }

    pub fn deserialize<'de, T: Spelled, D: Deserializer<'de>>(input: D) -> Result<T, D::Error> {
        let text = String::deserialize(input)?;
        T::read(&text).map_err(|reason| D::Error::custom(format!("{reason} in {text}")))
    }
}
