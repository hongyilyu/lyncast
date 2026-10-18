use std::env;
use std::ffi::{OsStr, OsString};

/// The environment the daemon reads at start. The app scan reads all of it but
/// `XDG_CONFIG_HOME` and `WAYLAND_DISPLAY`. An empty variable counts as unset, except `PATH`. A
/// value that is not UTF-8 counts as unset, except in `XDG_DATA_DIRS`, `PATH`, and
/// `WAYLAND_DISPLAY`, and the entries of the first two count one by one. `WAYLAND_SOCKET` counts
/// as set whenever it is in the environment, even empty or not UTF-8, and has no field, since
/// [`Vars::read`] then fails. `Vars::default()` is an environment with nothing set.
#[derive(Default)]
pub struct Vars {
    /// The locale names to try, best first. `LANGUAGE` split on `:`, as GLib reads it, or else
    /// the first non-empty of `LC_ALL`, `LC_MESSAGES`, and `LANG`.
    pub locales: Vec<String>,
    pub current_desktop: String,
    pub data_home: Option<String>,
    pub data_dirs: Option<OsString>,
    pub config_home: Option<String>,
    pub home: Option<String>,
    /// As set, even when empty, because an empty `PATH` holds one empty entry and not the default.
    pub path: Option<OsString>,
    /// The compositor whose clipboard the daemon keeps. wl-clipboard-rs reads it again from the
    /// environment, which the daemon never changes, through wayland-client's `connect_to_env`,
    /// which takes the descriptor that `WAYLAND_SOCKET` names first, so [`Vars::read`] refuses it.
    pub wayland_display: Option<OsString>,
}

impl Vars {
    /// Fails when `WAYLAND_SOCKET` is set, since the daemon reaches its compositor only through
    /// `WAYLAND_DISPLAY`.
    pub fn read() -> Result<Vars, String> {
        if env::var_os("WAYLAND_SOCKET").is_some() {
            return Err(
                "WAYLAND_SOCKET is set; lyncast connects through WAYLAND_DISPLAY".to_owned(),
            );
        }
        let var = |name: &str| env::var(name).ok().filter(|value| !value.is_empty());
        let locales = match var("LANGUAGE") {
            Some(list) => list.split(':').map(str::to_owned).collect(),
            None => ["LC_ALL", "LC_MESSAGES", "LANG"]
                .into_iter()
                .find_map(var)
                .into_iter()
                .collect(),
        };
        Ok(Vars {
            locales,
            current_desktop: var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            data_home: var("XDG_DATA_HOME"),
            data_dirs: env::var_os("XDG_DATA_DIRS").filter(|value| !value.is_empty()),
            config_home: var("XDG_CONFIG_HOME"),
            home: var("HOME"),
            path: env::var_os("PATH"),
            wayland_display: env::var_os("WAYLAND_DISPLAY").filter(|value| !value.is_empty()),
        })
    }

    /// An absolute `XDG_DATA_HOME`, else `$HOME/.local/share` for an absolute `HOME`, as the
    /// XDG Base Directory spec asks.
    pub fn data_home(&self) -> Option<String> {
        self.base_dir(self.data_home.as_deref(), ".local/share")
    }

    /// Each `XDG_DATA_DIRS` entry, by default `/usr/local/share` and `/usr/share`. An entry that
    /// is relative or not UTF-8 is dropped.
    pub fn data_dirs(&self) -> Vec<String> {
        let dirs = self.data_dirs.as_deref();
        env::split_paths(dirs.unwrap_or(OsStr::new("/usr/local/share:/usr/share")))
            .filter_map(|dir| dir.into_os_string().into_string().ok())
            .filter(|dir| dir.starts_with('/'))
            .collect()
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
