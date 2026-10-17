//! The socket and the threads around the daemon's state. An accept thread starts one thread per
//! connection, for [`MAX_CONNECTIONS`] at most at once, which reads the request line, parses it,
//! and passes it to the owner loop over a channel. The owner loop holds the [`Daemon`] and never
//! touches a socket. Work that blocks runs on a worker thread, or on the copier thread for a copy,
//! and its result comes back over the same channel. The capture thread hands over each entry
//! there, and says there when it has ended, after Quit or because the compositor has gone. The
//! writer of the clips hands over each new row it could not save, and each writer says there when
//! it has ended. No [`Daemon`] method blocks, and only this headless host waits, on its channel.
//! Plan step 7 makes the same calls from a GPUI task that reads an async channel.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::mem;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lyncast_core::ClipId;
use lyncast_core::rank::{Index, Timestamp, Visits};

use super::runner::Runner;
use super::store::Store;
use super::{
    Answer, Daemon, Done, Handled, Job, QUIT_WAIT, QUITTING, Success, Task, Work, WriterKind,
    quitting,
};
use crate::apps;
use crate::clip::{Capture, Clips, Copier, Payload, Seen};
use crate::config;
use crate::permit::Permits;
use crate::vars::Vars;
use crate::view;
use crate::warning;
use crate::wire::{self, Reply, Request};

/// The lock's file name in `$XDG_RUNTIME_DIR`.
const LOCK: &str = "lyncast.lock";

/// How long each read of a request and each write of a reply may wait, so a client that sends
/// or reads slowly is still served.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The start's wait for its warnings to reach stderr before it binds the socket, so a client that
/// the socket answers finds them there.
const WARNINGS_BEFORE_BIND: Duration = Duration::from_secs(1);

/// Quit's wait for its warnings to reach stderr before it replies.
const WARNINGS_BEFORE_REPLY: Duration = Duration::from_secs(1);

/// Quit's wait for the replies to be written, after its own goes out, before the process exits,
/// which would cut them off.
const REPLIES_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// The wait of a daemon that stops on an error for its warnings and its last line before it exits.
const LAST_LINE_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// The wait of the one write of a daemon whose warnings thread did not start.
const ALONE_WRITE_WAIT: Duration = Duration::from_secs(1);

/// The most bytes a request line may hold, with its newline.
const MAX_REQUEST: u64 = 1 << 20;

/// The most connections open at once. Each holds a thread, and [`TIMEOUT`] bounds each read, not
/// the request, so a client that sends a byte at a time holds its thread for good.
const MAX_CONNECTIONS: u64 = 128;

/// What the owner loop reads.
enum Event {
    Request(Request, ReplyTo),
    /// A worker thread's task or the copier's copy ended.
    Finished(Success, Result<Done, String>, ReplyTo),
    /// A writer ended, after Quit.
    WriterEnded(WriterKind),
    Unsaved(ClipId),
    Seen(Seen),
}

enum Stop {
    Quit(Answer, ReplyTo),
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

/// Runs the daemon until a Quit command, or until it stops on an error, and gives its exit code.
/// `record` swaps the runner for a recorder that writes to that file.
pub fn run(record: Option<PathBuf>) -> ExitCode {
    if let Err(error) = warning::start() {
        warning::write_alone(format_args!("{}", not_started(error)), ALONE_WRITE_WAIT);
        return ExitCode::FAILURE;
    }
    match start(record) {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            warning::last(format_args!("{reason}"));
            warning::flush(Instant::now() + LAST_LINE_BEFORE_EXIT);
            ExitCode::FAILURE
        }
    }
}

/// Claims the socket and serves until a Quit command. The lock comes first, so a second daemon
/// fails before it scans or reads `ranking.json`, and only the lock holder removes a stale
/// socket and binds.
fn start(record: Option<PathBuf>) -> Result<(), String> {
    let runtime = wire::runtime_dir()?;
    let lock = lock(&runtime.join(LOCK))?;
    let vars = Vars::read().map_err(not_started)?;
    let config = config::read(vars.config_home().as_deref().map(Path::new));
    let index = Index::new(apps::scan(&vars));
    let (events, inbox) = mpsc::channel();
    let ended = |writer: WriterKind| {
        let events = events.clone();
        move || {
            let _ = events.send(Event::WriterEnded(writer));
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
            let _ = events.send(Event::Unsaved(id));
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
                let _ = events.send(Event::Seen(seen));
            };
            let (capture, copier) = Capture::start(clips.pending(), seen).map_err(not_started)?;
            (Some(capture), Some(copier))
        }
        None => (None, None),
    };
    let daemon = Daemon::new(vars, index, visits, store, clips, capture);
    let socket = runtime.join(wire::SOCKET);
    warning::flush(Instant::now() + WARNINGS_BEFORE_BIND);
    let listener = bind(&socket)?;
    let host = Host {
        events,
        runner: record.map_or(Runner::Real, Runner::recorder),
        copier,
        unwritten: Unwritten::default(),
    };
    serve(daemon, host, inbox, listener, Claim { lock, socket })
}

