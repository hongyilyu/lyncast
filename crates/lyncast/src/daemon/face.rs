//! The summon, which each host shows through a [`Face`], and [`apply`], the one table of what an
//! answer to a socket client does to it.

use super::{After, Answer, Ask};
use crate::wire::Summon;

/// What shows the summon. Each host owns one, and it is the only state that says whether the
/// summon is open. It only does what [`apply`] decides.
pub(super) trait Face {
    fn summon(&self) -> Summon;
    fn open(&mut self);
    /// Closes the open summon: only [`IsOpen::of`] makes the proof.
    fn close(&mut self, open: IsOpen);
}

/// Proof that a face's summon is open, which [`Face::close`] takes, so no face is asked to
/// close a closed summon.
pub(super) struct IsOpen(());

impl IsOpen {
    pub(super) fn of(face: &dyn Face) -> Option<IsOpen> {
        (face.summon() == Summon::Open).then_some(IsOpen(()))
    }
}

/// The headless host's summon: a state, with no window.
pub(super) struct Windowless(pub(super) Summon);

impl Face for Windowless {
    fn summon(&self) -> Summon {
        self.0
    }

    fn open(&mut self) {
        self.0 = Summon::Open;
    }

    fn close(&mut self, _: IsOpen) {
        self.0 = Summon::Closed;
    }
}

/// The summon rules: what an answer to a client does to the summon, and the summon's state after
/// it. Toggle opens a closed summon and closes an open one. A success closes it, except Clipboard
/// History's, which opens it. Anything else leaves it alone.
pub(super) fn apply(face: &mut dyn Face, answer: &Answer) -> Summon {
    match (answer, IsOpen::of(face)) {
        (Answer::Summon(Ask::Toggle), Some(open)) => face.close(open),
        (Answer::Summon(Ask::Toggle), None) => face.open(),
        (
            Answer::Done {
                after: After::Close,
                ..
            },
            Some(open),
        ) => face.close(open),
        (
            Answer::Done {
                after: After::ShowClipboard,
                ..
            },
            None,
        ) => face.open(),
        _ => {}
    }
    face.summon()
}
