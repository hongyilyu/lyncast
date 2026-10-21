//! The socket and the threads around the daemon's state. An accept thread starts one thread per
//! connection, for [`MAX_CONNECTIONS`] at most at once, which reads the request line, parses it,
//! and passes it to the host over a channel. Work that blocks runs on a worker thread, or on the
//! copier thread for a copy, and its result comes back over the same channel. The capture thread
//! hands over each entry there, and says there when it has ended, after Quit or because the
//! compositor has gone. The writer of the clips hands over each new row it could not save, and
//! each writer says there when it has ended. [`start`] claims the socket and opens the daemon,
//! and the host runs the [`Owner`] it gives, which never touches a socket.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::mem;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use futures::channel::mpsc as wake;
use futures::channel::oneshot;
use lyncast_core::ClipId;
use lyncast_core::rank::{Index, Timestamp, Visits};

use super::face::Windowless;
use super::owner::Owner;
use super::runner::Runner;
use super::store::Store;
use super::{Answer, Daemon, Done, QUITTING, Success, WriterKind};
use crate::apps;
use crate::clip::{Capture, Clips, Copier, Seen};
use crate::config;
use crate::permit::Permits;
use crate::vars::Vars;
use crate::warning;
use crate::wire::{self, Reply, Request, Summon};

/// The lock's file name in `$XDG_RUNTIME_DIR`.
const LOCK: &str = "lyncast.lock";

/// How long each read of a request and each write of a reply may wait, so a client that sends
/// or reads slowly is still served.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The start's wait for its warnings to reach stderr before it binds the socket, so a client that
/// the socket answers finds them there.
const WARNINGS_BEFORE_BIND: Duration = Duration::from_secs(1);

/// The wait of a daemon that stops on an error for its warnings and its last line before it exits.
const LAST_LINE_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// The wait of the one write of a daemon whose warnings thread did not start.
const ALONE_WRITE_WAIT: Duration = Duration::from_secs(1);

/// The most bytes a request line may hold, with its newline.
const MAX_REQUEST: u64 = 1 << 20;

/// The most connections open at once. Each holds a thread, and [`TIMEOUT`] bounds each read, not
/// the request, so a client that sends a byte at a time holds its thread for good.
const MAX_CONNECTIONS: u64 = 128;

/// Why a daemon whose compositor went exits 1.
pub(crate) const WAYLAND_CLOSED: &str = "the Wayland connection closed";

/// What the host reads.
pub(crate) enum Event {
    Request(Request, ReplyTo),
    /// A worker thread's task or the copier's copy ended.
    Finished(Success, Result<Done, String>, ReplyTo),
    /// A writer ended, after Quit.
    WriterEnded(WriterKind),
    Unsaved(ClipId),
    Seen(Seen),
}

/// Where a request's answer goes.
pub(crate) enum ReplyTo {
    /// The connection thread that read the request, which writes the wire reply.
    Socket(Socket),
    /// The palette in this process, which reads the answer as it is.
    Palette(oneshot::Sender<Answer>),
}

/// A connection thread's side of a request: where its reply goes, and how it says the reply is
/// written.
pub(crate) struct Socket {
    pub(super) reply: Sender<Reply>,
    pub(super) written: Receiver<()>,
}

/// The host's doorbell. Each producer rings it once it has queued an event, and a wake goes out
/// only when none is pending since the host last took one. The headless host has none: it
/// waits on its channel instead.
#[derive(Clone)]
pub(crate) struct Bell {
    pending: Arc<AtomicBool>,
    wake: wake::UnboundedSender<()>,
}

/// The host's side of the [`Bell`].
pub(crate) struct Rung {
    pending: Arc<AtomicBool>,
    woken: wake::UnboundedReceiver<()>,
}

pub(crate) enum Wake {
    Rang,
    /// Every [`Bell`] is gone, so nothing rings again.
    BellsGone,
}

impl Bell {
    pub(crate) fn new() -> (Bell, Rung) {
        let pending = Arc::new(AtomicBool::new(false));
        let (wake, woken) = wake::unbounded();
        let rung = Rung {
            pending: Arc::clone(&pending),
            woken,
        };
        (Bell { pending, wake }, rung)
    }

