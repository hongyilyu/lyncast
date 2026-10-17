//! The daemon's state and what each request does to it. No method here blocks. [`serve`] owns
//! the socket and the threads, and only its owner loop calls the [`Daemon`], so no lock guards
//! the state. docs/design/daemon.md describes the requests.

mod runner;
mod serve;
mod store;

pub use serve::run;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lyncast_core::rank::{Index, Timestamp, Visits};
use lyncast_core::{ActionItem, Command, Key, Subject, root};

use crate::clip::{self, Clips};
use crate::config;
use crate::vars::Vars;
use crate::view::{self, Spelled};
use crate::wire::{Reply, Request, Summon, Via};
use store::Store;

/// The reason every request gets once Quit has begun.
const QUITTING: &str = "the daemon is quitting";

/// The reply to anything that arrives once Quit has begun.
pub fn quitting() -> Reply {
    Reply::Failed {
        reason: QUITTING.to_owned(),
    }
}

/// How long a host waits after Quit for the daemon to stop.
pub const QUIT_WAIT: Duration = Duration::from_secs(5);

/// Everything the requests read and change.
pub struct Daemon {
    /// The environment read at start. Reload Apps scans with it again on a worker thread.
    vars: Arc<Vars>,
    index: Index,
    /// The number of the newest scan handed out, and of the scan `index` came from. The scan at
    /// start is 0.
    scans: u64,
    indexed: u64,
    visits: Visits,
    /// Where the visits persist. `None` when they stay in memory, and also once the writer has
    /// ended after Quit, so nothing is left to write.
    store: Option<Store>,
    clips: Clips,
    summon: Summon,
    /// The jobs handed out that have not finished.
    jobs: usize,
    /// Quit has begun, so every request fails.
    quitting: bool,
}

/// What one request comes to.
pub enum Handled {
    Reply(Reply),
    /// Work that blocks, for a worker thread. Its result goes to [`Daemon::finish`].
    Job(Job),
    /// Quit has begun. The reply goes out once the daemon has [stopped](Daemon::stopped).
    Quit(Reply),
}

/// An activation's blocking work, and what the activation does once the work succeeds.
pub struct Job {
    pub work: Work,
    pub success: Success,
}

/// Work that blocks, so the owner loop hands it to a worker thread.
pub enum Work {
    /// Starts the argv through the runner.
    Spawn(Vec<String>),
    /// Creates the config file if it is missing, then opens it.
    OpenConfig(PathBuf),
    /// Scans the apps again and indexes them.
    Scan { vars: Arc<Vars>, number: u64 },
}

pub enum WriterKind {
    Visits,
    Clips,
}

/// What finished work brings back.
pub enum Done {
    Ran,
    Scanned { number: u64, index: Index },
}

/// What a successful activation records and replies.
pub struct Success {
    key: Key,
    /// The search text, which the visit keeps as a term.
    query: String,
    message: String,
}

impl Daemon {
    pub fn new(
        vars: Vars,
        index: Index,
        visits: Visits,
        store: Option<Store>,
        clips: Clips,
    ) -> Daemon {
        Daemon {
            vars: Arc::new(vars),
            index,
            scans: 0,
            indexed: 0,
            visits,
            store,
            clips,
            summon: Summon::Closed,
            jobs: 0,
            quitting: false,
        }
    }

    /// The start has succeeded, so the clips that have expired at `now` go.
    pub fn started(&mut self, now: Timestamp) {
        self.clips.prune(now);
    }

    /// Answers one request at `now`, or hands back the work it waits for. Once Quit has begun,
    /// every request fails with [`QUITTING`].
    pub fn handle(&mut self, request: Request, now: Timestamp) -> Handled {
        if self.quitting {
            return Handled::Reply(quitting());
        }
        let reply = match request {
            Request::Toggle => {
                self.summon = match self.summon {
                    Summon::Closed => Summon::Open,
                    Summon::Open => Summon::Closed,
                };
                Reply::Summon { state: self.summon }
            }
            Request::Status => Reply::Summon { state: self.summon },
            Request::Query { text } => Reply::Rows {
                rows: view::rows(&root::rows(&self.index, &self.visits, &text, now)),
            },
            Request::Clips => Reply::Clips {
                clips: view::clips(&self.clips.rows()),
            },
            Request::Activate {
                key,
                via,
                confirmed,
                query,
            } => {
                let handled = self.activate(key, &via, confirmed, query, now);
                if let Handled::Job(_) = handled {
                    self.jobs += 1;
                }
                return handled;
            }
        };
        Handled::Reply(reply)
    }