/// The reason the daemon gives when `WAYLAND_SOCKET` is set, when a thread it needs cannot start,
/// or when the compositor cannot be reached or offers no seat or neither data-control protocol.
fn not_started(error: impl fmt::Display) -> String {
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
    daemon.started(now());
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
        host.drain(&mut daemon, event);
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
    let stopped = match stop {
        Stop::Quit(answer, reply_to) => {
            warning::flush(Instant::now() + WARNINGS_BEFORE_REPLY);
            host.deliver(reply_to, answer);
            Ok(())
        }
        Stop::Closed => Err("the Wayland connection closed".to_owned()),
    };
    host.unwritten.wait(Instant::now() + REPLIES_BEFORE_EXIT);
    stopped
}

/// What the owner loop holds beside the daemon.
struct Host {
    /// A sender for the worker threads, the copier's results, and the accept thread.
    events: Sender<Event>,
    runner: Runner,
    copier: Option<Copier>,
    unwritten: Unwritten,
}

impl Host {
    /// Hands one event to the daemon while it serves, and sends the answer it gives, or starts
    /// the job it hands back. Gives back the answer to a Quit, which waits, or the compositor's
    /// end.
    fn dispatch(&mut self, daemon: &mut Daemon, event: Event) -> Option<Stop> {
        match event {
            Event::Request(request, reply_to) => match daemon.handle(request, now()) {
                Handled::Quit(answer) => return Some(Stop::Quit(answer, reply_to)),
                handled => self.answer(daemon, handled, reply_to),
            },
            Event::Seen(Seen::Closed) => {
                daemon.compositor_closed();
                return Some(Stop::Closed);
            }
            event => self.drain(daemon, event),
        }
        None
    }

    /// Hands one event to the daemon, and no event here begins a stop. The stop hands it every
    /// event, since the daemon then fails every request and the compositor's end only ends the
    /// capture thread, and [`Host::dispatch`] hands it the events that never begin one.
    fn drain(&mut self, daemon: &mut Daemon, event: Event) {
        match event {
            Event::Request(request, reply_to) => {
                let handled = daemon.handle(request, now());
                self.answer(daemon, handled, reply_to);
            }
            Event::Seen(Seen::Entry(entry)) => daemon.captured(entry, now()),
            Event::Seen(Seen::Ended | Seen::Closed) => daemon.capture_ended(),
            Event::Finished(success, done, reply_to) => {
                let answer = daemon.finish(success, done, now());
                self.deliver(reply_to, answer);
            }
            Event::WriterEnded(writer) => daemon.writer_ended(writer),
            Event::Unsaved(id) => daemon.unsaved(id),
        }
    }

    /// Sends the answer that a request came to, or starts its job. A Quit gets the answer of any
    /// request once the stop has begun, because [`Host::dispatch`] takes the Quit that begins it.
    fn answer(&mut self, daemon: &mut Daemon, handled: Handled, reply_to: ReplyTo) {
        match handled {
            Handled::Answer(answer) => self.deliver(reply_to, answer),
            Handled::Job(job) => self.start(daemon, job, reply_to),
            Handled::Quit(_) => self.deliver(reply_to, quitting()),
        }
    }

    /// Answers an event that is still queued once Quit stops waiting, without handing it to the
    /// daemon. A job's result is not taken either, so its client hears that the daemon is
    /// quitting, even when its program started.
    fn refuse(&mut self, event: Event) {
        if let Event::Request(_, reply_to) | Event::Finished(_, _, reply_to) = event {
            self.deliver(reply_to, quitting());
        }
    }

    fn deliver(&mut self, reply_to: ReplyTo, answer: Answer) {
        self.unwritten.send(reply_to, view::reply(answer));
    }

    /// Runs the job's task on a worker thread, or hands its copy to the copier, and its result
    /// comes back as an event.
    fn start(&mut self, daemon: &mut Daemon, job: Job, reply_to: ReplyTo) {
        let Job { work, success } = job;
        match work {
            Work::Task(task) => self.run(daemon, task, success, reply_to),
            Work::Copy(payload) => self.copy(payload, success, reply_to),
        }
    }

    fn copy(&self, payload: Payload, success: Success, reply_to: ReplyTo) {
        let copier = self
            .copier
            .as_ref()
            .expect("the daemon hands out a copy only with a compositor");
        let events = self.events.clone();
        copier.copy(payload, move |copied| {
            let done = copied.map(|()| Done::Copied);
            let _ = events.send(Event::Finished(success, done, reply_to));
        });
    }

    /// A task whose thread cannot start finishes at once, failed.
    fn run(&mut self, daemon: &mut Daemon, task: Task, success: Success, reply_to: ReplyTo) {
        let (events, runner) = (self.events.clone(), self.runner.clone());
        let started = spawn_with(
            (task, success, reply_to),
            move |(task, success, reply_to)| {
                let done = task.run(&runner);
                let _ = events.send(Event::Finished(success, done, reply_to));
            },
        );
        if let Err(((_, success, reply_to), error)) = started {
            let done = Err(format!("not run: {error}"));
            let answer = daemon.finish(success, done, now());
            self.deliver(reply_to, answer);
        }
    }
}

fn accept(listener: &UnixListener, events: &Sender<Event>) {
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
fn spawn_with<T: Send + 'static>(
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
                .unwrap_or_else(|| Reply::Failed {
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
fn now() -> Timestamp {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    Timestamp(i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}
