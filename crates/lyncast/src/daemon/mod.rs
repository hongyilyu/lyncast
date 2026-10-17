//! The daemon's state and what each request does to it. [`serve`] owns the socket and the
//! threads, and only its owner loop calls [`Daemon::handle`], so no lock guards the state.
//! docs/design/daemon.md describes the requests.

mod serve;

pub use serve::run;

use lyncast_core::rank::{Index, Timestamp, Visits};

use crate::view;
use crate::wire::{Reply, Request, Summon};

/// Everything the requests read and change.
pub struct Daemon {
    index: Index,
    summon: Summon,
}

impl Daemon {
    pub fn new(index: Index) -> Daemon {
        Daemon {
            index,
            summon: Summon::Closed,
        }
    }

    /// Answers one request at `now`.
    pub fn handle(&mut self, request: Request, now: Timestamp) -> Reply {
        match request {
            Request::Toggle => {
                self.summon = match self.summon {
                    Summon::Closed => Summon::Open,
                    Summon::Open => Summon::Closed,
                };
                Reply::Summon { state: self.summon }
            }
            Request::Status => Reply::Summon { state: self.summon },
            Request::Query { text } => Reply::Rows {
                rows: view::rows(&self.index.rows(&text, &Visits::default(), now)),
            },
        }
    }
}