    /// The reply to a job's result. A failure records nothing and leaves the summon as it was.
    /// A scan older than the one the index came from changes nothing.
    pub fn finish(
        &mut self,
        success: Success,
        done: Result<Done, String>,
        now: Timestamp,
    ) -> Reply {
        self.jobs -= 1;
        let reply = match done {
            Ok(done) => {
                if let Done::Scanned { number, index } = done
                    && number > self.indexed
                {
                    self.index = index;
                    self.indexed = number;
                }
                self.succeed(success, Summon::Closed, now)
            }
            Err(reason) => Reply::Failed { reason },
        };
        self.settle();
        reply
    }

    /// Quit has begun, no job is out, and each writer has ended after its last write.
    pub fn stopped(&self) -> bool {
        self.idle() && self.store.is_none() && self.clips.has_no_writer()
    }

    /// A writer has ended. Each ends only once Quit has begun and no job is out.
    pub fn writer_ended(&mut self, writer: WriterKind) {
        match writer {
            WriterKind::Visits => self.store = None,
            WriterKind::Clips => self.clips.writer_ended(),
        }
    }

    /// Quit has begun and no job is out, so nothing records a visit again.
    fn idle(&self) -> bool {
        self.quitting && self.jobs == 0
    }

    /// Once the daemon is idle, the writers can end after their last writes.
    fn settle(&self) {
        if !self.idle() {
            return;
        }
        if let Some(store) = &self.store {
            store.close();
        }
        self.clips.close();
    }

    /// Rebuilds the row from current state, takes the item that `via` names from its actions,
    /// asks first for a command that confirms, and runs the item.
    fn activate(
        &mut self,
        key: Key,
        via: &Via,
        confirmed: bool,
        query: String,
        now: Timestamp,
    ) -> Handled {
        let failed = |reason: String| Handled::Reply(Reply::Failed { reason });
        let actions = match self.rebuild(&key) {
            Ok(subject) => subject.actions(),
            Err(reason) => return failed(reason),
        };
        let item = match via {
            Via::Chord(chord) => actions.on(*chord),
            Via::Menu(verb) => match actions.pick(verb) {
                Some(item) => item,
                None => return failed(format!("{key} has no action {}", verb.spelling())),
            },
        };
        if let ActionItem::Run(command) = item
            && command.confirms()
            && !confirmed
        {
            let title = command.title().to_owned();
            return Handled::Reply(Reply::Confirm { title });
        }
        let success = |message: String| Success {
            key: key.clone(),
            query: query.clone(),
            message,
        };
        let launch = |id, message: String| {
            let argv = self
                .index
                .app(id)
                .and_then(|app| app.argv_for(item))
                .expect("an app's row offers only its own Open and Desktop Actions");
            Handled::Job(Job {
                work: Work::Spawn(argv.to_vec()),
                success: success(message),
            })
        };
        match item {
            ActionItem::Launch(id) => launch(id, format!("launched {key}")),
            ActionItem::LaunchAction(id, action) => {
                launch(id, format!("launched {key} {}", action.id))
            }
            ActionItem::CopyPath(_) | ActionItem::CopyAnswer(_) | ActionItem::CopyClip(_) => {
                failed("not copied: no Wayland compositor".to_owned())
            }
            ActionItem::Pin(id) => {
                let changed = self.clips.pin(*id, now);
                self.changed(changed, format!("pinned clip {}", id.0))
            }
            ActionItem::Unpin(id) => {
                let changed = self.clips.unpin(*id, now);
                self.changed(changed, format!("unpinned clip {}", id.0))
            }
            ActionItem::Delete(id) => {
                let changed = self.clips.delete(*id);
                self.changed(changed, format!("deleted clip {}", id.0))
            }
            ActionItem::Run(command) => self.command(*command, success(format!("ran {key}")), now),
        }
    }

    /// What each command does, as the table in docs/design/daemon.md lists it.
    fn command(&mut self, command: Command, success: Success, now: Timestamp) -> Handled {
        let spawn = |argv: &[&str]| Work::Spawn(argv.iter().map(|&arg| arg.to_owned()).collect());
        let work = match command {
            Command::ClipboardHistory => {
                return Handled::Reply(self.succeed(success, Summon::Open, now));
            }
            // Quit records no visit, so it never leads Suggestions.
            Command::Quit => {
                self.quitting = true;
                self.settle();
                return Handled::Quit(Reply::Done {
                    message: success.message,
                });
            }
            Command::OpenConfig => match self.vars.config_home() {
                Some(dir) => Work::OpenConfig(config::path(Path::new(&dir))),
                None => {
                    let reason = "not run: XDG_CONFIG_HOME and HOME are not set".to_owned();
                    return Handled::Reply(Reply::Failed { reason });
                }
            },
            Command::ReloadApps => {
                self.scans += 1;
                Work::Scan {
                    vars: Arc::clone(&self.vars),
                    number: self.scans,
                }
            }
            Command::LockScreen => spawn(&["loginctl", "lock-session"]),
            Command::Suspend => spawn(&["systemctl", "suspend"]),
            Command::LogOut => spawn(&["niri", "msg", "action", "quit", "--skip-confirmation"]),
            Command::Reboot => spawn(&["systemctl", "reboot"]),
            Command::PowerOff => spawn(&["systemctl", "poweroff"]),
        };
        Handled::Job(Job { work, success })
    }

