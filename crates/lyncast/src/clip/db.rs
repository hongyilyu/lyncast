use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::rank::Timestamp;
use rusqlite::{Connection, params};

use super::load::{self, Loaded};
use super::{Seq, WRITER_ENDED};
use crate::files::Unsaved;
use crate::warning::warn;

/// SQLite opens the file, and the journal beside it, by path after the daemon's check, and an
/// open can wait on a FIFO for good.
const LOAD_WAIT: Duration = Duration::from_secs(5);

/// The writer thread's name, by which the `freeze-thread` fixtures find it.
const THREAD: &str = "clips";

pub enum Change {
    Pin {
        id: ClipId,
        used: Timestamp,
        seq: Seq,
    },
    Unpin {
        id: ClipId,
        used: Timestamp,
    },
    Delete {
        ids: Vec<ClipId>,
    },
}

enum Command {
    Change(Change),
    Close,
}

/// What [`Writer::open`] comes to once its thread has started.
pub enum Opened {
    Loaded(Writer, Loaded),
    /// Why the rows stay in memory, and the file is never written.
    InMemory(String),
}

pub struct Writer {
    commands: Sender<Command>,
}

impl Writer {
    /// Starts the thread that opens the file at `path`, loads it, and then applies each change,
    /// and waits [`LOAD_WAIT`] at most for the load. The thread calls `ended` when it ends after
    /// [`Writer::close`], once it has closed the connection and so given up the lock. Fails only
    /// when the thread cannot start. The thread of a load that fails or runs over gets no change
    /// and does not call `ended`.
    pub fn open(path: PathBuf, ended: impl FnOnce() + Send + 'static) -> io::Result<Opened> {
        let (commands, inbox) = mpsc::channel();
        let (loads, loaded) = mpsc::channel();
        thread::Builder::new()
            .name(THREAD.to_owned())
            .spawn(move || match load::load(&path) {
                Ok((connection, rows)) => {
                    if loads.send(Ok(rows)).is_ok() && write(&connection, &path, &inbox) {
                        drop(connection);
                        ended();
                    }
                }
                Err(error) => {
                    let _ = loads.send(Err(error));
                }
            })?;
        Ok(match loaded.recv_timeout(LOAD_WAIT) {
            Ok(Ok(rows)) => Opened::Loaded(Writer { commands }, rows),
            Ok(Err(error)) => Opened::InMemory(error),
            Err(RecvTimeoutError::Timeout) => {
                Opened::InMemory(format!("not loaded within {} s", LOAD_WAIT.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => Opened::InMemory(WRITER_ENDED.to_owned()),
        })
    }

    pub fn send(&self, change: Change) {
        let _ = self.commands.send(Command::Change(change));
    }

    /// Tells the writer to end once it has applied every change it was handed.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

/// Applies each command in order, and gives whether [`Writer::close`] ended it.
fn write(connection: &Connection, path: &Path, inbox: &Receiver<Command>) -> bool {
    for command in inbox {
        let applied = match command {
            Command::Close => return true,
            Command::Change(Change::Pin { id, used, seq }) => connection.execute(
                "UPDATE clips SET pinned = 1, used = ?2, seq = ?3 WHERE id = ?1",
                params![id.0, used.0, seq],
            ),
            Command::Change(Change::Unpin { id, used }) => connection.execute(
                "UPDATE clips SET pinned = 0, used = ?2 WHERE id = ?1",
                [id.0, used.0],
            ),
            Command::Change(Change::Delete { ids }) => delete(connection, &ids),
        };
        if let Err(error) = applied {
            warn(format_args!("{}", Unsaved::new(path, error)));
        }
    }
    false
}

fn delete(connection: &Connection, ids: &[ClipId]) -> rusqlite::Result<usize> {
    let transaction = connection.unchecked_transaction()?;
    let mut deleted = 0;
    {
        let mut statement = transaction.prepare("DELETE FROM clips WHERE id = ?1")?;
        for id in ids {
            deleted += statement.execute([id.0])?;
        }
    }
    transaction.commit()?;
    Ok(deleted)
}
