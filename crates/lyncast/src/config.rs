use std::path::{Path, PathBuf};

use lyncast_core::clip::Retention;
use serde::Deserialize;

use crate::files;
use crate::warning::warn;

pub struct Config {
    pub retention: Retention,
    /// The terminal that runs `Terminal=true` apps, its program first.
    pub terminal: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
struct File {
    #[serde(default)]
    clipboard: Clipboard,
    #[serde(default)]
    launch: Launch,
}

#[derive(Default, Deserialize)]
struct Launch {
    #[serde(default)]
    terminal: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Clipboard {
    #[serde(default)]
    retention_days: Option<u32>,
}

const MAX_FILE_BYTES: u64 = 1 << 20;

/// The config file under `config_home`, which Open Config creates and opens.
pub fn path(config_home: &Path) -> PathBuf {
    config_home.join("lyncast/config.toml")
}

/// The config under `config_home`. Without a file, or without a config home, every key takes
/// its default. A file that cannot be read or does not parse warns on stderr and gives the
/// defaults too.
pub fn read(config_home: Option<&Path>) -> Config {
    let file = config_home.map_or_else(File::default, |dir| {
        let path = path(dir);
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
        terminal: file.launch.terminal,
    }
}

fn parse(path: &Path) -> Result<File, String> {
    let read = files::read_if_present(path, MAX_FILE_BYTES).map_err(|unread| unread.to_string())?;
    let Some(bytes) = read else {
        return Ok(File::default());
    };
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

fn place(text: &str, at: usize) -> String {
    let before = text.get(..at).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("line {line} column {column}")
}
