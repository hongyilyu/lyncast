//! The socket's wire, as docs/design/daemon.md spells it: one compact JSON request line from
//! the client and one reply line from the daemon per connection.

use std::env;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::view::RowView;

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
