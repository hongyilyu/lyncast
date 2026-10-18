//! The daemon's state and what each request does to it. No method here blocks. [`serve`] owns
//! the socket and the threads, and only its host calls the [`Daemon`], so no lock guards
//! the state. docs/design/daemon.md describes the requests.

mod face;
mod owner;
mod runner;
mod serve;
mod store;

pub use serve::run;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use lyncast_core::clip::{Delivery, Handed};
use lyncast_core::rank::{Index, Timestamp, Visits};
use lyncast_core::{ActionItem, ClipId, Command, Key, Row, Subject, root};

use crate::clip::{self, Capture, Clips, Entry, Payload};
use crate::config;
use crate::vars::Vars;
use crate::view::Spelled;
use crate::wire::{Request, Via};
use store::Store;

/// The reason every request gets once Quit has begun.
const QUITTING: &str = "the daemon is quitting";

/// The answer to anything that arrives once Quit has begun.
pub fn quitting() -> Answer {
    Answer::Failed(QUITTING.to_owned())
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
    capture: CaptureThread,
    /// The jobs handed out that have not finished.
    jobs: usize,
    /// Quit has begun, so every request fails.
    quitting: bool,
}

enum CaptureThread {
    NoCompositor,
    Running(Capture),
    Ended,
}

/// What one request comes to.
pub enum Handled {
    Answer(Answer),
    /// Work that waits, for the host. Its result goes to [`Daemon::finish`].
    Job(Job),
    /// Quit has begun. The answer goes out once the daemon has [stopped](Daemon::stopped).
    Quit(Answer),
}

/// The daemon's answer to a request, in the daemon's own types. Only `view::reply` spells one
/// as a wire reply.
pub enum Answer {
    /// Toggle or Status. The daemon holds no summon, so the host's face flips or reads its own.
    Summon(Ask),
    /// The root list for a query, in `Section` order. The calculator card's row carries its
    /// `calc::Answer`.
    Rows(Vec<Row>),
    /// The clipboard history in screen order.
    Clips(Vec<clip::Row>),
    /// The success line, such as `ran cmd:reboot`, and what the success does to the summon.
    Done { message: String, after: After },
    /// The command asks first. Nothing ran.
    Confirm(Command),
    /// The reason, without the `lyncast: ` prefix.
    Failed(String),
}

/// The two requests about the summon.
#[derive(Clone, Copy)]
pub enum Ask {
    Toggle,
    Status,
}

/// What a success does to the summon. A failure and a confirmation leave it alone.
#[derive(Clone, Copy)]
pub enum After {
    Close,
    /// Clipboard History's: open the summon, or leave it open.
    ShowClipboard,
}

/// An activation's work that waits, and what the activation does once the work succeeds.
pub struct Job {
    pub work: Work,
    pub success: Success,
}

/// Work that waits, so the daemon hands it to the host.
pub enum Work {
    Task(Task),
    Copy(Payload),
}

/// Work that blocks, for a worker thread.
pub enum Task {
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
    Copied,
    Scanned { number: u64, index: Index },
}

/// What a successful activation records and replies.
pub struct Success {
    record: Record,
    message: String,
}

enum Record {
    Visit {
        key: Key,
        /// The search text, which the visit keeps as a term.
        query: String,
    },
    Write(Delivery),
}