    /// The subject of the row a key names, from current state: an app from the index, a command
    /// from its key, a clip from the history, and a calculator row from the answer to the key's
    /// text, as the root list finds it.
    fn rebuild(&self, key: &Key) -> Result<Subject, String> {
        match key {
            Key::App(id) => match self.index.app(id) {
                Some(app) => Ok(app.subject()),
                None => Err(format!("no app {}", id.0)),
            },
            Key::Command(command) => Ok(Subject::Command(*command)),
            Key::Clip(id) => match self.clips.get(*id) {
                Some(row) => Ok(Subject::Clip {
                    id: *id,
                    pinned: row.pinned,
                }),
                None => Err(clip::no_clip(*id)),
            },
            Key::Calc(query) => match root::answer(query) {
                Some(_) => Ok(Subject::Calc {
                    query: query.clone(),
                }),
                None => Err(format!("{key} has no answer")),
            },
        }
    }

    fn changed(&mut self, changed: Result<(), String>, message: String) -> Handled {
        Handled::Reply(match changed {
            Ok(()) => {
                self.summon = Summon::Closed;
                Reply::Done { message }
            }
            Err(reason) => Reply::Failed { reason },
        })
    }

    /// Records the visit, sets the summon, hands the visits to the writer, and gives the success
    /// line.
    fn succeed(&mut self, success: Success, summon: Summon, now: Timestamp) -> Reply {
        self.visits.record(success.key, &success.query, now);
        self.summon = summon;
        if let Some(store) = &self.store {
            store.save(&self.visits);
        }
        Reply::Done {
            message: success.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lyncast_core::clip::Retention;
    use lyncast_core::desktop::{self, Session, Verdict};
    use lyncast_core::{AppId, Chord};

    use super::*;
    use crate::doc;

    /// Reload Apps, and the number and the success of the scan it hands out.
    fn reload(daemon: &mut Daemon) -> (u64, Success) {
        let request = Request::Activate {
            key: Key::Command(Command::ReloadApps),
            via: Via::Chord(Chord::Enter),
            confirmed: false,
            query: String::new(),
        };
        match daemon.handle(request, Timestamp(0)) {
            Handled::Job(Job {
                work: Work::Scan { number, .. },
                success,
            }) => (number, success),
            _ => panic!("Reload Apps hands out a scan"),
        }
    }

    /// The reply to a query for `text`, as the wire spells it.
    fn query(daemon: &mut Daemon, text: &str) -> String {
        let request = Request::Query {
            text: text.to_owned(),
        };
        match daemon.handle(request, Timestamp(0)) {
            Handled::Reply(reply) => serde_json::to_string(&reply).unwrap(),
            _ => panic!("a query is answered at once"),
        }
    }

    /// docs/design/daemon.md's "Reload Apps scans again" section: two scans that end out of
    /// order, where the newer one holds the doc's first `desktop` block and the older one
    /// holds no app. The older scan, ending last, changes nothing. A session cannot hold one
    /// scan back, so the test ends both scans itself. It runs no scan and no command, so no
    /// variable matters, and an empty `Vars` keeps the developer's environment out.
    #[test]
    fn a_scan_that_ends_after_a_newer_one_changes_nothing() {
        let block = &doc::DAEMON.blocks("desktop")[0];
        let path = &block.args[0];
        let id = Path::new(path).file_name().unwrap().to_str().unwrap();
        let session = Session::new(&[""; 0], "");
        let entry = desktop::parse(
            AppId(id.to_owned()),
            path,
            &block.lines.join("\n"),
            &session,
        );
        let Ok(Verdict::Shown(app)) = entry else {
            panic!("{path} shows");
        };
        let name = app.name().to_owned();
        let mut daemon = Daemon::new(
            Vars::default(),
            Index::new(Vec::new()),
            Visits::default(),
            None,
            Clips::open(None, Retention::DEFAULT, || {}).unwrap(),
        );
        let (older, older_success) = reload(&mut daemon);
        let (newer, newer_success) = reload(&mut daemon);
        let scanned = |number, apps| {
            Ok(Done::Scanned {
                number,
                index: Index::new(apps),
            })
        };
        daemon.finish(newer_success, scanned(newer, vec![*app]), Timestamp(0));
        let after_newer = query(&mut daemon, &name);
        assert!(
            after_newer.contains(&format!("\"key\":\"app:{id}\"")),
            "the newer scan's app shows: {after_newer}"
        );
        daemon.finish(older_success, scanned(older, Vec::new()), Timestamp(0));
        assert_eq!(
            query(&mut daemon, &name),
            after_newer,
            "the older scan changes nothing"
        );
    }
}
