//! The visits in `$XDG_DATA_HOME/lyncast/ranking.json`: one JSON object from key to visit, in
//! key order, so the bytes stay the same between writes of the same visits. One writer thread
//! writes the file, so the owner loop never waits on the disk. It takes the newest copy from one
//! slot, so a write that waits holds back one copy at most.

use std::collections::BTreeMap;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use lyncast_core::Key;
use lyncast_core::rank::{Timestamp, Visit, Visits};
use serde::{Deserialize, Serialize};

use crate::files::{self, Unread};
use crate::warning::warn;

/// One member of the file.
#[derive(Serialize, Deserialize)]
struct Stored {
    anchor: Millis,
    last_opened: Millis,
    terms: Vec<String>,
}

/// Milliseconds since the Unix epoch, from 1970 to the end of 9999. No clock gives a time
/// outside that range, so one fails the file as a bad file.
#[derive(Serialize, Deserialize)]
#[serde(try_from = "i64")]
struct Millis(i64);

/// 9999-12-31T23:59:59.999Z.
const LAST: i64 = 253_402_300_799_999;

impl TryFrom<i64> for Millis {
    type Error = String;

    fn try_from(millis: i64) -> Result<Millis, String> {
        if (0..=LAST).contains(&millis) {
            Ok(Millis(millis))
        } else {
            Err(format!("{millis} is not a time from 1970 to 9999"))
        }
    }
}

impl Millis {
    /// The nearest time in range, so the daemon reads back every time it writes. A clock near
    /// the end of 9999 can push an anchor past it.
    fn nearest(time: Timestamp) -> Millis {
        Millis(time.0.clamp(0, LAST))
    }
}

/// The most bytes the file may hold, as the daemon reads it or writes it. Real ones hold a few
/// kilobytes.
const MAX_FILE_BYTES: u64 = 64 << 20;

/// The error for a file over [`MAX_FILE_BYTES`], read or written.
fn too_large() -> String {
    format!("over {} MiB", MAX_FILE_BYTES >> 20)
}

/// The handle to the writer thread.
pub struct Store {
    shared: Arc<Shared>,
}

/// What the owner loop and the writer share. Each holds the lock only to read or swap a field.
struct Shared {
    slot: Mutex<Slot>,
    /// Wakes the writer when the slot changes.
    wake: Condvar,
}

#[derive(Default)]
struct Slot {
    /// The newest serialized visits that the writer has not taken.
    bytes: Option<Vec<u8>>,
    /// The writer ends once it has written what the slot holds.
    closed: bool,
}

