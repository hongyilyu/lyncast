//! The IO half of the app index: the environment reads, the XDG directory walk, the file
//! reads, the first file per desktop-file ID, and the `TryExec` and Exec program checks.
//! Everything a file says is decided by `lyncast_core::desktop::parse`.

use std::collections::HashSet;
use std::env;
use std::ffi::{CString, OsStr, OsString};
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use lyncast_core::AppId;
use lyncast_core::desktop::{self, App, Session, Verdict};

/// The environment the daemon reads at start, which the scan reads all of but
/// `XDG_CONFIG_HOME`. An empty variable counts as unset, except `PATH`. A value that is not
/// UTF-8 counts as unset, except in `XDG_DATA_DIRS` and `PATH`, whose entries count one by one.
/// `Vars::default()` is an environment with nothing set.
#[derive(Default)]
pub struct Vars {
    /// The locale names to try, best first. `LANGUAGE` split on `:`, as GLib reads it, or else
    /// the first non-empty of `LC_ALL`, `LC_MESSAGES`, and `LANG`.
    locales: Vec<String>,
    current_desktop: String,
    data_home: Option<String>,
    data_dirs: Option<OsString>,
    config_home: Option<String>,
    home: Option<String>,
    /// As set, even when empty, because an empty `PATH` holds one empty entry and not the default.
    path: Option<OsString>,
}

impl Vars {
    pub fn read() -> Vars {
        let var = |name: &str| env::var(name).ok().filter(|value| !value.is_empty());
        let locales = match var("LANGUAGE") {
            Some(list) => list.split(':').map(str::to_owned).collect(),
            None => ["LC_ALL", "LC_MESSAGES", "LANG"]
                .into_iter()
                .find_map(var)
                .into_iter()
                .collect(),
        };
        Vars {
            locales,
            current_desktop: var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            data_home: var("XDG_DATA_HOME"),
            data_dirs: env::var_os("XDG_DATA_DIRS").filter(|value| !value.is_empty()),
            config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
            path: env::var_os("PATH"),
        }
    }

    /// An absolute `XDG_DATA_HOME`, else `$HOME/.local/share` for an absolute `HOME`, as the
    /// XDG Base Directory spec asks.
    pub fn data_home(&self) -> Option<String> {
        self.base_dir(self.data_home.as_deref(), ".local/share")
    }

    /// An absolute `XDG_CONFIG_HOME`, else `$HOME/.config` for an absolute `HOME`.
    pub fn config_home(&self) -> Option<String> {
        self.base_dir(self.config_home.as_deref(), ".config")
    }

    fn base_dir(&self, dir: Option<&str>, under_home: &str) -> Option<String> {
        let dir = dir.filter(|dir| dir.starts_with('/')).map(str::to_owned);
        dir.or_else(|| {
            let home = self.home.as_deref()?;
            home.starts_with('/')
                .then(|| format!("{}/{under_home}", home.trim_end_matches('/')))
        })
    }
}

/// Every app that shows. The first file for an ID decides for that ID before it is read, as
/// GLib masks an ID by its file name, so a later directory's file with the same ID is never
/// read. A byte that is not UTF-8 reads as U+FFFD.
pub fn scan(vars: &Vars) -> Vec<App> {
    let session = Session::new(&vars.locales, &vars.current_desktop);
    let mut decided = HashSet::new();
    let mut apps = Vec::new();
    for root in application_dirs(vars) {
        let mut found = Vec::new();
        walk(&root, "", &mut Vec::new(), &mut found);
        for file in found {
            if !decided.insert(file.id.clone()) {
                continue;
            }
            let Some(bytes) = read_entry(&file.path) else {
                continue;
            };
            // GLib loads a file with a bad byte and fails only that key, so one stray byte in
            // a translation must not hide the entry.
            let text = String::from_utf8_lossy(&bytes);
            let verdict = desktop::parse(AppId(file.id), &file.path, &text, &session);
            let Ok(Verdict::Shown(app)) = verdict else {
                continue;
            };
            let installed = |program: &str| on_path(program, vars.path.as_deref(), app.work_dir());
            if app.try_exec().is_none_or(installed) && installed(&app.argv()[0]) {
                apps.push(*app);
            }
        }
    }
    apps
}

