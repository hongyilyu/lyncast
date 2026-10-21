//! The work that blocks, which runs on worker threads. Every program the daemon starts goes
//! through one [`Runner`], as the action that niri runs.

use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use lyncast_core::rank::Index;
use niri_ipc::{Action, Request};

use super::{Done, Task};
use crate::{apps, niri};

#[derive(Clone)]
pub enum Runner {
    /// Sends each action to the niri at this socket.
    Niri(Option<PathBuf>),
    /// Appends to the file the line that [`Runner::Niri`] would send for each action, and sends
    /// nothing. `lyncast daemon --record <file>` picks it, so a Tier 1 test never runs
    /// `systemctl reboot`.
    Recorder(Arc<Recorder>),
}

/// The recorder's file, and the lock that every clone of the runner shares.
pub struct Recorder {
    path: PathBuf,
    /// Held while one line is written. A FIFO takes at most 4 KiB in one write, so a longer
    /// line goes in several, and another worker's line could land between them.
    line: Mutex<()>,
}

impl Runner {
    pub fn recorder(path: PathBuf) -> Runner {
        Runner::Recorder(Arc::new(Recorder {
            path,
            line: Mutex::new(()),
        }))
    }

    pub fn send(&self, action: Action) -> Result<(), String> {
        match self {
            Runner::Niri(socket) => niri::act(socket.as_deref(), action),
            Runner::Recorder(recorder) => {
                let line = niri::line(&Request::Action(action));
                let failed =
                    |error: io::Error| format!("not run: {}: {error}", recorder.path.display());
                // Opened for each action, so a recorder that cannot be opened fails only that
                // program, and the tests count the daemon's descriptors on the file to see a
                // program waiting in the runner.
                let mut file = File::options()
                    .create(true)
                    .append(true)
                    .open(&recorder.path)
                    .map_err(failed)?;
                let _whole = recorder.line.lock().unwrap_or_else(PoisonError::into_inner);
                file.write_all(&line).map_err(failed)
            }
        }
    }
}

impl Task {
    pub fn run(self, runner: &Runner) -> Result<Done, String> {
        match self {
            Task::Act(action) => runner.send(action).map(|()| Done::Ran),
            Task::Launch(launch, lookup) => {
                let command = lookup.command(&launch)?;
                runner.send(Action::Spawn { command }).map(|()| Done::Ran)
            }
            Task::OpenConfig(path) => {
                create(&path)?;
                let command = vec!["xdg-open".to_owned(), path.display().to_string()];
                runner.send(Action::Spawn { command }).map(|()| Done::Ran)
            }
            Task::Scan { vars, number } => Ok(Done::Scanned {
                number,
                index: Index::new(apps::scan(&vars)),
            }),
        }
    }
}

/// Creates an empty file at `path`, and its directories, unless the file exists.
fn create(path: &Path) -> Result<(), String> {
    let failed = |error: io::Error| format!("not run: {}: {error}", path.display());
    fs::create_dir_all(path.parent().expect("config.toml is in a directory")).map_err(failed)?;
    match File::options().write(true).create_new(true).open(path) {
        Err(error) if error.kind() != ErrorKind::AlreadyExists => Err(failed(error)),
        _ => Ok(()),
    }
}