impl Store {
    /// Loads the visits under an absolute data home and starts the writer, which calls `ended`
    /// when it ends, or fails when the writer's thread cannot start. Without a file there are
    /// none. A file that cannot be read, is not JSON of this shape, or holds a time out of range
    /// is moved to `ranking.json.bad`, with a warning on stderr, and gives none. When
    /// `ranking.json.bad` is already there, or the move fails, there is no store, so neither
    /// file is ever written over. A member whose key no longer parses is dropped.
    pub fn open(
        data_home: &Path,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<(Visits, Option<Store>)> {
        let path = data_home.join("lyncast/ranking.json");
        let stored = match read(&path) {
            Ok(stored) => stored,
            Err(error) => {
                let bad = path.with_extension("json.bad");
                let moved = move_aside(&path, &bad).map_err(|error| match error.kind() {
                    ErrorKind::AlreadyExists => "it already exists".to_owned(),
                    _ => error.to_string(),
                });
                if let Err(moved) = moved {
                    let why = format!(
                        "{error}; not moved to ranking.json.bad: {moved}; the visits stay in memory"
                    );
                    warn(format_args!("{}: {why}", path.display()));
                    return Ok((Visits::default(), None));
                }
                warn(format_args!(
                    "{}: {error}; moved it to ranking.json.bad",
                    path.display()
                ));
                BTreeMap::new()
            }
        };
        let mut visits = Visits::default();
        for (key, stored) in stored {
            let Ok(key) = key.parse::<Key>() else {
                continue;
            };
            let terms = stored.terms.iter().map(String::as_str);
            let visit = Visit::restore(
                Timestamp(stored.anchor.0),
                Timestamp(stored.last_opened.0),
                terms,
            );
            visits.insert(key, visit);
        }
        Ok((visits, Some(Store::start(path, ended)?)))
    }

    /// Starts the thread that writes the file at `path`.
    fn start(path: PathBuf, ended: impl FnOnce() + Send + 'static) -> io::Result<Store> {
        let shared = Arc::new(Shared {
            slot: Mutex::default(),
            wake: Condvar::new(),
        });
        let writer = Arc::clone(&shared);
        thread::Builder::new().spawn(move || {
            while let Some(bytes) = writer.next() {
                if let Err(error) = write(&path, &bytes) {
                    warn(format_args!("{}: not saved: {error}", path.display()));
                }
            }
            ended();
        })?;
        Ok(Store { shared })
    }

    /// Hands every visit to the writer.
    pub fn save(&self, visits: &Visits) {
        let stored: BTreeMap<String, Stored> = visits
            .iter()
            .map(|(key, visit)| {
                let stored = Stored {
                    anchor: Millis::nearest(visit.anchor()),
                    last_opened: Millis::nearest(visit.last_opened()),
                    terms: visit.terms().iter().map(|term| term.to_string()).collect(),
                };
                (key.to_string(), stored)
            })
            .collect();
        let mut bytes = serde_json::to_vec(&stored).expect("visits serialize");
        bytes.push(b'\n');
        self.shared.slot().bytes = Some(bytes);
        self.shared.wake.notify_one();
    }

    /// Tells the writer to end once it has written what it was handed.
    pub fn close(&self) {
        self.shared.slot().closed = true;
        self.shared.wake.notify_one();
    }
}

impl Shared {
    fn slot(&self) -> MutexGuard<'_, Slot> {
        // A slot holds plain values, so a panic while it was locked leaves it whole.
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The newest bytes to write, once there are some, or `None` once the store is closed and
    /// every copy is written.
    fn next(&self) -> Option<Vec<u8>> {
        self.wake
            .wait_while(self.slot(), |slot| slot.bytes.is_none() && !slot.closed)
            .unwrap_or_else(PoisonError::into_inner)
            .bytes
            .take()
    }
}

/// The file's members. Without a file there are none, but a symlink to nothing is a file that
/// cannot be read. Only a regular file of at most [`MAX_FILE_BYTES`] is read, after symlinks.
fn read(path: &Path) -> Result<BTreeMap<String, Stored>, String> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        return Ok(BTreeMap::new());
    }
    let bytes = files::read_regular(path, MAX_FILE_BYTES).map_err(|unread| match unread {
        Unread::NotRegular => "not a regular file".to_owned(),
        Unread::TooLarge => too_large(),
        Unread::Io(error) => error.to_string(),
    })?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

/// Writes `bytes` to a temporary file in the same directory and renames it over the file, so a
/// crash leaves the old file or the new one. Two daemons with their own runtime directories hold
/// their own locks and can share a data home, so the temporary name carries the process ID, and
/// neither writes into the other's file. Bytes over [`MAX_FILE_BYTES`] are not written, because
/// the daemon would not read them back.
fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other(too_large()));
    }
    files::create_private_dir(path.parent().expect("ranking.json is in a directory"))?;
    let temporary = path.with_extension(format!("json.{}.tmp", process::id()));
    let written = File::create(&temporary).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    });
    // No later daemon has this process's name, so a failed write removes its own file.
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Renames `path` to `bad` in one step that fails with `AlreadyExists` rather than replace `bad`,
/// so a daemon with another runtime directory and the same data home can neither replace a bad
/// file that one kept nor lose a file that one has just written. A filesystem that cannot rename
/// without replacing fails the move, and the caller keeps the visits in memory.
fn move_aside(path: &Path, bad: &Path) -> io::Result<()> {
    let from = CString::new(path.as_os_str().as_bytes())?;
    let to = CString::new(bad.as_os_str().as_bytes())?;
    // SAFETY: both pointers are NUL-terminated strings that live through the call.
    let renamed = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if renamed == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
