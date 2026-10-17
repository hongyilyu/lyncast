use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

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

pub fn dir(database: &Path) -> PathBuf {
    database.with_file_name("blobs")
}

pub fn path(dir: &Path, hash: Hash) -> PathBuf {
    dir.join(hash.to_string())
}

pub fn write(dir: &Path, hash: Hash, bytes: &[u8]) -> io::Result<()> {
    files::write_private(&path(dir, hash), bytes)
}
