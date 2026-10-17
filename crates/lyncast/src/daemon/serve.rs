//! The socket and the threads around the daemon's state. An accept thread starts one thread per
//! connection, which reads the request line, parses it, and passes it to the owner loop over a
//! channel. The owner loop holds the [`Daemon`] and never touches a socket. Work that blocks runs
//! on a worker thread, and a copy on the copier thread, and each result comes back over the same
//! channel. No [`Daemon`] method blocks, and only this headless host waits, on its channel. Plan
//! step 7 makes the same calls from a GPUI task that reads an async channel.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::mem;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lyncast_core::ClipId;
use lyncast_core::rank::{Index, Timestamp, Visits};

use super::runner::Runner;
use super::store::Store;
use super::{Daemon, Done, Handled, Job, QUIT_WAIT, Success, Task, Work, Writer, quitting};
use crate::apps;
use crate::clip::{Capture, Clips, Copier, Seen};
use crate::config;
use crate::vars::Vars;
use crate::warning;
use crate::wire::{self, Reply, Request};

/// The lock's file name in `$XDG_RUNTIME_DIR`.
const LOCK: &str = "lyncast.lock";

/// How long each read of a request and each write of a reply may wait, so a client that sends
/// or reads slowly is still served.
const TIMEOUT: Duration = Duration::from_secs(5);

/// How long the start waits for its warnings to reach stderr before it binds the socket, so a
/// client that the socket answers finds them there, and how long Quit waits for its warnings and
/// its replies before the process exits.
const LAST_WRITES: Duration = Duration::from_secs(1);

/// The most bytes a request line may hold, with its newline.
const MAX_REQUEST: u64 = 1 << 20;

/// What the owner loop reads.
enum Event {
    Request(Request, ReplyTo),
    /// A worker thread's job ended.
    Finished(Success, Result<Done, String>, ReplyTo),
    /// A writer ended, after Quit.
    WriterEnded(Writer),
    /// The writer of the clips could not save a new row.
    Unsaved(ClipId),
    /// The capture thread saw an offer, or ended.
    Seen(Seen),
}

/// Why the owner loop stops serving.
enum Stop {
    /// A Quit command, whose reply goes out once the daemon has stopped.
    Quit(Reply, ReplyTo),
    /// The compositor has gone.
    Closed,
}

/// Where a request's reply goes: the connection thread that read it, which says when the reply
/// is written.
struct ReplyTo {
    reply: Sender<Reply>,
    written: Receiver<()>,
}

/// The replies that went to connection threads and may not be written yet. Quit waits for
/// them, because the process exit would cut them off.
#[derive(Default)]
struct Unwritten(Vec<Receiver<()>>);

impl Unwritten {
    fn send(&mut self, reply_to: ReplyTo, reply: Reply) {
        self.0
            .retain(|written| written.try_recv() == Err(TryRecvError::Empty));
        // The send fails only when the connection's thread is gone, with no one to tell.
        if reply_to.reply.send(reply).is_ok() {
            self.0.push(reply_to.written);
        }
    }

    /// Waits until each reply is written, or its thread gives up, or `deadline` passes.
    fn wait(self, deadline: Instant) {
        for written in self.0 {
            let _ = written.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        }
    }
}

/// Claims the socket and serves until a Quit command. The lock comes first, so a second daemon
/// fails before it scans or reads `ranking.json` and `clips.sqlite3`, and only the lock holder
/// removes a stale socket and binds. `record` swaps the runner for a recorder that writes to that
/// file.
pub fn run(record: Option<PathBuf>) -> Result<(), String> {
    warning::start().map_err(not_started)?;
    let runtime = wire::runtime_dir()?;
    let lock = lock(&runtime.join(LOCK))?;
    let vars = Vars::read();
    let config = config::read(vars.config_home().as_deref().map(Path::new));
    let index = Index::new(apps::scan(&vars));
    let (events, inbox) = mpsc::channel();
    let ended = |writer: Writer| {
        let events = events.clone();
        move || {
            let _ = events.send(Event::WriterEnded(writer));
        }
    };
    let data_home = vars.data_home().map(PathBuf::from);
    let (visits, store) = match &data_home {
        Some(dir) => Store::open(dir, ended(Writer::Visits)).map_err(not_started)?,
        None => (Visits::default(), None),
    };
    let unsaved = {
        let events = events.clone();
        move |id| {
            let _ = events.send(Event::Unsaved(id));
        }
    };
    let clips = Clips::open(
        data_home.as_deref(),
        config.retention,
        now(),
        unsaved,
        ended(Writer::Clips),
    )
    .map_err(not_started)?;
    let (capture, copier) = match vars.wayland_display {
        Some(_) => {
            let events = events.clone();
            let seen = move |seen| {
                let _ = events.send(Event::Seen(seen));
            };
            let (capture, copier) =
                Capture::start(seen).map_err(|error| format!("not started: {error}"))?;
            (Some(capture), Some(copier))
        }
        None => (None, None),
    };
    let daemon = Daemon::new(vars, index, visits, store, clips, capture);
    let socket = runtime.join(wire::SOCKET);
    warning::flush(Instant::now() + LAST_WRITES);
    let listener = bind(&socket)?;
    let host = Host {
        events,
        runner: record.map_or(Runner::Real, Runner::recorder),
        copier,
        unwritten: Unwritten::default(),
    };
    serve(daemon, host, inbox, listener, Claim { lock, socket })
}

