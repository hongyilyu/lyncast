//! Desktop entries. [`parse`] turns one `.desktop` file's text into the verdict for its ID: an
//! [`App`] that the index ranks and step 8 launches, or the reason the ID shows nothing. The
//! binary walks the directories, reads the files, keeps the first file per ID, and
//! looks up `TryExec` and the Exec program. The rules and their fixtures are in
//! docs/design/ranking.md.

mod exec;
mod keyfile;

pub use exec::{ExecContext, ExecError, tokenize};
pub use keyfile::Unparseable;

use crate::{ActionItem, AppId, DesktopAction, Subject};
use keyfile::KeyFile;

/// The session's locale and desktops, as the binary read them from the environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// `Key[...]` suffixes to try, best first. Empty for `C`, `POSIX`, or no locale.
    locales: Vec<String>,
    desktops: Vec<String>,
}

impl Session {
    /// `locales` are the locale names the binary read, best first, each in the form
    /// `lang_COUNTRY.ENCODING@MODIFIER`. Each name gives its candidates in turn, and a candidate
    /// an earlier name gave is not repeated. `current_desktop` is `XDG_CURRENT_DESKTOP`. Either
    /// may be empty.
    pub fn new(locales: &[impl AsRef<str>], current_desktop: &str) -> Session {
        let mut candidates: Vec<String> = Vec::new();
        for candidate in locales
            .iter()
            .flat_map(|name| locale_candidates(name.as_ref()))
        {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        Session {
            locales: candidates,
            desktops: current_desktop
                .split(':')
                .filter(|desktop| !desktop.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }
}

/// `sr_RS.UTF-8@latin` gives `sr_RS@latin`, `sr_RS`, `sr@latin`, and `sr`, in the spec's
/// order. The encoding is dropped.
fn locale_candidates(locale: &str) -> Vec<String> {
    let (rest, modifier) = match locale.split_once('@') {
        Some((rest, modifier)) => (rest, Some(modifier).filter(|m| !m.is_empty())),
        None => (locale, None),
    };
    let rest = rest.split_once('.').map_or(rest, |(rest, _)| rest);
    let (lang, country) = match rest.split_once('_') {
        Some((lang, country)) => (lang, Some(country).filter(|c| !c.is_empty())),
        None => (rest, None),
    };
    if matches!(lang, "" | "C" | "POSIX") {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let (Some(country), Some(modifier)) = (country, modifier) {
        candidates.push(format!("{lang}_{country}@{modifier}"));
    }
    if let Some(country) = country {
        candidates.push(format!("{lang}_{country}"));
    }
    if let Some(modifier) = modifier {
        candidates.push(format!("{lang}@{modifier}"));
    }
    candidates.push(lang.to_owned());
    candidates
}

/// An application to rank and launch. Only [`parse`] builds one, so its argv is always
/// tokenized and every action has a name and an argv.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    id: AppId,
    path: String,
    name: String,
    unlocalized_name: String,
    generic_name: Option<String>,
    keywords: Vec<String>,
    icon: Option<String>,
    argv: Vec<String>,
    terminal: bool,
    try_exec: Option<String>,
    work_dir: Option<String>,
    actions: Vec<AppAction>,
}

/// One Desktop Action with a Name and an Exec that tokenizes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppAction {
    pub action: DesktopAction,
    pub argv: Vec<String>,
}

impl App {
    /// The desktop-file ID, such as `mozilla-firefox.desktop`.
    pub fn id(&self) -> &AppId {
        &self.id
    }

    /// The file the entry came from. `%k` expands to it, and Copy Path copies it.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `Name` in the session locale. The row's title.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `Name` without a locale, or the localized one when the file has no plain `Name`.
    pub fn unlocalized_name(&self) -> &str {
        &self.unlocalized_name
    }

    /// `GenericName` in the session locale.
    pub fn generic_name(&self) -> Option<&str> {
        self.generic_name.as_deref()
    }

    /// `Keywords` in the session locale, then the unlocalized `Keywords`.
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }

    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Exec, tokenized, with its field codes expanded. Never empty. The binary must find the
    /// first argument before the app shows, as it must find [`App::try_exec`].
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// `Terminal=true`. Such apps show, and step 8 decides which terminal runs them.
    pub fn terminal(&self) -> bool {
        self.terminal
    }

    /// The program the binary must find before the app shows: an absolute path, or a name
    /// to look up on `PATH`.
    pub fn try_exec(&self) -> Option<&str> {
        self.try_exec.as_deref()
    }

    /// `Path`, the directory the app runs in. The binary looks up a bare program there first,
    /// when it is absolute.
    pub fn work_dir(&self) -> Option<&str> {
        self.work_dir.as_deref()
    }

    /// In `Actions=` order.
    pub fn actions(&self) -> &[AppAction] {
        &self.actions
    }

    /// The argv that one of this app's items starts: Exec for Open, and a Desktop Action's own
    /// Exec for that action. Copy Path and the items of other rows start nothing.
    pub fn argv_for(&self, item: &ActionItem) -> Option<&[String]> {
        match item {
            ActionItem::Launch(id) if *id == self.id => Some(&self.argv),
            ActionItem::LaunchAction(id, action) if *id == self.id => self
                .actions
                .iter()
                .find(|own| own.action.id == action.id)
                .map(|own| own.argv.as_slice()),
            _ => None,
        }
    }

    /// The facts the chord table reads: the ID and the Desktop Actions.
    pub fn subject(&self) -> Subject {
        Subject::App {
            id: self.id.clone(),
            actions: self
                .actions
                .iter()
                .map(|action| action.action.clone())
                .collect(),
        }
    }
}

/// What a file that parses says. The binary reads only the first file for an ID, so a
/// `Hidden` verdict masks the same ID in later directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Shown(Box<App>),
    Hidden(Skip),
}

