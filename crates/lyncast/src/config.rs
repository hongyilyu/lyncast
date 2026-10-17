//! `$XDG_CONFIG_HOME/lyncast/config.toml`, which the daemon reads once, at start. Only the keys
//! below mean anything, and any other key is left alone.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use lyncast_core::clip::Retention;
use serde::Deserialize;

use crate::files::{self, Unread};
use crate::warning::warn;

/// What the daemon takes from the file.
pub struct Config {
    pub retention: Retention,
}

#[derive(Default, Deserialize)]
struct File {
    #[serde(default)]
    clipboard: Clipboard,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Clipboard {
    /// [`Retention::DEFAULT`] when the key is missing.
    #[serde(default)]
    retention_days: Option<u32>,
}

/// The most bytes the daemon reads from the file. A real one holds a few lines.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// The config under `config_home`. Without a file, or without a config home, every key takes
/// its default. A file that cannot be read or does not parse warns on stderr and gives the
/// defaults too.
pub fn read(config_home: Option<&Path>) -> Config {
    let file = config_home.map_or_else(File::default, |dir| {
        let path = dir.join("lyncast/config.toml");
        parse(&path).unwrap_or_else(|error| {
            warn(format_args!(
                "{}: {error}; using the defaults",
                path.display()
            ));
            File::default()
        })
    });
    Config {
        retention: file
            .clipboard
            .retention_days
            .map_or(Retention::DEFAULT, Retention::days),
    }
}

/// The file's keys. No file holds none, but a symlink to nothing is a file that cannot be read.
fn parse(path: &Path) -> Result<File, String> {
    if fs::symlink_metadata(path).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
        return Ok(File::default());
    }
    let bytes = files::read_regular(path, MAX_FILE_BYTES).map_err(|unread| match unread {
        Unread::NotRegular => "not a regular file".to_owned(),
        Unread::TooLarge => "over 1 MiB".to_owned(),
        Unread::Io(error) => error.to_string(),
    })?;
    let text = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    toml::from_str(&text).map_err(|error| match error.span() {
        Some(span) => format!(
            "{} at {}",
            error.message().trim_end(),
            place(&text, span.start)
        ),
        None => error.message().trim_end().to_owned(),
    })
}

/// The line and column of byte `at` in `text`, counted from 1, as serde_json counts them.
fn place(text: &str, at: usize) -> String {
    let before = text.get(..at).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("line {line} column {column}")
}