/// The reason the daemon gives when a thread it needs cannot start.
fn not_started(error: io::Error) -> String {
    format!("not started: {error}")
}

/// The lock and the socket path, which Quit gives up.
struct Claim {
    lock: File,
    socket: PathBuf,
}

/// Takes an exclusive `flock` on `path`. The kernel releases it when the file closes, which
/// happens however the process exits. The open does not wait for a FIFO's reader, so a FIFO at
/// `path` fails at once, and any other file that is not regular, such as a link to a device, is
/// refused.
fn lock(path: &Path) -> Result<File, String> {
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
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err("a daemon is already running".to_owned()),
        Err(fs::TryLockError::Error(error)) => Err(failed(error)),
    }
}

/// Removes a socket that a daemon left when it died, then binds a new one. Only the lock holder
/// calls this.
fn bind(socket: &Path) -> Result<UnixListener, String> {
    let failed = |error: io::Error| format!("{}: {error}", socket.display());
    match fs::remove_file(socket) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(failed(error)),
        _ => {}
    }
    UnixListener::bind(socket).map_err(failed)
}

/// The owner loop. It serves until a Quit command or until the compositor goes, and then stops in
/// the order docs/design/daemon.md gives: no new connections, the work in flight, the last
/// writes, the reply. The daemon keeps answering while it stops.
fn serve(
    mut daemon: Daemon,
    mut host: Host,
    inbox: Receiver<Event>,
    listener: UnixListener,
    claim: Claim,
) -> Result<(), String> {
    let accepted = host.events.clone();
    thread::Builder::new()
        .spawn(move || accept(&listener, &accepted))
        .map_err(not_started)?;
    let stop = loop {
        let event = inbox.recv().expect("the host holds a sender");
        if let Some(stop) = host.dispatch(&mut daemon, event) {
            break stop;
        }
    };
    let _ = fs::remove_file(&claim.socket);
    let deadline = Instant::now() + QUIT_WAIT;
    while !daemon.stopped() {
        // A queued event comes back at once even when the wait is zero, so the deadline is
        // checked before each one.
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            break;
        }
        let Ok(event) = inbox.recv_timeout(wait) else {
            break;
        };
        // The daemon fails every request now, and the capture thread has ended or been
        // cancelled, so no second stop comes back.
        host.dispatch(&mut daemon, event);
    }
    // The daemon takes no more events, so each one still queued fails.
    for event in inbox.try_iter() {
        host.refuse(event);
    }
    // The lock goes after the last write and before the reply, so a daemon started after the
    // reply can take it. A daemon that has not stopped may still be writing, so it keeps the lock
    // until the process exits, and a daemon started in its place never writes ranking.json at
    // the same time.
    if daemon.stopped() {
        drop(claim.lock);
    } else {
        mem::forget(claim.lock);
    }
    let deadline = Instant::now() + LAST_WRITES;
    warning::flush(deadline);
    let closed = match stop {
        Stop::Quit(reply, reply_to) => {
            host.unwritten.send(reply_to, reply);
            Ok(())
        }
        Stop::Closed => Err("the Wayland connection closed".to_owned()),
    };
    host.unwritten.wait(deadline);
    closed
}

/// What the owner loop holds beside the daemon.
struct Host {
    /// A sender for the worker threads.
    events: Sender<Event>,
    runner: Runner,
    /// The copier, which runs one copy at a time. `None` without a compositor.
    copier: Option<Copier>,
    unwritten: Unwritten,
}

