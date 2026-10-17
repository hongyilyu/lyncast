//! Image bytes in `$XDG_DATA_HOME/lyncast/blobs/`, one file per distinct image, named by the
//! SHA-256 of its bytes. Only the writer of the clips touches them: it writes a blob before the
//! row that names it, reads it for a copy, and removes it once no saved row names it.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process;

use sha2::{Digest, Sha256};

use crate::files;

/// The SHA-256 of what a clip holds. Two rows of one kind with the same hash hold the same bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hash([u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Hash {
        Hash(Sha256::digest(bytes).into())
    }

    /// The hash that 64 lowercase hex digits spell, as a blob's name and the database spell it.
    pub fn from_hex(text: &str) -> Option<Hash> {
        let digits = text.as_bytes();
        if digits.len() != 64
            || !digits
                .iter()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        let mut hash = [0; 32];
        for (byte, pair) in hash.iter_mut().zip(digits.chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
        }
        Some(Hash(hash))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// The directory of the blobs beside `clips.sqlite3`.
pub fn dir(database: &Path) -> PathBuf {
    database.with_file_name("blobs")
}

/// The blob of `hash` in `dir`.
pub fn path(dir: &Path, hash: Hash) -> PathBuf {
    dir.join(hash.to_string())
}

/// Writes `bytes` as the blob of `hash`, through a temporary file named with the process ID, so
/// another daemon never writes into it. A crash leaves the old blob or the new one, and a write
/// that fails removes its temporary file. The blob gets mode 0600 and a new `dir` mode 0700,
/// because an image may be anything the user copied.
pub fn write(dir: &Path, hash: Hash, bytes: &[u8]) -> io::Result<()> {
    files::create_private_dir(dir)?;
    let blob = path(dir, hash);
    let temporary = dir.join(format!("{hash}.{}.tmp", process::id()));
    let created = File::options()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary);
    let written = created.and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &blob)
    });
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}
