//! Pure launcher logic for lyncast. This crate does no IO and has no UI or
//! Wayland dependencies; the clock and every external input are injected.
//! See `docs/design/architecture.md`.

mod command;
mod key;

pub use command::Command;
pub use key::{AppId, ClipId, Key, KeyError, Kind};
