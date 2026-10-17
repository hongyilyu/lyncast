//! `$XDG_DATA_HOME/lyncast/clips.sqlite3` and `blobs/`, which only the writer thread touches. It
//! opens the file and loads every row's metadata, then applies each change in the order the
//! daemon made it, so the owner loop never waits on the disk. The connection holds SQLite's
//! exclusive lock until the writer ends, so a second daemon with the same data home cannot load
//! or write the file.

use std::fmt::Display;
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::rank::Timestamp;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use rusqlite::{Connection, OptionalExtension, params};

use super::blob::{self, Hash};
use super::{Body, Content, Meta, Row};
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

/// The most bytes of new rows that may wait for the writer.
pub const MAX_PENDING: u64 = 64 << 20;

/// What a load finds.
pub struct Loaded {
    pub rows: Vec<Row>,
    /// One above every id the file has used, deleted ones included.
    pub next_id: ClipId,
    /// One above every row's sequence number.
    pub next_seq: u64,
}

/// The file's path under a data home.
pub fn path(data_home: &Path) -> PathBuf {
    data_home.join("lyncast/clips.sqlite3")
}

impl ToSql for Content {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.spelling().into())
    }
}

impl FromSql for Content {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Content> {
        Content::read(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

impl ToSql for Hash {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.to_string().into())
    }
}

impl FromSql for Hash {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Hash> {
        Hash::from_hex(value.as_str()?).ok_or(FromSqlError::InvalidType)
    }
}

/// Opens the file at `path`, creating it when there is none, takes SQLite's exclusive lock, and
/// reads every row but its bytes. It checks the path first and opens only a regular file, after
/// symlinks, so a FIFO or a device there fails at once. A symlink to nothing is a file that cannot
/// be read, and so is one whose ids have run out.
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
        .prepare("SELECT id, kind, mime, bytes, hash, preview, used, pinned, seq FROM clips")
        .and_then(|mut select| {
            select
                .query_map([], |row| {
                    let meta = Meta {
                        content: row.get(1)?,
                        mime: row.get(2)?,
                        bytes: unsigned(row, 3)?,
                        hash: row.get(4)?,
                        preview: row.get(5)?,
                    };
                    let used = Timestamp(row.get(6)?);
                    Ok(Row::new(
                        ClipId(row.get(0)?),
                        meta,
                        used,
                        row.get(7)?,
                        unsigned(row, 8)?,
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
    let next_id = used
        .checked_add(1)
        .ok_or_else(|| format!("no clip id is left after {used}"))?;
    let next_seq = rows.iter().map(|row| row.seq + 1).max().unwrap_or(0);
    Ok((
        connection,
        Loaded {
            rows,
            next_id: ClipId(next_id),
            next_seq,
        },
    ))
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

/// A size or a sequence number as SQLite stores it. Neither reaches `i64::MAX`.
fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// One change to the file, in the order the daemon made it.
pub enum Change {
    /// A new row, with what it holds. An image's blob is written before the row.
    Insert {
        id: ClipId,
        meta: Meta,
        body: Body,
        at: Timestamp,
        seq: u64,
    },
    /// A capture or a copy of the same bytes moves the row to the front.
    Promote {
        id: ClipId,
        used: Timestamp,
        seq: u64,
    },
    /// A pin is a use, so it takes the row's place too.
    Pin {
        id: ClipId,
        used: Timestamp,
        seq: u64,
    },
    /// An unpin keeps the row's place and restarts its age.
    Unpin { id: ClipId, used: Timestamp },
    /// The row goes, and then an image's blob, once no saved row names it.
    Delete { id: ClipId },
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
    /// The bytes of the new rows handed over that the writer has not saved yet.
    pending: Arc<AtomicU64>,
    path: PathBuf,
}

impl Writer {
    /// Starts the thread that opens the file at `path`, loads it, and then applies each change,
    /// and waits [`LOAD_WAIT`] at most for the load. The thread calls `unsaved` with the id of each
    /// new row it could not save, and `ended` when it ends, after it has closed the connection and
    /// so given up the lock. Fails only when the thread cannot start, and gives why the rows stay
    /// in memory when the load fails or runs over. The thread of a load that runs over writes
    /// nothing once it ends.
    pub fn open(
        path: PathBuf,
        unsaved: impl Fn(ClipId) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Result<(Writer, Loaded), String>> {
        let (commands, inbox) = mpsc::channel();
        let (loads, loaded) = mpsc::channel();
        let pending = Arc::new(AtomicU64::new(0));
        let (file, saved) = (path.clone(), Arc::clone(&pending));
        thread::Builder::new().spawn(move || {
            match load(&file) {
                Ok((connection, rows)) => {
                    if loads.send(Ok(rows)).is_ok() {
                        write(&connection, &file, &inbox, &saved, &unsaved);
                    }
                }
                Err(error) => {
                    let _ = loads.send(Err(error));
                }
            }
            ended();
        })?;
        Ok(match loaded.recv_timeout(LOAD_WAIT) {
            Ok(Ok(rows)) => Ok((
                Writer {
                    commands,
                    pending,
                    path,
                },
                rows,
            )),
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => {
                Err(format!("not loaded within {} s", LOAD_WAIT.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => Err("the writer of the clips ended".to_owned()),
        })
    }

    /// The file the writer writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether a new row of `bytes` keeps what waits for the writer within [`MAX_PENDING`].
    pub fn has_room(&self, bytes: u64) -> bool {
        self.pending.load(Ordering::Relaxed).saturating_add(bytes) <= MAX_PENDING
    }

    /// Hands one change to the writer.
    pub fn send(&self, change: Change) {
        if let Change::Insert { meta, .. } = &change {
            self.pending.fetch_add(meta.bytes, Ordering::Relaxed);
        }
        // The writer ends only after Close, which the daemon sends last.
        let _ = self.commands.send(Command::Change(change));
    }

    /// Tells the writer to end once it has applied every change it was handed.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }
}

/// Applies each change until Close. A change that fails prints a warning and leaves the file as
/// it was, and the next change still runs. A new row that fails goes to `unsaved`, which takes it
/// out of the history, and its id stays used: the next change that the file saves raises the
/// file's count of used ids to it, so the next daemon does not give it either.
fn write(
    connection: &Connection,
    path: &Path,
    inbox: &Receiver<Command>,
    pending: &AtomicU64,
    unsaved: &dyn Fn(ClipId),
) {
    let blobs = blob::dir(path);
    // The id of the newest new row that failed, until the file's count holds it. Ids only grow,
    // so it is the largest.
    let mut reserved = None;
    for command in inbox {
        match command {
            Command::Close => return,
            Command::Change(change) => {
                let (inserted, bytes) = match &change {
                    Change::Insert { id, meta, .. } => (Some(*id), meta.bytes),
                    _ => (None, 0),
                };
                match apply(connection, path, &blobs, change) {
                    Ok(()) => {
                        if let Some(id) = reserved
                            && reserve(connection, id).is_ok()
                        {
                            reserved = None;
                        }
                    }
                    Err((path, error)) => match inserted {
                        Some(id) => {
                            warn(format_args!(
                                "{}: not saved: {error}; the clip was not recorded",
                                path.display()
                            ));
                            reserved = Some(id);
                            unsaved(id);
                        }
                        None => warn(format_args!("{}: not saved: {error}", path.display())),
                    },
                }
                pending.fetch_sub(bytes, Ordering::Relaxed);
            }
        }
    }
}

/// Raises the file's count of used ids, which AUTOINCREMENT keeps in `sqlite_sequence`, to `id`.
fn reserve(connection: &Connection, id: ClipId) -> rusqlite::Result<()> {
    let raised = connection.execute(
        "UPDATE sqlite_sequence SET seq = max(seq, ?1) WHERE name = 'clips'",
        [id.0],
    )?;
    if raised == 0 {
        connection.execute(
            "INSERT INTO sqlite_sequence (name, seq) VALUES ('clips', ?1)",
            [id.0],
        )?;
    }
    Ok(())
}

/// Applies one change, or gives the file it could not write and why.
fn apply(
    connection: &Connection,
    path: &Path,
    blobs: &Path,
    change: Change,
) -> Result<(), (PathBuf, Box<dyn Display>)> {
    let failed = |error: rusqlite::Error| (path.to_owned(), Box::new(error) as Box<dyn Display>);
    match change {
        Change::Insert {
            id,
            meta,
            body,
            at,
            seq,
        } => {
            let text = match &body {
                Body::Text(text) => Some(&**text),
                Body::Image(bytes) => {
                    blob::write(blobs, meta.hash, bytes).map_err(|error| {
                        (
                            blob::path(blobs, meta.hash),
                            Box::new(error) as Box<dyn Display>,
                        )
                    })?;
                    None
                }
            };
            connection
                .execute(
                    "INSERT INTO clips
                    (id, kind, mime, bytes, body, hash, preview, created, used, pinned, seq)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, ?9)",
                    params![
                        id.0,
                        meta.content,
                        meta.mime,
                        signed(meta.bytes),
                        text,
                        meta.hash,
                        meta.preview,
                        at.0,
                        signed(seq)
                    ],
                )
                .map_err(failed)?;
        }
        Change::Promote { id, used, seq } => {
            connection
                .execute(
                    "UPDATE clips SET used = ?2, seq = ?3 WHERE id = ?1",
                    params![id.0, used.0, signed(seq)],
                )
                .map_err(failed)?;
        }
        Change::Pin { id, used, seq } => {
            connection
                .execute(
                    "UPDATE clips SET pinned = 1, used = ?2, seq = ?3 WHERE id = ?1",
                    params![id.0, used.0, signed(seq)],
                )
                .map_err(failed)?;
        }
        Change::Unpin { id, used } => {
            connection
                .execute(
                    "UPDATE clips SET pinned = 0, used = ?2 WHERE id = ?1",
                    [id.0, used.0],
                )
                .map_err(failed)?;
        }
        Change::Delete { id } => {
            // The delete commits before the blob goes, so a crash between them leaves a blob
            // that no row names, never a row whose blob is gone.
            let deleted: Option<(Content, Hash)> = connection
                .query_row(
                    "DELETE FROM clips WHERE id = ?1 RETURNING kind, hash",
                    [id.0],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(failed)?;
            let Some((Content::Image, hash)) = deleted else {
                return Ok(());
            };
            let named: bool = connection
                .query_row(
                    "SELECT EXISTS (SELECT 1 FROM clips WHERE kind = ?1 AND hash = ?2)",
                    params![Content::Image, hash],
                    |row| row.get(0),
                )
                .map_err(failed)?;
            if named {
                return Ok(());
            }
            let blob = blob::path(blobs, hash);
            match fs::remove_file(&blob) {
                Err(error) if error.kind() != ErrorKind::NotFound => {
                    return Err((blob, Box::new(error)));
                }
                _ => {}
            }
        }
    }
    Ok(())
}
