//! The command that niri spawns for an app, which a worker builds, because finding the program,
//! the terminal, and the directory reads the disk. docs/design/launching.md gives the steps.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use lyncast_core::{AppId, Key};

use crate::apps;

/// The terminals that run a `Terminal=true` app when config.toml names none, in the order the
/// daemon tries them, each with the words that go before the app's argv.
const TERMINALS: [(&str, &[&str]); 9] = [
    ("xdg-terminal-exec", &[]),
    ("foot", &[]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("ghostty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xterm", &["-e"]),
];

/// The script that runs the argv after it in the directory before it, since niri's Spawn carries
/// no working directory. `--` keeps a directory whose name starts with `-` a directory.
const IN_DIRECTORY: &str = r#"cd -- "$1" && shift && exec "$@""#;

/// Where the daemon finds the programs it launches: `PATH` as it read it at start, and the
/// terminal that config.toml names.
pub struct Lookup {
    pub path: Option<OsString>,
    pub terminal: Option<Vec<String>>,
}

/// An app's item as the index holds it: its Exec argv or a Desktop Action's, with field codes
/// expanded, and the facts that decide how it runs.
pub struct Launch {
    pub app: AppId,
    pub argv: Vec<String>,
    pub terminal: bool,
    pub work_dir: Option<String>,
}

impl Lookup {
    /// The command that niri spawns for `launch`: its program found as the scan found it, after
    /// a terminal for a terminal app, all inside the `Path=` script when the app has a `Path`.
    /// The error is the reason the CLI prints after `lyncast: `.
    pub fn command(&self, launch: &Launch) -> Result<Vec<String>, String> {
        let (program, args) = launch
            .argv
            .split_first()
            .expect("an app's argv is never empty");
        let program = self
            .find(program, launch)?
            .ok_or_else(|| no_program(program))?;
        let mut command = vec![program];
        command.extend_from_slice(args);
        if launch.terminal {
            let mut terminal = self.terminal(launch)?;
            terminal.append(&mut command);
            command = terminal;
        }
        let Some(dir) = &launch.work_dir else {
            return Ok(command);
        };
        let dir = in_directory(dir)?;
        let script = ["/bin/sh", "-c", IN_DIRECTORY, "lyncast", &dir];
        Ok(script
            .iter()
            .map(|&word| word.to_owned())
            .chain(command)
            .collect())
    }

    /// The terminal's words: config.toml's, else the first of [`TERMINALS`] that is found. An
    /// empty list in config.toml counts as none.
    fn terminal(&self, launch: &Launch) -> Result<Vec<String>, String> {
        if let Some((program, args)) = self.terminal.as_deref().and_then(<[String]>::split_first) {
            let program = self
                .find(program, launch)?
                .ok_or_else(|| no_program(program))?;
            return Ok([program].into_iter().chain(args.iter().cloned()).collect());
        }
        for (program, args) in TERMINALS {
            if let Some(program) = self.find(program, launch)? {
                let args = args.iter().map(|&arg| arg.to_owned());
                return Ok([program].into_iter().chain(args).collect());
            }
        }
        Err(format!(
            "not run: no terminal for {}",
            Key::App(launch.app.clone())
        ))
    }

    /// The absolute path of `program`, as [`apps::find`] finds it for `launch`'s app. niri's
    /// Spawn takes text, so a path that is not UTF-8 fails.
    fn find(&self, program: &str, launch: &Launch) -> Result<Option<String>, String> {
        apps::find(program, self.path.as_deref(), launch.work_dir.as_deref())
            .map(text)
            .transpose()
    }
}

fn no_program(program: &str) -> String {
    format!("not run: no program {program}")
}

/// `dir` made absolute against the working directory, where the scan read it, once it is a
/// directory.
fn in_directory(dir: &str) -> Result<String, String> {
    let dir = std::path::absolute(dir).map_err(|error| format!("not run: {dir}: {error}"))?;
    match fs::metadata(&dir) {
        Ok(found) if found.is_dir() => text(dir),
        Ok(_) => Err(format!("not run: {}: not a directory", dir.display())),
        Err(error) => Err(format!("not run: {}: {error}", dir.display())),
    }
}

fn text(path: PathBuf) -> Result<String, String> {
    path.into_os_string()
        .into_string()
        .map_err(|path| format!("not run: {}: not UTF-8", Path::new(&path).display()))
}
