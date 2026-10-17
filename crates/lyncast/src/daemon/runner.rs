//! The work that blocks, which runs on worker threads. Every program the daemon starts goes
//! through one [`Runner`], as an argv.

use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use lyncast_core::rank::Index;

use super::{Done, Work};
use crate::apps;

#[derive(Clone)]
pub enum Runner {
    /// Starts programs through niri IPC, which plan step 8 builds.
    Real,
    /// Appends each argv to the file as one compact JSON array per line, and starts nothing.
    /// `lyncast daemon --record <file>` picks it, so a Tier 1 test never runs `systemctl reboot`.
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

    pub fn spawn(&self, argv: &[String]) -> Result<(), String> {
        match self {
            Runner::Real => Err("not run: launching through niri is not built yet".to_owned()),
            Runner::Recorder(recorder) => {
                let mut line = serde_json::to_vec(argv).expect("an argv serializes");
                line.push(b'\n');
                let failed =
                    |error: io::Error| format!("not run: {}: {error}", recorder.path.display());
                // Opened for each argv, so a recorder that cannot be opened fails only that
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

impl Work {
    pub fn run(self, runner: &Runner) -> Result<Done, String> {
        match self {
            Work::Spawn(argv) => runner.spawn(&argv).map(|()| Done::Ran),
            Work::OpenConfig(path) => {
                create(&path)?;
                let argv = ["xdg-open".to_owned(), path.display().to_string()];
                runner.spawn(&argv).map(|()| Done::Ran)
            }
            Work::Scan { vars, number } => Ok(Done::Scanned {
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
