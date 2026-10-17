//! Reads a file only when it is a regular file of at most a given size, after symlinks, and writes
//! files and directories that only their owner can read.

use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process;

/// Why [`read_regular`] read nothing.
pub enum Unread {
    /// A FIFO, a device, or a directory.
    NotRegular,
    /// Over the cap, by the file's size or by what the read found.
    TooLarge {
        max: u64,
    },
    Io(io::Error),
}

impl fmt::Display for Unread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unread::NotRegular => f.write_str("not a regular file"),
            Unread::TooLarge { max } => f.write_str(&over(*max)),
            Unread::Io(error) => error.fmt(f),
        }
    }
}

/// A change that did not reach the disk, and why. A warning spells it with its path, for a
/// write that failed or a clip refused because too much waits for the writer, and a reply spells
/// it without one, for a pin, an unpin, or a delete refused because the writer has no room or has
/// ended.
pub struct Unsaved {
    path: Option<PathBuf>,
    why: String,
}

impl Unsaved {
    pub fn new(path: &Path, why: impl ToString) -> Unsaved {
        Unsaved {
            path: Some(path.to_owned()),
            why: why.to_string(),
        }
    }

    pub fn refused(why: impl ToString) -> Unsaved {
        Unsaved {
            path: None,
            why: why.to_string(),
        }
    }
}

impl fmt::Display for Unsaved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.path {
            write!(f, "{}: ", path.display())?;
        }
        write!(f, "not saved: {}", self.why)
    }
}

/// Why a file of more than `max` bytes, a whole number of MiB, is not read or written.
pub fn over(max: u64) -> String {
    format!("over {} MiB", max >> 20)
}

/// The bytes of the file at `path`, when it is a regular file of at most `max` bytes after
/// symlinks. It checks the path first and never opens a FIFO, a device, or a file over `max`,
/// because opening a device can change it. Its open does not wait for a FIFO's writer, and it
/// checks the file it opened before it reads, so a FIFO or a device swapped in after the first
/// check is neither waited on nor read. The read stops after `max + 1` bytes, in case the file
/// grows meanwhile.
pub fn read_regular(path: &Path, max: u64) -> Result<Vec<u8>, Unread> {
    check(&fs::metadata(path).map_err(Unread::Io)?, max)?;
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(Unread::Io)?;
    check(&file.metadata().map_err(Unread::Io)?, max)?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(Unread::Io)?;
    if bytes.len() as u64 > max {
        return Err(Unread::TooLarge { max });
    }
    Ok(bytes)
}

/// Whether a regular file is at `path` after symlinks, `false` when nothing is there. Anything
/// else there fails, and a symlink to nothing is a file that cannot be read.
pub fn regular_if_present(path: &Path) -> Result<bool, Unread> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        return Ok(false);
    }
    if !fs::metadata(path).map_err(Unread::Io)?.is_file() {
        return Err(Unread::NotRegular);
    }
    Ok(true)
}

/// [`read_regular`], or `None` when nothing is at `path`, as [`regular_if_present`] finds it.
pub fn read_if_present(path: &Path, max: u64) -> Result<Option<Vec<u8>>, Unread> {
    if !regular_if_present(path)? {
        return Ok(None);
    }
    read_regular(path, max).map(Some)
}

/// Fails unless the file is a regular file of at most `max` bytes.
fn check(metadata: &Metadata, max: u64) -> Result<(), Unread> {
    if !metadata.is_file() {
        return Err(Unread::NotRegular);
    }
    if metadata.len() > max {
        return Err(Unread::TooLarge { max });
    }
    Ok(())
}

/// Creates `dir` and each parent it lacks with mode 0700, as the XDG Base Directory spec asks for
/// a data home. A directory that is already there keeps its mode.
pub fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Writes `bytes` to `path` through a temporary file beside it, `<file name>.<pid>.tmp`, which it
/// syncs and renames over `path`, so a crash leaves the old file or the new one. Two daemons with
/// their own runtime directories hold their own locks and can share a data home, so the name
/// carries the process ID, and neither writes into the other's file. A write that fails removes
/// the temporary file it created, because no later daemon has its name. The file gets mode 0600
/// and a missing directory mode 0700, through [`create_private_dir`].
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path.parent().expect("a data file is in a directory");
    create_private_dir(dir)?;
    let mut name = path.file_name().expect("a data file has a name").to_owned();
    name.push(format!(".{}.tmp", process::id()));
    let temporary = dir.join(name);
    let mut file = create_temporary(&temporary)?;
    let written = file
        .write_all(bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn create_temporary(path: &Path) -> io::Result<File> {
    let create = || {
        File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    };
    let replace_leftover_of_this_pid = || {
        fs::remove_file(path)?;
        create()
    };
    match create() {
        Err(error) if error.kind() == ErrorKind::AlreadyExists => replace_leftover_of_this_pid(),
        created => created,
    }
}
