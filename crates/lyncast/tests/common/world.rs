//! One block's world: its tree, its runtime directory, and its environment, and its `$ ...`
//! lines as commands in that world.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::tree::{environment, materialize, raw_bytes, substitute};
use super::{LYNCAST, Runtime, doc, umask};

/// One `$ ...` command line of a doc.
pub struct CommandLine {
    /// The `env -u NAME` variables, removed from the env block.
    pub unset: Vec<String>,
    /// The `NAME=value` words, which override the env block after the removals.
    pub vars: Vec<(String, OsString)>,
    pub args: Vec<String>,
}

impl CommandLine {
    pub fn var(&self, name: &str) -> Option<&OsStr> {
        self.vars
            .iter()
            .find(|(set, _)| set == name)
            .map(|(_, value)| value.as_os_str())
    }

    /// `lyncast` with `args` and only `env`'s variables, as this line changes them.
    pub fn command<S: AsRef<OsStr>>(
        &self,
        args: impl IntoIterator<Item = S>,
        env: &[(String, String)],
    ) -> Command {
        let mut command = Command::new(LYNCAST);
        command
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::null());
        umask(&mut command);
        // env(1) unsets before it sets, so `env -u PATH PATH=` runs with an empty `PATH`.
        for name in &self.unset {
            command.env_remove(name);
        }
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
    }
}

/// A `$ [env -u NAME ...] [NAME=value ...] lyncast ...` line, or `None` for a line that runs
/// another program. A value may hold each `$NAME` in `paths` and `\x{HH}`, and `""` is an empty
/// argument.
pub fn command_line(command: &str, paths: &[(&str, &Path)]) -> Option<CommandLine> {
    let mut words: &[&str] = &command
        .strip_prefix("$ ")
        .unwrap_or_else(|| panic!("a command line starts with `$ `: {command:?}"))
        .split(' ')
        .collect::<Vec<_>>();
    let mut unset = Vec::new();
    if let ["env", rest @ ..] = words {
        words = rest;
        while let ["-u", name, rest @ ..] = words {
            unset.push((*name).to_owned());
            words = rest;
        }
    }
    let program = words.iter().position(|word| !word.contains('='))?;
    if words[program] != "lyncast" {
        return None;
    }
    let vars = words[..program]
        .iter()
        .map(|word| {
            let (name, value) = word.split_once('=').expect("a word with =");
            let value = OsString::from_vec(raw_bytes(&substitute(value, paths)));
            (name.to_owned(), value)
        })
        .collect();
    let args = words[program + 1..]
        .iter()
        .map(|word| {
            let arg = doc::unquote(word);
            assert!(
                !arg.contains('"'),
                "a quoted argument with a space: {command:?}"
            );
            arg.to_owned()
        })
        .collect();
    Some(CommandLine { unset, vars, args })
}

/// One block's tree, runtime directory, and environment.
pub struct World {
    pub tree: PathBuf,
    pub runtime: Runtime,
    pub env: Vec<(String, String)>,
    /// Where each run's stdout and stderr go, beside the tree.
    pub out: PathBuf,
}

impl World {
    /// A fresh tree at `tree`, with a fresh runtime directory.
    pub fn new(fixtures: &[doc::Block], tree: PathBuf) -> World {
        World::within(fixtures, tree, Runtime::new(), &[])
    }

    /// A fresh tree at `tree`, with a runtime directory that holds a Tier 2 compositor of its
    /// own, whose `WAYLAND_DISPLAY` joins the env block's variables.
    pub fn compositor(fixtures: &[doc::Block], tree: PathBuf) -> World {
        let (runtime, display) = Runtime::compositor();
        let wayland = [("WAYLAND_DISPLAY".to_owned(), display)];
        World::within(fixtures, tree, runtime, &wayland)
    }

    /// A fresh tree at `tree` with `runtime`, and `more` variables beside the `env` block's. The
    /// tree's path is canonical, as the working directory of a command in it reads, so a path that
    /// the daemon makes absolute against its working directory reads as `$TREE`.
    fn within(
        fixtures: &[doc::Block],
        tree: PathBuf,
        runtime: Runtime,
        more: &[(String, String)],
    ) -> World {
        materialize(fixtures, &tree);
        let tree = fs::canonicalize(&tree).unwrap();
        let mut env = environment(fixtures, &[("$TREE", &tree), ("$RUNTIME", runtime.path())]);
        env.extend_from_slice(more);
        let out = tree.with_extension("out");
        fs::create_dir_all(&out).unwrap();
        World {
            tree,
            runtime,
            env,
            out,
        }
    }

    /// A world on a fresh tree named `name` under `CARGO_TARGET_TMPDIR`, for a test that no
    /// block drives.
    pub fn scratch(fixtures: &[doc::Block], name: &str) -> World {
        World::new(fixtures, Path::new(env!("CARGO_TARGET_TMPDIR")).join(name))
    }

    /// Removes the tree and the output beside it once the run in them passes, so a run that
    /// fails keeps them to look at.
    pub fn passed(self) {
        fs::remove_dir_all(&self.out).unwrap();
        fs::remove_dir_all(&self.tree).unwrap();
    }

    /// `text` with `$TREE` and `$RUNTIME` replaced.
    pub fn substitute(&self, text: &str) -> String {
        substitute(
            text,
            &[("$TREE", &self.tree), ("$RUNTIME", self.runtime.path())],
        )
    }

    /// The `$ ...` line as a command in this world, and the line itself when it runs lyncast.
    /// Any other line runs through `/bin/sh`.
    pub fn command(&self, line: &str) -> (Command, Option<CommandLine>) {
        let paths = [
            ("$TREE", self.tree.as_path()),
            ("$RUNTIME", self.runtime.path()),
        ];
        let Some(lyncast) = command_line(line, &paths) else {
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(&line[2..])
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("TREE", &self.tree)
                .env("RUNTIME", self.runtime.path())
                .env("LYNCAST", LYNCAST)
                .current_dir(&self.tree);
            umask(&mut command);
            return (command, None);
        };
        let mut command = lyncast.command(&lyncast.args, &self.env);
        command.current_dir(&self.tree);
        (command, Some(lyncast))
    }

    /// The bytes the recorder file holds now. A recorder that is not a file holds none.
    pub fn recorded(&self) -> Vec<u8> {
        fs::read(self.runtime.runs()).unwrap_or_default()
    }

    /// The request lines that the fake niri has read so far.
    pub fn niri_read(&self) -> Vec<u8> {
        fs::read(self.runtime.niri_log()).unwrap_or_default()
    }
}
