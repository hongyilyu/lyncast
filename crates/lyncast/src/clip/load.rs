use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::clip::PREVIEW_BYTES;
use lyncast_core::rank::Timestamp;
use rusqlite::config::DbConfig;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};

use super::schema::SCHEMA;
use super::{Content, Meta, Row, Seq, no_id, no_seq};
use crate::files;

pub struct Loaded {
    pub rows: Vec<Row>,
    /// One above every id the file has used, deleted ones included.
    pub next_id: ClipId,
    pub next_seq: Seq,
}

/// A Wayland message gives its size in 16 bits, so no type that a capture reads is longer, and a
/// longer one is in a file the daemon did not write.
const MAX_TYPE: usize = 65_535;

const HASH_BYTES: usize = 64;

pub fn path(data_home: &Path) -> PathBuf {
    data_home.join("lyncast/clips.sqlite3")
}

/// Opens the file at `path`, takes its lock, and reads its rows. It writes nothing to the file,
/// so a file that it refuses, or whose load the start has stopped waiting for, stays as it was,
/// but for what SQLite does as it opens and closes the file. Beside a file in rollback mode that
/// is not empty, SQLite takes any file at the journal's path whose first byte is not 0 for a
/// crashed writer's journal, rolls back what it can read of it, and removes it, even when it is
/// no journal. Beside an empty file, it removes a journal or a `-wal` that holds anything. Beside
/// a file in WAL mode, an empty `-wal` goes as the connection closes, and one that holds anything
/// stays as it is.
pub fn load(path: &Path) -> Result<(Connection, Loaded), String> {
    if !files::regular_if_present(path).map_err(|unread| unread.to_string())? {
        create(path).map_err(|error| error.to_string())?;
    }
    let failed = |error: rusqlite::Error| error.to_string();
    let connection = open(path).map_err(failed)?;
    // SQLite copies a WAL's frames into the file as the connection closes.
    connection
        .set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .map_err(failed)?;
    lock(&connection).map_err(failed)?;
    // At the first read, SQLite creates an empty `-wal` beside a file in WAL mode, which only that
    // checkpoint removes, and the lock keeps every other connection from writing to it.
    if wal_is_empty(&connection) {
        connection
            .set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, false)
            .map_err(failed)?;
    }
    let pages: i64 = connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .map_err(failed)?;
    if pages == 0 {
        let loaded = Loaded {
            rows: Vec::new(),
            next_id: ClipId(1),
            next_seq: Seq::FIRST,
        };
        return Ok((connection, loaded));
    }
    let made = Connection::open_in_memory()
        .and_then(|made| made.execute_batch(SCHEMA).map(|()| made))
        .map_err(failed)?;
    let check = |found: rusqlite::Result<Option<String>>| match found {
        Ok(None) => Ok(()),
        Ok(Some(why)) => Err(why),
        Err(error) => Err(error.to_string()),
    };
    check(foreign_header(&connection, &made))?;
    check(foreign_schema(&connection, &made))?;
    check(foreign_count(&connection))?;
    check(foreign_row(&connection))?;
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
                        row.get(8)?,
                    ))
                })?
                .collect::<Result<Vec<Row>, _>>()
        })
        .map_err(failed)?;
    let used: i64 = connection
        .query_row(
            "SELECT max(coalesce((SELECT max(id) FROM clips), 0),
                coalesce((SELECT seq FROM sqlite_sequence), 0))",
            [],
            |row| row.get(0),
        )
        .map_err(failed)?;
    let next_id = used
        .checked_add(1)
        .ok_or_else(|| no_id(Some(ClipId(used))))?;
    let next_seq = match rows.iter().map(|row| row.seq).max() {
        Some(seq) => seq.next().ok_or_else(|| no_seq(Some(seq)))?,
        None => Seq::FIRST,
    };
    Ok((
        connection,
        Loaded {
            rows,
            next_id: ClipId(next_id),
            next_seq,
        },
    ))
}

/// rusqlite's default flags hold `SQLITE_OPEN_CREATE`, which would create a file that went after
/// the daemon's check, or the target of a symlink to nothing put there since.
fn open(path: &Path) -> rusqlite::Result<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    Connection::open_with_flags(path, flags)
}

