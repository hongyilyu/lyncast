//! `$XDG_DATA_HOME/lyncast/clips.sqlite3`, which only the writer thread touches. It opens the
//! file and loads every row's metadata, then applies each change in the order the daemon made it,
//! so the owner loop never waits on the disk. The connection holds SQLite's exclusive lock until
//! the writer ends, so a second daemon with the same data home cannot load or write the file.

use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::rank::Timestamp;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ValueRef};
use rusqlite::{Connection, params};

use super::{Content, Meta, Row};
use crate::files;
use crate::warning::warn;

/// The table. A text or a file list keeps its bytes in `body`, and an image's bytes live in the
/// blob that `hash`, the SHA-256 of what a row holds, names. `kind`, `hash`, and `preview` are
/// what the row derived from its bytes when they arrived. Times are milliseconds since the Unix
/// epoch. `seq` orders the rows, as `lyncast_core::clip::Place` says.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL CHECK (kind IN ('text', 'link', 'email', 'image', 'files')),
    mime TEXT NOT NULL,
    bytes INTEGER NOT NULL CHECK (bytes >= 0),
    body TEXT,
    hash TEXT NOT NULL,
    preview TEXT,
    created INTEGER NOT NULL,
    used INTEGER NOT NULL,
    pinned INTEGER NOT NULL CHECK (pinned IN (0, 1)),
    seq INTEGER NOT NULL CHECK (seq >= 0),
    CHECK ((kind = 'image') = (body IS NULL)),
    CHECK ((kind = 'image') = (preview IS NULL))
) STRICT;
";

/// How long the start waits for the writer to open the file and load it. SQLite opens the file,
/// and the journal beside it, by path after the daemon's check, and an open can wait on a FIFO
/// for good, so a load that has not ended by then leaves the rows in memory.
const LOAD_WAIT: Duration = Duration::from_secs(5);

/// What a load finds.
pub struct Loaded {
    pub rows: Vec<Row>,
    /// One above every row's sequence number.
    pub next_seq: u64,
}

/// The file's path under a data home.
pub fn path(data_home: &Path) -> PathBuf {
    data_home.join("lyncast/clips.sqlite3")
}

impl FromSql for Content {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Content> {
        Content::read(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

/// Opens the file at `path`, creating it when there is none, takes SQLite's exclusive lock, and
/// reads every row but its bytes. It checks the path first and opens only a regular file, after
/// symlinks, so a FIFO or a device there fails at once. A symlink to nothing is a file that cannot
/// be read, and so is one whose ids have run out, because it can take no new row.
fn load(path: &Path) -> Result<(Connection, Loaded), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            create(path).map_err(|error| error.to_string())?;
        }
        _ => match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => return Err("not a regular file".to_owned()),
            Err(error) => return Err(error.to_string()),
        },
    }
    let failed = |error: rusqlite::Error| error.to_string();
    let connection = Connection::open(path).map_err(failed)?;
    // A daemon that holds the lock holds it until it exits, so waiting for it helps nobody.
    connection.busy_timeout(Duration::ZERO).map_err(failed)?;
    // In exclusive mode the lock that the first write takes stays until the connection closes.
    connection
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .map_err(failed)?;
    connection
        .execute_batch(&format!("BEGIN EXCLUSIVE; {SCHEMA} COMMIT;"))
        .map_err(failed)?;
    let rows = connection
        .prepare("SELECT id, kind, mime, bytes, preview, used, pinned, seq FROM clips")
        .and_then(|mut select| {
            select
                .query_map([], |row| {
                    let meta = Meta {
                        content: row.get(1)?,
                        mime: row.get(2)?,
                        bytes: unsigned(row, 3)?,
                        preview: row.get(4)?,
                    };
                    let used = Timestamp(row.get(5)?);
                    Ok(Row::new(
                        ClipId(row.get(0)?),
                        meta,
                        used,
                        row.get(6)?,
                        unsigned(row, 7)?,
                    ))
                })?
                .collect::<Result<Vec<Row>, _>>()
        })
        .map_err(failed)?;
    let used: i64 = connection
        .query_row(
            "SELECT max(coalesce((SELECT max(id) FROM clips), 0),
                coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'clips'), 0))",
            [],
            |row| row.get(0),
        )
        .map_err(failed)?;
    if used.checked_add(1).is_none() {
        return Err(format!("no clip id is left after {used}"));
    }
    let next_seq = rows.iter().map(|row| row.seq + 1).max().unwrap_or(0);
    Ok((connection, Loaded { rows, next_seq }))
}