    /// Callable from any thread. The swap pairs with [`Rung::next`]'s, so a host that drains
    /// after the wake sees the event queued before the ring.
    fn ring(&self) {
        if !self.pending.swap(true, Ordering::AcqRel) {
            let _ = self.wake.unbounded_send(());
        }
    }
}

impl Rung {
    /// Waits for a ring and takes it, clearing the bell in the same poll, so a ring after this
    /// one wakes the host again however often it drains in between.
    pub(crate) async fn next(&mut self) -> Wake {
        if self.woken.next().await.is_none() {
            return Wake::BellsGone;
        }
        self.pending.swap(false, Ordering::AcqRel);
        Wake::Rang
    }
}

/// The sender every thread uses: the connection threads, the workers, the copier, the capture
/// thread, and the writers. It rings the host's bell after each event.
#[derive(Clone)]
pub(crate) struct Events {
    inbox: Sender<Event>,
    bell: Option<Bell>,
}

impl Events {
    /// Queues the event and rings. Once the host has ended, the event is dropped instead, with
    /// no one to tell.
    pub(crate) fn send_or_drop(&self, event: Event) {
        if self.inbox.send(event).is_ok() {
            self.ring();
        }
    }

    pub(super) fn ring(&self) {
        if let Some(bell) = &self.bell {
            bell.ring();
        }
    }
}

/// Starts the warnings thread, then runs the daemon in `host`, [`run_headless`] or the palette's,
/// until a Quit command or until it stops on an error, and gives its exit code.
pub fn run(host: impl FnOnce() -> Result<(), String>) -> ExitCode {
    if let Err(error) = warning::start() {
        warning::write_alone(format_args!("{}", not_started(error)), ALONE_WRITE_WAIT);
        return ExitCode::FAILURE;
    }
    match host() {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            warning::last(format_args!("{reason}"));
            warning::flush(Instant::now() + LAST_LINE_BEFORE_EXIT);
            ExitCode::FAILURE
        }
    }
}

/// Claims the socket and starts the daemon and its threads, in the order docs/design/daemon.md
/// gives, and gives the owner a host runs it with. The lock comes first: [`open_daemon`], which
/// reads the user's files, borrows it, and [`Lock::bind`] takes it, so a second daemon fails
/// before it scans or reads `ranking.json`, and only the lock holder removes a stale socket and
/// binds. The runner sends each program to niri, or `record` swaps it for a recorder that writes
/// to that file. Each event rings `bell`, if the host has one.
pub(crate) fn start(record: Option<PathBuf>, bell: Option<Bell>) -> Result<Owner, String> {
    let runtime = wire::runtime_dir()?;
    let lock = lock(&runtime.join(LOCK))?;
    let (inbox_sender, inbox) = mpsc::channel();
    let events = Events {
        inbox: inbox_sender,
        bell,
    };
    let (mut daemon, copier) = open_daemon(&lock, &events)?;
    warning::flush(Instant::now() + WARNINGS_BEFORE_BIND);
    let (claim, listener) = lock.bind(runtime.join(wire::SOCKET))?;
    let accepted = events.clone();
    thread::Builder::new()
        .spawn(move || accept(&listener, &accepted))
        .map_err(not_started)?;
    daemon.started(now());
    let niri = daemon.vars().niri_socket.clone();
    let runner = record.map_or(Runner::Niri(niri), Runner::recorder);
    Ok(Owner::new(
        daemon, inbox, claim, events, runner, copier, now,
    ))
}

/// The daemon, from the environment and the user's files: `config.toml`, the desktop entries,
/// `ranking.json`, and `clips.sqlite3`. Then the capture thread starts. Only the lock holder
/// reads them, which `_held` proves.
fn open_daemon(_held: &Lock, events: &Events) -> Result<(Daemon, Option<Copier>), String> {
    let vars = Vars::read().map_err(not_started)?;
    let config = config::read(vars.config_home().as_deref().map(Path::new));
    let index = Index::new(apps::scan(&vars));
    let ended = |writer: WriterKind| {
        let events = events.clone();
        move || {
            events.send_or_drop(Event::WriterEnded(writer));
        }
    };
    let data_home = vars.data_home().map(PathBuf::from);
    let (visits, store) = match &data_home {
        Some(dir) => Store::open(dir, ended(WriterKind::Visits)).map_err(not_started)?,
        None => (Visits::default(), None),
    };
    let unsaved = {
        let events = events.clone();
        move |id| {
            events.send_or_drop(Event::Unsaved(id));
        }
    };
    let clips = Clips::open(
        data_home.as_deref(),
        config.retention,
        unsaved,
        ended(WriterKind::Clips),
    )
    .map_err(not_started)?;
    let (capture, copier) = match vars.wayland_display {
        Some(_) => {
            let events = events.clone();
            let seen = move |seen| {
                events.send_or_drop(Event::Seen(seen));
            };
            let (capture, copier) = Capture::start(clips.pending(), seen).map_err(not_started)?;
            (Some(capture), Some(copier))
        }
        None => (None, None),
    };
    let daemon = Daemon::new(vars, config.terminal, index, visits, store, clips, capture);
    Ok((daemon, copier))
}