/// Why a parseable entry shows nothing, in the order the parser checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    /// `Hidden=true`: the entry is deleted.
    Hidden,
    /// `Type` is missing or is not `Application`.
    NotApplication,
    NoDisplay,
    /// `OnlyShowIn` or `NotShowIn` rules out the current desktops.
    NotShownIn,
    NoName,
    /// No `Exec`. D-Bus activation is not supported.
    NoExec,
    BadExec(ExecError),
}

/// Reads one desktop file. `id` is its desktop-file ID and `path` its path, for `%k`.
pub fn parse(id: AppId, path: &str, text: &str, session: &Session) -> Result<Verdict, Unparseable> {
    let file = KeyFile::parse(text)?;
    Ok(match app(id, path, &file, session) {
        Ok(app) => Verdict::Shown(Box::new(app)),
        Err(skip) => Verdict::Hidden(skip),
    })
}

fn app(id: AppId, path: &str, file: &KeyFile<'_>, session: &Session) -> Result<App, Skip> {
    let entry = file.entry();
    let locales = &session.locales;
    if entry.boolean("Hidden") {
        return Err(Skip::Hidden);
    }
    if entry.string("Type").as_deref() != Some("Application") {
        return Err(Skip::NotApplication);
    }
    if entry.boolean("NoDisplay") {
        return Err(Skip::NoDisplay);
    }
    let only = entry.list("OnlyShowIn");
    let not = entry.list("NotShowIn");
    if !shows_in(only.as_deref(), not.as_deref(), &session.desktops) {
        return Err(Skip::NotShownIn);
    }
    let name = entry.localized("Name", locales).ok_or(Skip::NoName)?;
    let exec = entry.string("Exec").ok_or(Skip::NoExec)?;
    let icon = entry
        .localized("Icon", locales)
        .filter(|icon| !icon.is_empty());
    let context = ExecContext {
        name: &name,
        icon: icon.as_deref(),
        path,
    };
    let argv = tokenize(&exec, &context).map_err(Skip::BadExec)?;
    let actions = actions(file, &context, locales);
    let mut keywords = entry
        .localized_list("Keywords", locales)
        .unwrap_or_default();
    keywords.extend(entry.list("Keywords").unwrap_or_default());
    Ok(App {
        id,
        path: path.to_owned(),
        unlocalized_name: entry.string("Name").unwrap_or_else(|| name.clone()),
        generic_name: entry
            .localized("GenericName", locales)
            .filter(|generic| !generic.is_empty()),
        keywords,
        argv,
        terminal: entry.boolean("Terminal"),
        try_exec: entry
            .string("TryExec")
            .filter(|program| !program.is_empty()),
        work_dir: entry.string("Path").filter(|dir| !dir.is_empty()),
        actions,
        name,
        icon,
    })
}

/// The `Actions=` IDs in order, each kept when its `[Desktop Action <id>]` group has a Name and
/// an Exec that tokenizes. Field codes expand to the app's values.
fn actions(file: &KeyFile<'_>, context: &ExecContext<'_>, locales: &[String]) -> Vec<AppAction> {
    let ids = file.entry().list("Actions").unwrap_or_default();
    ids.into_iter()
        .filter_map(|id| {
            let group = file.group(&format!("Desktop Action {id}"))?;
            let name = group.localized("Name", locales)?;
            let argv = tokenize(&group.string("Exec")?, context).ok()?;
            Some(AppAction {
                action: DesktopAction { id, name },
                argv,
            })
        })
        .collect()
}

/// GLib's walk: the first current desktop in `OnlyShowIn` shows the entry, and the first in
/// `NotShowIn` hides it. With neither, the entry shows only when it has no `OnlyShowIn`.
fn shows_in(only: Option<&[String]>, not: Option<&[String]>, desktops: &[String]) -> bool {
    for desktop in desktops {
        if only.is_some_and(|only| only.contains(desktop)) {
            return true;
        }
        if not.is_some_and(|not| not.contains(desktop)) {
            return false;
        }
    }
    only.is_none()
}