/// The most bytes the scan reads from one desktop file. Real ones hold a few kilobytes.
const MAX_ENTRY_BYTES: u64 = 1 << 20;

/// The file's bytes, when it is a regular file after symlinks and holds at most
/// [`MAX_ENTRY_BYTES`]. A link to a device such as `/dev/zero` is not read, and a FIFO is not
/// opened.
fn read_entry(path: &str) -> Option<Vec<u8>> {
    if !fs::metadata(path).is_ok_and(|meta| meta.is_file()) {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_ENTRY_BYTES).then_some(bytes)
}

/// `$XDG_DATA_HOME/applications` (default `$HOME/.local/share`), then each `$XDG_DATA_DIRS`
/// entry's `applications` (default `/usr/local/share:/usr/share`). Relative paths and paths
/// that are not UTF-8 are ignored, and trailing slashes are dropped, as GLib's
/// `g_build_filename` drops them.
fn application_dirs(vars: &Vars) -> Vec<String> {
    let home = vars.data_home();
    let dirs = env::split_paths(
        vars.data_dirs
            .as_deref()
            .unwrap_or(OsStr::new("/usr/local/share:/usr/share")),
    )
    .filter_map(|dir| dir.into_os_string().into_string().ok())
    .filter(|dir| dir.starts_with('/'));
    home.into_iter()
        .chain(dirs)
        .map(|dir| format!("{}/applications", dir.trim_end_matches('/')))
        .collect()
}

/// One name that ends in `.desktop` under an `applications` directory, whatever it is.
struct Found {
    /// The path under `applications/` with `/` replaced by `-`.
    id: String,
    path: String,
}

/// Collects the names that end in `.desktop` under `dir` recursively, in name order. GLib
/// records such a name before it looks at the file, so a directory or a broken symlink with
/// that name still takes its ID. The walk follows symlinked directories and skips `dir` when
/// its canonical path is in `ancestors`, the directories it is inside, so a link cycle ends and
/// a directory reached first through a link is still walked under its own path. `prefix` is
/// the ID prefix of `dir`, such as `mozilla-`.
fn walk(dir: &str, prefix: &str, ancestors: &mut Vec<PathBuf>, found: &mut Vec<Found>) {
    let Ok(canonical) = fs::canonicalize(dir) else {
        return;
    };
    if ancestors.contains(&canonical) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    ancestors.push(canonical);
    let mut names: Vec<_> = entries
        .filter_map(|entry| Some(entry.ok()?.file_name()))
        .collect();
    names.sort();
    for name in names {
        // An ID and `%k` are text, so a name that is not UTF-8 names no entry.
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = format!("{dir}/{name}");
        if name.ends_with(".desktop") {
            found.push(Found {
                id: format!("{prefix}{name}"),
                path,
            });
        } else if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
            walk(&path, &format!("{prefix}{name}-"), ancestors, found);
        }
    }
    ancestors.pop();
}

/// `TryExec` or the Exec program: an absolute path must be an executable file, and a bare name
/// must be one in the entry's `Path` directory, when that is absolute, or in a `PATH` directory.
/// An unset `PATH` searches `/bin:/usr/bin:.`, and a relative entry, such as `.` or an empty one,
/// names a directory under `Path`, or under the working directory without one, as GLib does. A
/// relative path with a `/` is never found.
fn on_path(program: &str, path: Option<&OsStr>, work_dir: Option<&str>) -> bool {
    if program.starts_with('/') {
        return is_executable_file(Path::new(program));
    }
    if program.contains('/') {
        return false;
    }
    let base = Path::new(work_dir.unwrap_or(""));
    work_dir.is_some_and(|dir| dir.starts_with('/') && is_executable_file(&base.join(program)))
        || env::split_paths(path.unwrap_or(OsStr::new("/bin:/usr/bin:.")))
            .any(|dir| is_executable_file(&base.join(dir).join(program)))
}