/// The headless host. It serves until a Quit command or until the compositor goes, and then
/// stops in the order docs/design/daemon.md gives: no new connections, the work in flight, the
/// last writes, the reply. The daemon keeps answering while it stops.
pub(crate) fn run_headless(record: Option<PathBuf>) -> Result<(), String> {
    start(record, None)?.serve_blocking(&mut Windowless(Summon::Closed))
}

/// A daemon's start with no socket, no environment, and no compositor, for the palette's Tier 3
/// tests: the runner records to `record`, the claim is a lock file beside it, and `now` is the
/// clock. The events go to the inbox as a socket's connection threads send them.
#[cfg(test)]
pub(crate) fn detached(
    daemon: Daemon,
    record: &Path,
    bell: Bell,
    now: fn() -> Timestamp,
) -> (Owner, Events) {
    let (inbox_sender, inbox) = mpsc::channel();
    let events = Events {
        inbox: inbox_sender,
        bell: Some(bell),
    };
    let lock = File::create(record.with_extension("lock")).expect("a scratch lock file");
    let claim = Claim {
        lock: Lock(lock),
        socket: record.with_extension("sock"),
    };
    let runner = Runner::recorder(record.to_owned());
    let owner = Owner::new(daemon, inbox, claim, events.clone(), runner, None, now);
    (owner, events)
}

/// A request as a connection thread hands it over, from a client that reads no reply.
#[cfg(test)]
pub(crate) fn from_socket(request: Request) -> Event {
    let (reply, _) = mpsc::channel();
    let (_, written) = mpsc::channel();
    Event::Request(request, ReplyTo::Socket(Socket { reply, written }))
}

/// The reason the daemon gives when `WAYLAND_SOCKET` is set, when a thread it needs cannot start,
/// or when the compositor cannot be reached or offers no seat or neither data-control protocol.
fn not_started(error: impl fmt::Display) -> String {
    format!("not started: {error}")
}

/// The `flock` on the lock file, which only [`lock`] takes.
pub(super) struct Lock(pub(super) File);

impl Lock {
    /// Removes a socket that a daemon left when it died, then binds a new one. Only the lock
    /// holder binds, so this takes the lock, and the claim holds it from then on.
    fn bind(self, socket: PathBuf) -> Result<(Claim, UnixListener), String> {
        let failed = |error: io::Error| format!("{}: {error}", socket.display());
        match fs::remove_file(&socket) {
            Err(error) if error.kind() != ErrorKind::NotFound => return Err(failed(error)),
            _ => {}
        }
        let listener = UnixListener::bind(&socket).map_err(failed)?;
        Ok((Claim { lock: self, socket }, listener))
    }
}

/// The lock and the socket path, which Quit gives up.
pub(super) struct Claim {
    pub(super) lock: Lock,
    pub(super) socket: PathBuf,
}

impl Claim {
    /// The lock goes after the last write and before the reply, so a daemon started after the
    /// reply can take it. A daemon that has not stopped may still be writing, so it keeps the
    /// lock until the process exits, and a daemon started in its place never writes
    /// ranking.json at the same time.
    pub(super) fn give_up(self, stopped: bool) {
        if stopped {
            drop(self.lock.0);
        } else {
            mem::forget(self.lock.0);
        }
    }
}