impl Daemon {
    pub fn new(
        vars: Vars,
        index: Index,
        visits: Visits,
        store: Option<Store>,
        clips: Clips,
        capture: Option<Capture>,
    ) -> Daemon {
        Daemon {
            vars: Arc::new(vars),
            index,
            scans: 0,
            indexed: 0,
            visits,
            store,
            clips,
            capture: capture.map_or(CaptureThread::NoCompositor, CaptureThread::Running),
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
            return Handled::Answer(quitting());
        }
        let answer = match request {
            Request::Toggle => Answer::Summon(Ask::Toggle),
            Request::Status => Answer::Summon(Ask::Status),
            Request::Query { text } => {
                Answer::Rows(root::rows(&self.index, &self.visits, &text, now))
            }
            Request::Clips => Answer::Clips(self.clips.rows().into_iter().cloned().collect()),
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
        Handled::Answer(answer)
    }

    /// The answer to a job's result. A failure records nothing. A scan older than the one the
    /// index came from changes nothing.
    pub fn finish(
        &mut self,
        success: Success,
        done: Result<Done, String>,
        now: Timestamp,
    ) -> Answer {
        self.jobs -= 1;
        let answer = match done {
            Ok(done) => {
                if let Done::Scanned { number, index } = done
                    && number > self.indexed
                {
                    self.index = index;
                    self.indexed = number;
                }
                self.succeed(success, After::Close, now)
            }
            Err(reason) => Answer::Failed(reason),
        };
        self.settle();
        answer
    }

    /// Records what the clipboard offered. The capture thread keeps running while Quit waits
    /// for the work in flight, so its entries still count.
    pub fn captured(&mut self, entry: Entry, now: Timestamp) {
        self.clips.record(entry, now);
    }

    /// The writer of the clips could not save the new row `id`, so the history drops it.
    pub fn unsaved(&mut self, id: ClipId) {
        self.clips.unsaved(id);
    }

    /// The capture thread has ended: Quit cancelled it, or the compositor went once the stop had
    /// begun.
    pub fn capture_ended(&mut self) {
        self.capture = CaptureThread::Ended;
        self.settle();
    }

    /// The compositor has gone, so the daemon stops as Quit does, with no reply to send.
    pub fn compositor_closed(&mut self) {
        self.quitting = true;
        self.capture_ended();
    }

    /// Quit has begun, no job is out, the capture thread has ended, and each writer has ended
    /// after its last write.
    pub fn stopped(&self) -> bool {
        self.idle()
            && !matches!(self.capture, CaptureThread::Running(_))
            && self.store.is_none()
            && self.clips.has_no_writer()
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
        if let CaptureThread::Running(capture) = &self.capture {
            return capture.cancel();
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
        let failed = |reason: String| Handled::Answer(Answer::Failed(reason));
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
            return Handled::Answer(Answer::Confirm(*command));
        }
        let success = |message: String| Success {
            record: Record::Visit {
                key: key.clone(),
                query: query.clone(),
            },
            message,
        };
        let launch = |id, message: String| {
            let argv = self
                .index
                .app(id)
                .and_then(|app| app.argv_for(item))
                .expect("an app's row offers only its own Open and Desktop Actions");
            Handled::Job(Job {
                work: Work::Task(Task::Spawn(argv.to_vec())),
                success: success(message),
            })
        };
        match item {
            ActionItem::Launch(id) => launch(id, format!("launched {key}")),
            ActionItem::LaunchAction(id, action) => {
                launch(id, format!("launched {key} {}", action.id))
            }
            ActionItem::CopyClip(id) => self.copy(Delivery::Clip(*id)),
            ActionItem::CopyAnswer(query) => match root::answer(query) {
                Some(answer) => self.copy(answer.delivery()),
                None => failed(format!("{key} has no answer")),
            },
            ActionItem::CopyPath(id) => {
                let app = self.index.app(id).expect("an app's row is in the index");
                self.copy(Delivery::Text(app.path().to_owned()))
            }
            ActionItem::Pin(id) => {
                let changed = self.clips.pin(*id, now);
                Self::changed(changed, format!("pinned clip {}", id.0))
            }
            ActionItem::Unpin(id) => {
                let changed = self.clips.unpin(*id, now);
                Self::changed(changed, format!("unpinned clip {}", id.0))
            }
            ActionItem::Delete(id) => {
                let changed = self.clips.delete(*id);
                Self::changed(changed, format!("deleted clip {}", id.0))
            }
            ActionItem::Run(command) => self.command(*command, success(format!("ran {key}")), now),
        }
    }

    /// What each command does, as the table in docs/design/daemon.md lists it.
    fn command(&mut self, command: Command, success: Success, now: Timestamp) -> Handled {
        let spawn = |argv: &[&str]| Task::Spawn(argv.iter().map(|&arg| arg.to_owned()).collect());
        let task = match command {
            Command::ClipboardHistory => {
                return Handled::Answer(self.succeed(success, After::ShowClipboard, now));
            }
            // Quit records no visit, so it never leads Suggestions.
            Command::Quit => {
                self.quitting = true;
                self.settle();
                return Handled::Quit(Answer::Done {
                    message: success.message,
                    after: After::Close,
                });
            }
            Command::OpenConfig => match self.vars.config_home() {
                Some(dir) => Task::OpenConfig(config::path(Path::new(&dir))),
                None => {
                    let reason = "not run: XDG_CONFIG_HOME and HOME are not set".to_owned();
                    return Handled::Answer(Answer::Failed(reason));
                }
            },
            Command::ReloadApps => {
                self.scans += 1;
                Task::Scan {
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
        Handled::Job(Job {
            work: Work::Task(task),
            success,
        })
    }

    /// The subject of the row a key names, from current state: an app from the index, a command
    /// from its key, a clip from the history, and a calculator row from the key's text.
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
            Key::Calc(query) => Ok(Subject::Calc {
                query: query.clone(),
            }),
        }
    }

    fn copy(&self, delivery: Delivery) -> Handled {
        let failed = |reason: String| Handled::Answer(Answer::Failed(reason));
        if !matches!(self.capture, CaptureThread::Running(_)) {
            return failed(clip::not_copied("no Wayland compositor"));
        }
        let payload = match self.clips.payload(&delivery) {
            Ok(payload) => payload,
            Err(reason) => return failed(reason),
        };
        let message = match delivery.handed() {
            Handed::Clip(id) => format!("copied clip {}", id.0),
            Handed::Text(text) => format!("copied {text}"),
        };
        Handled::Job(Job {
            work: Work::Copy(payload),
            success: Success {
                record: Record::Write(delivery),
                message,
            },
        })
    }

    fn changed(changed: Result<(), String>, message: String) -> Handled {
        Handled::Answer(match changed {
            Ok(()) => Answer::Done {
                message,
                after: After::Close,
            },
            Err(reason) => Answer::Failed(reason),
        })
    }

    /// Records the visit and hands the visits to the writer, or records the write a copy made,
    /// and gives the success line.
    fn succeed(&mut self, success: Success, after: After, now: Timestamp) -> Answer {
        match success.record {
            Record::Visit { key, query } => {
                self.visits.record(key, &query, now);
                if let Some(store) = &self.store {
                    store.save(&self.visits);
                }
            }
            Record::Write(delivery) => self.clips.wrote(&delivery, now),
        }
        Answer::Done {
            message: success.message,
            after,
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
    use crate::wire::Summon;
    use crate::{doc, view};

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
                work: Work::Task(Task::Scan { number, .. }),
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
            Handled::Answer(answer) => {
                serde_json::to_string(&view::reply(answer, Summon::Closed)).unwrap()
            }
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
            Clips::open(None, Retention::DEFAULT, |_| {}, || {}).unwrap(),
            None,
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