impl Host {
    /// Hands one event to the daemon and sends the reply it gives, or starts the job it hands
    /// back. Gives back why the daemon stops serving: the reply to a Quit, which waits, or the
    /// compositor's end.
    fn dispatch(&mut self, daemon: &mut Daemon, event: Event) -> Option<Stop> {
        match event {
            Event::Request(request, reply_to) => match daemon.handle(request, now()) {
                Handled::Reply(reply) => self.unwritten.send(reply_to, reply),
                Handled::Job(job) => self.start(daemon, job, reply_to),
                Handled::Quit(reply) => return Some(Stop::Quit(reply, reply_to)),
            },
            Event::Seen(Seen::Entry(entry)) => daemon.captured(entry, now()),
            Event::Seen(Seen::Ended) => daemon.capture_ended(),
            Event::Seen(Seen::Closed) => {
                daemon.compositor_closed();
                return Some(Stop::Closed);
            }
            Event::Finished(success, done, reply_to) => {
                self.unwritten
                    .send(reply_to, daemon.finish(success, done, now()));
            }
            Event::WriterEnded(writer) => daemon.writer_ended(writer),
            Event::Unsaved(id) => daemon.unsaved(id),
        }
        None
    }

    /// Answers an event that is still queued once Quit stops waiting, without handing it to the
    /// daemon. A job's result is not taken either, so its client hears that the daemon is
    /// quitting, even when its program started.
    fn refuse(&mut self, event: Event) {
        if let Event::Request(_, reply_to) | Event::Finished(_, _, reply_to) = event {
            self.unwritten.send(reply_to, quitting());
        }
    }

    /// Runs the job's task on a worker thread, or hands its copy to the copier, and its result
    /// comes back as an event. A task whose thread cannot start finishes at once, failed.
    fn start(&mut self, daemon: &mut Daemon, job: Job, reply_to: ReplyTo) {
        let Job { work, success } = job;
        let task = match work {
            Work::Task(task) => task,
            Work::Copy(payload) => {
                let copier = self
                    .copier
                    .as_ref()
                    .expect("the daemon hands out a copy only with a compositor");
                let events = self.events.clone();
                return copier.copy(payload, move |copied| {
                    let done = copied.map(|()| Done::Copied);
                    let _ = events.send(Event::Finished(success, done, reply_to));
                });
            }
        };
        let (events, runner) = (self.events.clone(), self.runner.clone());
        // The task goes to the thread once it runs, so a thread that cannot start leaves it here.
        let (hand, handed) = mpsc::channel::<(Task, Success, ReplyTo)>();
        let started = thread::Builder::new().spawn(move || {
            if let Ok((task, success, reply_to)) = handed.recv() {
                let done = task.run(&runner);
                let _ = events.send(Event::Finished(success, done, reply_to));
            }
        });
        match started {
            Ok(_) => {
                let _ = hand.send((task, success, reply_to));
            }
            Err(error) => {
                let done = Err(format!("not run: {error}"));
                let reply = daemon.finish(success, done, now());
                self.unwritten.send(reply_to, reply);
            }
        }
    }
}

fn accept(listener: &UnixListener, events: &Sender<Event>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            // An error such as EMFILE repeats until a descriptor frees, so wait instead of spinning.
            thread::sleep(Duration::from_millis(10));
            continue;
        };
        let events = events.clone();
        // A connection whose thread cannot start, or that fails to read or write, ends alone.
        let _ = thread::Builder::new().spawn(move || connection(stream, &events));
    }
}

/// Reads one request line, hands it to the owner loop, and writes the reply line. A line that
/// does not parse gets a `failed` reply here, and the daemon keeps serving.
fn connection(stream: UnixStream, events: &Sender<Event>) -> io::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_until(b'\n', &mut line)?;
    let (written, written_rx) = mpsc::channel();
    let reply = match parse(&line) {
        Ok(request) => {
            let (reply, reply_rx) = mpsc::channel();
            let reply_to = ReplyTo {
                reply,
                written: written_rx,
            };
            // Either fails only once the owner loop has ended at a Quit. A failed send hands the
            // event back, and its reply sender would keep the wait open, so `ok` drops it first.
            let asked = events.send(Event::Request(request, reply_to)).ok();
            asked
                .and_then(|()| reply_rx.recv().ok())
                .unwrap_or_else(quitting)
        }
        Err(reason) => Reply::Failed { reason },
    };
    let mut out = serde_json::to_vec(&reply).expect("a reply serializes");
    out.push(b'\n');
    (&stream).write_all(&out)?;
    let _ = written.send(());
    Ok(())
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
fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}