/// Creates the empty file at `path` with mode 0600, and its directory with mode 0700, because the
/// history holds whatever the user copied. SQLite gives its journal the file's mode. Another
/// daemon may create the file first, and then the load opens that one.
fn create(path: &Path) -> io::Result<()> {
    files::create_private_dir(path.parent().expect("clips.sqlite3 is in a directory"))?;
    match File::options()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Err(error) if error.kind() != ErrorKind::AlreadyExists => Err(error),
        _ => Ok(()),
    }
}

/// Column `at` of `row`, which the schema keeps at 0 or more.
fn unsigned(row: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(at)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Integer, error.into())
    })
}

/// A sequence number as SQLite stores it. None reaches `i64::MAX`.
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// One change to the file, in the order the daemon made it.
pub enum Change {
    /// A pin is a use, so it takes the row's place too.
    Pin {
        id: ClipId,
        used: Timestamp,
        seq: u64,
    },
    /// An unpin keeps the row's place and restarts its age.
    Unpin {
        id: ClipId,
        used: Timestamp,
    },
    Delete {
        id: ClipId,
    },
}

/// What the writer thread reads.
enum Command {
    Change(Change),
    /// The writer ends once it has applied every change before this one.
    Close,
}

/// The handle to the writer thread.
pub struct Writer {
    commands: Sender<Command>,
}

impl Writer {
    /// Starts the thread that opens the file at `path`, loads it, and then applies each change,
    /// and waits [`LOAD_WAIT`] at most for the load. The thread calls `ended` when it ends, after
    /// it has closed the connection and so given up the lock. Fails only when the thread cannot
    /// start, and gives why the rows stay in memory when the load fails or runs over. The thread
    /// of a load that runs over writes nothing once it ends.
    pub fn open(
        path: PathBuf,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Result<(Writer, Loaded), String>> {
        let (commands, inbox) = mpsc::channel();
        let (loads, loaded) = mpsc::channel();
        thread::Builder::new().spawn(move || {
            match load(&path) {
                Ok((connection, rows)) => {
                    if loads.send(Ok(rows)).is_ok() {
                        write(&connection, &path, &inbox);
                    }
                }
                Err(error) => {
                    let _ = loads.send(Err(error));
                }
            }
            ended();
        })?;
        Ok(match loaded.recv_timeout(LOAD_WAIT) {
            Ok(Ok(rows)) => Ok((Writer { commands }, rows)),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("not loaded within {} s", LOAD_WAIT.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => Err("the writer of the clips ended".to_owned()),
        })
    }

    /// Hands one change to the writer.
    pub fn send(&self, change: Change) {
        // The writer ends only after Close, which the daemon sends last.
        let _ = self.commands.send(Command::Change(change));
    }

    /// Tells the writer to end once it has applied every change it was handed.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

/// Applies each change until Close. A change that fails prints a warning and leaves the file as
/// it was, and the next change still runs.
fn write(connection: &Connection, path: &Path, inbox: &Receiver<Command>) {
    for command in inbox {
        let applied = match command {
            Command::Close => return,
            Command::Change(Change::Pin { id, used, seq }) => connection.execute(
                "UPDATE clips SET pinned = 1, used = ?2, seq = ?3 WHERE id = ?1",
                params![id.0, used.0, signed(seq)],
            ),
            Command::Change(Change::Unpin { id, used }) => connection.execute(
                "UPDATE clips SET pinned = 0, used = ?2 WHERE id = ?1",
                [id.0, used.0],
            ),
            Command::Change(Change::Delete { id }) => {
                connection.execute("DELETE FROM clips WHERE id = ?1", [id.0])
            }
        };
        if let Err(error) = applied {
            warn(format_args!("{}: not saved: {error}", path.display()));
        }
    }
}
