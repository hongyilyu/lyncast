use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::rank::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use super::blob::{self, Hash};
use super::load::{self, Loaded};
use super::schema::SCHEMA;
use super::{Body, Content, MAX_BYTES, Meta, Seq, WRITER_ENDED, no_clip, not_copied, not_recorded};
use crate::files::{self, Unread, Unsaved};
use crate::permit::{Permit, Permits};
use crate::warning::warn;

/// SQLite opens the file, and the journal beside it, by path after the daemon's check, and an
/// open can wait on a FIFO for good.
const LOAD_WAIT: Duration = Duration::from_secs(5);

/// The writer thread's name, by which the `freeze-thread` fixtures find it.
const THREAD: &str = "clips";

pub const MAX_PENDING: u64 = 64 << 20;

pub const MAX_CHANGES: u64 = 100;

const READ_WAIT: Duration = Duration::from_secs(5);

pub enum Change {
    /// A new row. The history holds no other row of its kind and hash, so the writer replaces any
    /// that the file holds, which only a delete that the file did not save leaves there.
    Insert {
        id: ClipId,
        meta: Meta,
        body: Body,
        at: Timestamp,
        seq: Seq,
        share: Permit,
    },
    Promote {
        id: ClipId,
        used: Timestamp,
        seq: Seq,
    },
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
    Change {
        change: Change,
        place: Permit,
    },
    Read {
        id: ClipId,
        body: Sender<Result<Body, String>>,
    },
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
    path: PathBuf,
    pending: Pending,
    changes: Permits,
}

/// The bytes of the entries on their way to the writer, from the capture thread on, which
/// [`MAX_PENDING`] bounds. An entry takes its share before it is handed over, and the share
/// goes back when it drops: once the writer has applied the entry's row, or once the entry goes
/// without one.
#[derive(Clone)]
pub struct Pending {
    bytes: Permits,
    path: PathBuf,
}

impl Pending {
    /// A share of `bytes`, or `None`, with a warning that the clip was not recorded, when it would
    /// bring what waits past [`MAX_PENDING`].
    pub fn take(&self, bytes: u64) -> Option<Permit> {
        let share = self.bytes.take(bytes);
        if share.is_none() {
            let full = format!("{} MiB of clips wait for the disk", MAX_PENDING >> 20);
            not_recorded(Unsaved::new(&self.path, full));
        }
        share
    }
}

pub struct Reader {
    commands: Sender<Command>,
}

impl Reader {
    /// What row `id` holds, or why it cannot be read, within [`READ_WAIT`].
    pub fn read(&self, id: ClipId) -> Result<Body, String> {
        let (body, read) = mpsc::channel();
        self.commands
            .send(Command::Read { id, body })
            .map_err(|_| not_copied(WRITER_ENDED))?;
        match read.recv_timeout(READ_WAIT) {
            Ok(body) => body,
            Err(RecvTimeoutError::Timeout) => Err(not_copied(format_args!(
                "the writer of the clips did not answer within {} s",
                READ_WAIT.as_secs()
            ))),
            Err(RecvTimeoutError::Disconnected) => Err(not_copied(WRITER_ENDED)),
        }
    }
}

