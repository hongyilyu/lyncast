//! Reads a file only when it is a regular file of at most a given size, after symlinks. The
//! scan reads desktop entries this way, and the daemon reads `ranking.json`.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Why [`read_regular`] read nothing.
pub enum Unread {
    /// A FIFO, a device, or a directory.
    NotRegular,
    /// Over the cap, by the file's size or by what the read found.
    TooLarge,
    Io(io::Error),
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
        return Err(Unread::TooLarge);
    }
    Ok(bytes)
}

/// Fails unless the file is a regular file of at most `max` bytes.
fn check(metadata: &Metadata, max: u64) -> Result<(), Unread> {
    if !metadata.is_file() {
        return Err(Unread::NotRegular);
    }
    if metadata.len() > max {
        return Err(Unread::TooLarge);
    }
    Ok(())
}