/// A regular file, after symlinks, that this user may execute. GLib asks `access(X_OK)`, so a
/// file whose execute bit is only the owner's hides the entry from everyone else.
fn is_executable_file(path: &Path) -> bool {
    if !fs::metadata(path).is_ok_and(|meta| meta.is_file()) {
        return false;
    }
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `c_path` is a NUL-terminated string that lives through the call.
    unsafe { libc::access(c_path.as_ptr(), libc::X_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory variable that ends in `/` still gives the path GLib's `g_build_filename`
    /// gives, which `%k` expands to and Copy Path copies.
    #[test]
    fn a_trailing_slash_leaves_no_double_slash_in_an_app_path() {
        let tree = env::temp_dir().join(format!("lyncast-apps-test-{}", std::process::id()));
        let file = "[Desktop Entry]\nType=Application\nName=Kay\nExec=/bin/sh %k\n";
        for dir in [
            "data-home/applications",
            "home/.local/share/applications",
            "usr/applications",
        ] {
            fs::create_dir_all(tree.join(dir)).unwrap();
        }
        fs::write(tree.join("data-home/applications/data-home.desktop"), file).unwrap();
        fs::write(
            tree.join("home/.local/share/applications/home.desktop"),
            file,
        )
        .unwrap();
        fs::write(tree.join("usr/applications/usr.desktop"), file).unwrap();
        let root = tree.to_str().expect("a UTF-8 temp dir").to_owned();
        let vars = |data_home: Option<String>| Vars {
            locales: Vec::new(),
            current_desktop: String::new(),
            data_home,
            data_dirs: Some(format!("{root}/usr//").into()),
            config_home: None,
            home: Some(format!("{root}/home/")),
            path: None,
        };
        let paths = |vars: Vars| -> Vec<(String, Vec<String>)> {
            scan(&vars)
                .into_iter()
                .map(|app| (app.path().to_owned(), app.argv().to_vec()))
                .collect()
        };
        let with_data_home = paths(vars(Some(format!("{root}/data-home/"))));
        let with_home = paths(vars(None));
        fs::remove_dir_all(&tree).unwrap();
        let want = |file: &str| {
            let path = format!("{root}/{file}");
            (path.clone(), vec!["/bin/sh".to_owned(), path])
        };
        assert_eq!(
            with_data_home,
            [
                want("data-home/applications/data-home.desktop"),
                want("usr/applications/usr.desktop")
            ]
        );
        assert_eq!(
            with_home,
            [
                want("home/.local/share/applications/home.desktop"),
                want("usr/applications/usr.desktop")
            ]
        );
    }

    /// A file of 1 MiB is read. One a byte longer is not, and it still takes its ID, as a file
    /// that does not parse does.
    #[test]
    fn a_file_over_1_mib_is_not_read_and_still_takes_its_id() {
        let tree = env::temp_dir().join(format!("lyncast-apps-cap-{}", std::process::id()));
        let entry = |name: &str, bytes: usize| {
            let text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/sh\n");
            text.clone() + &"\n".repeat(bytes - text.len())
        };
        for dir in ["home/applications", "usr/applications"] {
            fs::create_dir_all(tree.join(dir)).unwrap();
        }
        let mib = 1 << 20;
        fs::write(
            tree.join("home/applications/big.desktop"),
            entry("Big Home", mib + 1),
        )
        .unwrap();
        fs::write(
            tree.join("usr/applications/big.desktop"),
            entry("Big System", 100),
        )
        .unwrap();
        fs::write(
            tree.join("usr/applications/edge.desktop"),
            entry("Edge", mib),
        )
        .unwrap();
        let root = tree.to_str().expect("a UTF-8 temp dir");
        let names: Vec<String> = scan(&Vars {
            locales: Vec::new(),
            current_desktop: String::new(),
            data_home: Some(format!("{root}/home")),
            data_dirs: Some(format!("{root}/usr").into()),
            config_home: None,
            home: None,
            path: None,
        })
        .iter()
        .map(|app| app.name().to_owned())
        .collect();
        fs::remove_dir_all(&tree).unwrap();
        assert_eq!(names, ["Edge"]);
    }
}