/// Takes an exclusive `flock` on `path`. The kernel releases it when the file closes, which
/// happens however the process exits. The open does not wait for a FIFO's reader, so a FIFO at
/// `path` fails at once, and any other file that is not regular, such as a link to a device, is
/// refused.
fn lock(path: &Path) -> Result<Lock, String> {
    let failed = |error: io::Error| format!("{}: {error}", path.display());
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
        .map_err(failed)?;
    if !file.metadata().map_err(failed)?.is_file() {
        return Err(format!("{}: not a regular file", path.display()));
    }
    match file.try_lock() {
        Ok(()) => Ok(Lock(file)),
        Err(fs::TryLockError::WouldBlock) => Err("a daemon is already running".to_owned()),
        Err(fs::TryLockError::Error(error)) => Err(failed(error)),
    }
}

impl Event {
    /// A `query`, `status`, or `clips` request, which changes nothing.
    pub(super) fn is_read(&self) -> bool {
        matches!(
            self,
            Event::Request(Request::Query { .. } | Request::Status | Request::Clips, _)
        )
    }
}

fn accept(listener: &UnixListener, events: &Events) {
    let places = Permits::new(MAX_CONNECTIONS);
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // An error such as EMFILE repeats until a descriptor frees, so wait instead of spinning.
            thread::sleep(Duration::from_millis(10));
            continue;
        };
        let Some(place) = places.take(1) else {
            turn_away(
                &stream,
                &format!("busy: {MAX_CONNECTIONS} connections are open"),
            );
            continue;
        };
        let events = events.clone();
        let started = spawn_with((stream, place), move |(stream, _place)| {
            // A connection that fails to read or write ends alone.
            let _ = connection(stream, &events);
        });
        if let Err(((stream, _place), error)) = started {
            turn_away(&stream, &format!("not served: {error}"));
        }
    }
}

/// Starts a thread that runs `work` on `value`, or gives `value` back with the error when the
/// thread cannot start. `thread::Builder::spawn` takes its closure even when the thread cannot
/// start, so `value` goes to the thread over a channel once it runs.
pub(super) fn spawn_with<T: Send + 'static>(
    value: T,
    work: impl FnOnce(T) + Send + 'static,
) -> Result<(), (T, io::Error)> {
    let (hand, handed) = mpsc::channel();
    let started = thread::Builder::new().spawn(move || {
        if let Ok(value) = handed.recv() {
            work(value);
        }
    });
    match started {
        Ok(_) => {
            let _ = hand.send(value);
            Ok(())
        }
        Err(error) => Err((value, error)),
    }
}

/// Fails a connection that gets no thread, before its request is read, without waiting, so the
/// accept thread never blocks. A new connection has room for the line.
fn turn_away(stream: &UnixStream, reason: &str) {
    let failed = Reply::Failed {
        reason: reason.to_owned(),
    };
    let _ = stream
        .set_nonblocking(true)
        .and_then(|()| (&*stream).write_all(&reply_line(&failed)));
}

/// Reads one request line, hands it to the host, and writes the reply line. A line that
/// does not parse gets a `failed` reply here, and the daemon keeps serving.
fn connection(stream: UnixStream, events: &Events) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_until(b'\n', &mut line)?;
    let (written, written_rx) = mpsc::channel();
    let reply = match parse(&line) {
        Ok(request) => {
            let (reply, reply_rx) = mpsc::channel();
            let reply_to = ReplyTo::Socket(Socket {
                reply,
                written: written_rx,
            });
            events.send_or_drop(Event::Request(request, reply_to));
            reply_rx.recv().unwrap_or_else(|_| Reply::Failed {
                reason: QUITTING.to_owned(),
            })
        }
        Err(reason) => Reply::Failed { reason },
    };
    (&stream).write_all(&reply_line(&reply))?;
    let _ = written.send(());
    Ok(())
}

fn reply_line(reply: &Reply) -> Vec<u8> {
    let mut line = serde_json::to_vec(reply).expect("a reply serializes");
    line.push(b'\n');
    line
}

/// The request on a line that `read_until` read. The line has no newline when the client
/// stopped early or [`MAX_REQUEST`] cut it, and then none of it runs, because the bytes before
/// the cut can parse as a whole request.
fn parse(line: &[u8]) -> Result<Request, String> {
    let Some(line) = line.strip_suffix(b"\n") else {
        return Err("bad request: no newline in the first 1 MiB".to_owned());
    };
    serde_json::from_slice(line).map_err(|error| format!("bad request: {error}"))
}

/// The one clock read per event. A clock before 1970 reads as the epoch.
pub(crate) fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}