impl Writer {
    /// Starts the thread that opens the file at `path`, loads it, and then applies each change,
    /// and waits [`LOAD_WAIT`] at most for the load. The thread calls `unsaved` with the id of each
    /// new row it could not save, and `ended` when it ends after [`Writer::close`], once it has
    /// closed the connection and so given up the lock. Fails only when the thread cannot start.
    /// The thread of a load that fails or runs over gets no change and calls neither.
    pub fn open(
        path: PathBuf,
        unsaved: impl Fn(ClipId) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Opened> {
        let (commands, inbox) = mpsc::channel();
        let (loads, loaded) = mpsc::channel();
        let file = path.clone();
        thread::Builder::new()
            .name(THREAD.to_owned())
            .spawn(move || match load::load(&file) {
                Ok((connection, rows)) => {
                    if loads.send(Ok(rows)).is_ok() && write(&connection, &file, &inbox, &unsaved) {
                        drop(connection);
                        ended();
                    }
                }
                Err(error) => {
                    let _ = loads.send(Err(error));
                }
            })?;
        Ok(match loaded.recv_timeout(LOAD_WAIT) {
            Ok(Ok(rows)) => Opened::Loaded(
                Writer {
                    commands,
                    pending: Pending {
                        bytes: Permits::new(MAX_PENDING),
                        path: path.clone(),
                    },
                    path,
                    changes: Permits::new(MAX_CHANGES),
                },
                rows,
            ),
            Ok(Err(error)) => Opened::InMemory(error),
            Err(RecvTimeoutError::Timeout) => {
                Opened::InMemory(format!("not loaded within {} s", LOAD_WAIT.as_secs()))
            }
            Err(RecvTimeoutError::Disconnected) => Opened::InMemory(WRITER_ENDED.to_owned()),
        })
    }

    pub fn pending(&self) -> &Pending {
        &self.pending
    }

    /// Fails, with why, when `changes` more changes would bring the changes that wait for the
    /// writer past [`MAX_CHANGES`].
    pub fn room(&self, changes: u64) -> Result<(), String> {
        if !self.changes.room(changes) {
            return Err(format!("{MAX_CHANGES} changes wait for the disk"));
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hands the writer a change that [`Writer::room`] has found room for. Only the owner loop
    /// sends changes, and only the writer gives their places back, so that room is still there.
    pub fn send(&self, change: Change) {
        let place = self
            .changes
            .take(1)
            .expect("Writer::room found room for the change");
        let _ = self.commands.send(Command::Change { change, place });
    }

    /// Tells the writer to end once it has applied every change it was handed.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }

    pub fn reader(&self) -> Reader {
        Reader {
            commands: self.commands.clone(),
        }
    }
}

/// Applies each command in order, and gives whether [`Writer::close`] ended it.
fn write(
    connection: &Connection,
    path: &Path,
    inbox: &Receiver<Command>,
    unsaved: &dyn Fn(ClipId),
) -> bool {
    let blobs = blob::dir(path);
    let mut table = Table::default();
    let mut unreserved = Unreserved::default();
    for command in inbox {
        match command {
            Command::Close => {
                unreserved.retry(connection, &mut table);
                return true;
            }
            Command::Read { id, body } => {
                let _ = body.send(read(connection, &blobs, id));
            }
            Command::Change { change, place } => {
                let inserted = match &change {
                    Change::Insert { id, .. } => Some(*id),
                    _ => None,
                };
                let applied = table
                    .make(connection)
                    .map_err(|error| Unsaved::new(path, error))
                    .and_then(|()| apply(connection, path, &blobs, change));
                if let Err(failed) = applied {
                    match inserted {
                        Some(id) => {
                            unreserved.raise(connection, &mut table, id);
                            not_recorded(&failed);
                            unsaved(id);
                        }
                        None => warn(format_args!("{failed}")),
                    }
                }
                drop(place);
            }
        }
    }
    false
}

/// Whether the file holds the table. The load creates no table, and takes a file that holds
/// anything only when it holds the table, so the writer creates the table, with the application
/// id, before its first write to an empty file.
#[derive(Default)]
struct Table(bool);

impl Table {
    fn make(&mut self, connection: &Connection) -> rusqlite::Result<()> {
        if self.0 {
            return Ok(());
        }
        let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        if pages == 0 {
            let transaction = connection.unchecked_transaction()?;
            transaction.execute_batch(SCHEMA)?;
            transaction.commit()?;
        }
        self.0 = true;
        Ok(())
    }
}

/// The newest id of a new row that the file did not save, while the file's count of used ids
/// could not be raised to it either. Ids only grow, so a raise to it covers every older one.
#[derive(Default)]
struct Unreserved(Option<ClipId>);

impl Unreserved {
    /// Raises the count to `id`, so no later daemon gives it again, and keeps it for one more try
    /// when that fails.
    fn raise(&mut self, connection: &Connection, table: &mut Table, id: ClipId) {
        self.0 = reserve(connection, table, id).err().map(|_| id);
    }

    /// The one more try, as the writer ends.
    fn retry(self, connection: &Connection, table: &mut Table) {
        if let Some(id) = self.0 {
            let _ = reserve(connection, table, id);
        }
    }
}

fn reserve(connection: &Connection, table: &mut Table, id: ClipId) -> rusqlite::Result<()> {
    table.make(connection)?;
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

fn apply(
    connection: &Connection,
    path: &Path,
    blobs: &Path,
    change: Change,
) -> Result<(), Unsaved> {
    let failed = |error: rusqlite::Error| Unsaved::new(path, error);
    match change {
        Change::Insert {
            id,
            meta,
            body,
            at,
            seq,
            share,
        } => {
            let bytes = i64::try_from(meta.bytes).map_err(|error| Unsaved::new(path, error))?;
            let text = match &body {
                Body::Text(text) => Some(&**text),
                Body::Image(image) => {
                    blob::write(blobs, meta.hash, image)
                        .map_err(|error| Unsaved::new(&blob::path(blobs, meta.hash), error))?;
                    None
                }
            };
            let inserted = connection
                .unchecked_transaction()
                .and_then(|transaction| {
                    transaction.execute(
                        "DELETE FROM clips WHERE kind = ?1 AND hash = ?2",
                        params![meta.content, meta.hash],
                    )?;
                    transaction.execute(
                        "INSERT INTO clips
                        (id, kind, mime, bytes, body, hash, preview, created, used, pinned, seq, source)
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 0, ?9, ?10)",
                        params![
                            id.0,
                            meta.content,
                            meta.mime,
                            bytes,
                            text,
                            meta.hash,
                            meta.preview,
                            at.0,
                            seq,
                            meta.source
                        ],
                    )?;
                    transaction.commit()
                })
                .map_err(failed);
            drop(share);
            if inserted.is_err() && matches!(body, Body::Image(_)) {
                let _ = remove_unnamed_blobs(connection, path, blobs, &[meta.hash]);
            }
            inserted?;
        }
        Change::Promote { id, used, seq } => {
            connection
                .execute(
                    "UPDATE clips SET used = ?2, seq = ?3 WHERE id = ?1",
                    params![id.0, used.0, seq],
                )
                .map_err(failed)?;
        }
        Change::Pin { id, used, seq } => {
            connection
                .execute(
                    "UPDATE clips SET pinned = 1, used = ?2, seq = ?3 WHERE id = ?1",
                    params![id.0, used.0, seq],
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
        Change::Delete { ids } => delete(connection, path, blobs, &ids)?,
    }
    Ok(())
}

fn delete(
    connection: &Connection,
    path: &Path,
    blobs: &Path,
    ids: &[ClipId],
) -> Result<(), Unsaved> {
    let failed = |error: rusqlite::Error| Unsaved::new(path, error);
    let transaction = connection.unchecked_transaction().map_err(failed)?;
    let mut images = Vec::new();
    {
        let mut statement = transaction
            .prepare("DELETE FROM clips WHERE id = ?1 RETURNING kind, hash")
            .map_err(failed)?;
        for id in ids {
            let deleted: Option<(Content, Hash)> = statement
                .query_row([id.0], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()
                .map_err(failed)?;
            if let Some((Content::Image, hash)) = deleted {
                images.push(hash);
            }
        }
    }
    transaction.commit().map_err(failed)?;
    remove_unnamed_blobs(connection, path, blobs, &images)
}

fn remove_unnamed_blobs(
    connection: &Connection,
    path: &Path,
    blobs: &Path,
    images: &[Hash],
) -> Result<(), Unsaved> {
    if images.is_empty() {
        return Ok(());
    }
    let named: HashSet<Hash> = connection
        .prepare("SELECT hash FROM clips WHERE kind = ?1")
        .and_then(|mut select| {
            select
                .query_map([Content::Image], |row| row.get(0))?
                .collect()
        })
        .map_err(|error| Unsaved::new(path, error))?;
    let mut removed = Ok(());
    for &hash in images.iter().filter(|hash| !named.contains(hash)) {
        let blob = blob::path(blobs, hash);
        if let Err(error) = fs::remove_file(&blob)
            && error.kind() != ErrorKind::NotFound
        {
            removed = removed.and(Err(Unsaved::new(&blob, error)));
        }
    }
    removed
}

/// The text is read only when octet_length, which reads none of it, finds it within the cap.
fn read(connection: &Connection, blobs: &Path, id: ClipId) -> Result<Body, String> {
    let row: Option<(Content, Option<String>, Option<i64>, Hash)> = connection
        .query_row(
            "SELECT kind, CASE WHEN octet_length(body) <= ?2 THEN body END, octet_length(body),
                hash FROM clips WHERE id = ?1",
            params![id.0, MAX_BYTES as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(not_copied)?;
    let failed = |reason: &str| format!("clip {}: {reason}", id.0);
    match row {
        None => Err(no_clip(id)),
        Some((Content::Image, _, _, hash)) => {
            match files::read_regular(&blob::path(blobs, hash), MAX_BYTES as u64) {
                Ok(bytes) => Ok(Body::Image(bytes.into())),
                Err(Unread::Io(error)) if error.kind() == ErrorKind::NotFound => {
                    Err(failed("blob missing"))
                }
                Err(unread) => Err(failed(&format!("blob: {unread}"))),
            }
        }
        Some((_, Some(text), _, _)) => Ok(Body::Text(text.into())),
        Some((_, None, Some(_), _)) => Err(failed(&files::over(MAX_BYTES as u64))),
        Some((_, None, None, _)) => Err(failed("no text")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Entry;

    #[test]
    fn a_blob_stays_until_the_delete_of_its_row_is_saved() {
        let blobs = std::env::current_exe()
            .unwrap()
            .with_file_name("clip-db-delete");
        let _ = fs::remove_dir_all(&blobs);
        fs::create_dir_all(&blobs).unwrap();
        let image = Hash::of(b"image");
        fs::write(blob::path(&blobs, image), b"image").unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO clips VALUES (1, 'image', 'image/png', 5, NULL, ?1, NULL, 0, 0, 0, 1, NULL)",
                [image],
            )
            .unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                CREATE TABLE held (clip INTEGER REFERENCES clips (id) DEFERRABLE INITIALLY DEFERRED);
                INSERT INTO held VALUES (1);",
            )
            .unwrap();
        let deleted = delete(
            &connection,
            Path::new("clips.sqlite3"),
            &blobs,
            &[ClipId(1)],
        );
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM clips", [], |row| row.get(0))
            .unwrap();
        let kept = blob::path(&blobs, image).try_exists().unwrap();
        fs::remove_dir_all(&blobs).unwrap();
        assert!(deleted.is_err(), "the delete was saved");
        assert_eq!(rows, 1);
        assert!(kept, "the blob went before the delete of its row was saved");
    }

    #[test]
    fn a_change_in_hand_keeps_its_place() {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("clip-db-in-hand");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (in_hand, reported) = mpsc::channel();
        let (finish, finished) = mpsc::channel::<()>();
        let (end, ended) = mpsc::channel();
        let opened = Writer::open(
            dir.join("clips.sqlite3"),
            move |_| {
                let _ = in_hand.send(());
                let _ = finished.recv();
            },
            move || {
                let _ = end.send(());
            },
        );
        let Ok(Opened::Loaded(writer, _)) = opened else {
            panic!("the writer did not load the file");
        };
        fs::create_dir(dir.join("clips.sqlite3-journal")).unwrap();
        let entry = Entry::text("alpha", "text/plain;charset=utf-8".to_owned());
        let share = writer.pending().take(entry.meta.bytes).unwrap();
        writer.room(1).unwrap();
        writer.send(Change::Insert {
            id: ClipId(1),
            meta: entry.meta,
            body: entry.body,
            at: Timestamp(0),
            seq: Seq::FIRST,
            share,
        });
        writer.close();
        let held = reported.recv();
        let beside = (writer.room(MAX_CHANGES - 1), writer.room(MAX_CHANGES));
        drop(finish);
        let closed = ended.recv();
        let after = writer.room(MAX_CHANGES);
        fs::remove_dir_all(&dir).unwrap();
        assert!(
            held.is_ok(),
            "the writer reported no row that it could not save"
        );
        assert_eq!(
            beside,
            (Ok(()), Err("100 changes wait for the disk".to_owned()))
        );
        assert!(closed.is_ok(), "the writer did not end");
        assert_eq!(after, Ok(()));
    }

    #[test]
    fn an_insert_that_is_not_saved_keeps_the_row_it_replaces() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        connection
            .execute(
                "INSERT INTO clips VALUES
                (1, 'text', 'text/plain;charset=utf-8', 5, 'alpha', ?1, 'alpha', 0, 0, 0, 1, NULL),
                (2, 'text', 'text/plain;charset=utf-8', 4, 'beta', ?2, 'beta', 0, 0, 0, 2, NULL)",
                [Hash::of(b"alpha"), Hash::of(b"beta")],
            )
            .unwrap();
        let shares = Permits::new(MAX_PENDING);
        let insert = |id| {
            let entry = Entry::text("alpha", "text/plain;charset=utf-8".to_owned());
            let share = shares.take(entry.meta.bytes).unwrap();
            apply(
                &connection,
                Path::new("clips.sqlite3"),
                Path::new("blobs"),
                Change::Insert {
                    id: ClipId(id),
                    meta: entry.meta,
                    body: entry.body,
                    at: Timestamp(0),
                    seq: Seq::FIRST,
                    share,
                },
            )
        };
        let ids = || -> Vec<i64> {
            connection
                .prepare("SELECT id FROM clips ORDER BY id")
                .and_then(|mut select| select.query_map([], |row| row.get(0))?.collect())
                .unwrap()
        };
        let unsaved = insert(2);
        let kept = ids();
        let saved = insert(3);
        assert!(unsaved.is_err(), "the insert on a used id was saved");
        assert_eq!(kept, [1, 2], "the old row went without the new one");
        assert!(saved.is_ok(), "the insert on a free id was not saved");
        assert_eq!(ids(), [2, 3]);
    }
}