/// Takes SQLite's exclusive lock, which stays until the connection closes, with a transaction
/// that changes nothing. A transaction on an empty file makes SQLite write a journal for its first
/// page, so on an empty file the journal stays in memory while the lock is taken.
fn lock(connection: &Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(Duration::ZERO)?;
    connection.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    if pages == 0 {
        connection.pragma_update(None, "journal_mode", "MEMORY")?;
    }
    connection.execute_batch("BEGIN EXCLUSIVE; ROLLBACK;")?;
    if pages == 0 {
        connection.pragma_update(None, "journal_mode", "DELETE")?;
    }
    Ok(())
}

/// SQLite keeps the `-wal` beside the file that symlinks lead to. rusqlite gives no path that is
/// not UTF-8, and then the `-wal` counts as one that holds frames.
fn wal_is_empty(connection: &Connection) -> bool {
    connection.path().is_some_and(|file| {
        fs::metadata(format!("{file}-wal")).map_or_else(
            |error| error.kind() == ErrorKind::NotFound,
            |wal| wal.len() == 0,
        )
    })
}

fn foreign_header(connection: &Connection, made: &Connection) -> rusqlite::Result<Option<String>> {
    let encoding = |db: &Connection| db.query_row("PRAGMA encoding", [], |row| row.get(0));
    let theirs: String = encoding(connection)?;
    if theirs != encoding(made)? {
        return Ok(Some(format!(
            "a text encoding of {theirs}, which the daemon does not write"
        )));
    }
    let application = |db: &Connection| db.query_row("PRAGMA application_id", [], |row| row.get(0));
    let theirs: i64 = application(connection)?;
    if theirs != application(made)? {
        return Ok(Some(format!(
            "an application id of {theirs}, which the daemon does not write"
        )));
    }
    Ok(None)
}

fn foreign_schema(connection: &Connection, made: &Connection) -> rusqlite::Result<Option<String>> {
    let objects: Vec<[String; 5]> = made
        .prepare(
            "SELECT type, name, tbl_name, CAST(rootpage AS TEXT), sql FROM sqlite_master
            ORDER BY rowid",
        )?
        .query_map([], |row| {
            Ok([
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ])
        })?
        .collect::<rusqlite::Result<_>>()?;
    let values = vec!["(?, ?, ?, ?, ?)"; objects.len()].join(", ");
    let foreign: Option<(String, Option<String>)> = connection
        .query_row(
            &format!(
                "WITH made (type, name, tbl_name, rootpage, sql) AS (VALUES {values})
                SELECT CASE type WHEN 'index' THEN 'an index' ELSE 'a ' || type END,
                    (SELECT 'a ' || name || ' ' || type FROM made
                        WHERE made.type = theirs.type AND made.name = theirs.name)
                FROM sqlite_master AS theirs WHERE NOT EXISTS (SELECT 1 FROM made
                    WHERE made.type IS theirs.type AND made.name IS theirs.name
                        AND made.tbl_name IS theirs.tbl_name
                        AND made.rootpage IS CAST(theirs.rootpage AS TEXT)
                        AND made.sql IS theirs.sql)
                ORDER BY theirs.rowid LIMIT 1"
            ),
            params_from_iter(objects.iter().flatten()),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((what, named)) = foreign {
        let what = named.unwrap_or(what);
        return Ok(Some(format!("{what} that the daemon does not make")));
    }
    let mut has = connection.prepare(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = ?1 AND name = ?2
            AND tbl_name = ?3 AND CAST(rootpage AS TEXT) = ?4 AND sql = ?5)",
    )?;
    for object in &objects {
        if !has.query_row(params_from_iter(object), |row| row.get::<_, bool>(0))? {
            let [what, name, ..] = object;
            return Ok(Some(format!("no {name} {what}")));
        }
    }
    Ok(None)
}

fn foreign_count(connection: &Connection) -> rusqlite::Result<Option<String>> {
    let rows: i64 =
        connection.query_row("SELECT count(*) FROM sqlite_sequence", [], |row| row.get(0))?;
    if rows > 1 {
        return Ok(Some("sqlite_sequence: more than one row".to_owned()));
    }
    // `octet_length` reads only a value's header, so a long name is never read.
    let count: Option<(bool, String)> = connection
        .query_row(
            "SELECT CASE WHEN typeof(name) = 'text' AND octet_length(name) = octet_length('clips')
                THEN name = 'clips' ELSE 0 END, typeof(seq) FROM sqlite_sequence",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match count {
        Some((false, _)) => Some("sqlite_sequence: a row not named clips".to_owned()),
        Some((true, count)) if count != "integer" => {
            Some("sqlite_sequence: a count of used ids that is not an integer".to_owned())
        }
        _ => None,
    })
}

/// Why a row of `clips` holds what the daemon does not write. `typeof` and `octet_length` read
/// only a value's header, and a kind or a hash is compared only once it is measured, so a long
/// value costs no memory. `GLOB` stops at a NUL byte, and `ltrim` reads every byte of the hash.
fn foreign_row(connection: &Connection) -> rusqlite::Result<Option<String>> {
    let kind_bytes = Content::ALL.map(|content| content.spelling().len());
    let kind_bytes = kind_bytes.into_iter().max().unwrap_or(0);
    let kinds = Content::ALL
        .map(|content| format!("'{}'", content.spelling()))
        .join(", ");
    let image = Content::Image.spelling();
    let row: Option<(i64, String)> = connection
        .query_row(
            &format!(
                "SELECT id, why FROM (SELECT id,
                CASE
                    WHEN id < 1 THEN 'an id below 1'
                    WHEN typeof(kind) <> 'text' THEN 'a kind that is not text'
                    WHEN octet_length(kind) > ?1 THEN 'a kind over ' || ?1 || ' bytes'
                    WHEN kind NOT IN ({kinds}) THEN 'an unknown kind'
                    WHEN typeof(mime) <> 'text' THEN 'a type that is not text'
                    WHEN octet_length(mime) > ?2 THEN 'a type over ' || ?2 || ' bytes'
                    WHEN typeof(bytes) <> 'integer' THEN 'a size that is not an integer'
                    WHEN bytes < 0 THEN 'a negative size'
                    WHEN kind = '{image}' AND typeof(body) <> 'null' THEN 'an image with a text'
                    WHEN kind <> '{image}' AND typeof(body) = 'null' THEN 'no text'
                    WHEN typeof(hash) <> 'text' THEN 'a hash that is not text'
                    WHEN octet_length(hash) <> ?3 THEN 'a hash that is not ' || ?3 || ' bytes'
                    WHEN ltrim(hash, '0123456789abcdef') <> '' THEN 'a hash that is not lowercase hex'
                    WHEN typeof(preview) NOT IN ('text', 'null') THEN 'a preview that is not text'
                    WHEN octet_length(preview) > ?4 THEN 'a preview over ' || ?4 || ' bytes'
                    WHEN kind = '{image}' AND typeof(preview) <> 'null' THEN 'an image with a preview'
                    WHEN kind <> '{image}' AND typeof(preview) = 'null' THEN 'no preview'
                    WHEN typeof(used) <> 'integer' THEN 'a last use that is not an integer'
                    WHEN typeof(pinned) <> 'integer' THEN 'a pin that is not an integer'
                    WHEN pinned NOT IN (0, 1) THEN 'a pin that is not 0 or 1'
                    WHEN typeof(seq) <> 'integer' THEN 'a sequence number that is not an integer'
                    WHEN seq < 0 THEN 'a negative sequence number'
                END AS why
            FROM clips) WHERE why IS NOT NULL LIMIT 1"
            ),
            params![
                kind_bytes as i64,
                MAX_TYPE as i64,
                HASH_BYTES as i64,
                PREVIEW_BYTES as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(row.map(|(id, why)| format!("clip {id}: {why}")))
}

/// SQLite gives its journal the file's mode.
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

fn unsigned(row: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(at)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(at, rusqlite::types::Type::Integer, error.into())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_open_creates_no_file() {
        let dir = std::env::current_exe()
            .unwrap()
            .with_file_name("clip-load-open");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("clips.sqlite3");
        std::os::unix::fs::symlink("missing.sqlite3", &file).unwrap();
        let opened = open(&file).map(drop);
        let missing = dir.join("missing.sqlite3").try_exists().unwrap();
        fs::remove_dir_all(&dir).unwrap();
        let unopened = format!("unable to open database file: {}", file.display());
        assert_eq!(opened.map_err(|error| error.to_string()), Err(unopened));
        assert!(!missing, "the open created the symlink's target");
    }
}
