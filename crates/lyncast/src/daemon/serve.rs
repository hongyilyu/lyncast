//! The socket and the threads around the daemon's state. An accept thread starts one thread per
//! connection, which reads the request line, parses it, and passes it to the owner loop over a
//! channel. The owner loop holds the [`Daemon`] and never touches a socket. [`Daemon::handle`]
//! never blocks, and only this headless host waits, on its channel. Plan step 7 makes the same
//! calls from a GPUI task that reads an async channel.

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lyncast_core::rank::{Index, Timestamp};

use super::Daemon;
use crate::apps;
use crate::wire::{self, Reply, Request};

/// The lock's file name in `$XDG_RUNTIME_DIR`.
const LOCK: &str = "lyncast.lock";

/// How long a connection may take to send its request or to read its reply.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The most bytes a request line may hold, with its newline.
const MAX_REQUEST: u64 = 1 << 20;

/// What the owner loop reads.
enum Event {
    /// A parsed request, and where its reply goes.
    Request(Request, Sender<Reply>),
}

/// Claims the socket and serves until the process ends. The lock comes first, so a second
/// daemon fails before it scans, and only the lock holder removes a stale socket and binds.
pub fn run() -> Result<(), String> {
    let runtime = wire::runtime_dir()?;
    let _lock = lock(&runtime.join(LOCK))?;
    let daemon = Daemon::new(Index::new(apps::scan(&apps::Vars::read())));
    let listener = bind(&runtime.join(wire::SOCKET))?;
    serve(daemon, listener)
}

/// The reason the daemon gives when a thread it needs cannot start.
fn not_started(error: io::Error) -> String {
    format!("not started: {error}")
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

/// The owner loop. It answers each event in turn, for as long as the accept thread runs.
fn serve(mut daemon: Daemon, listener: UnixListener) -> Result<(), String> {
    let (events, inbox) = mpsc::channel();
    thread::Builder::new()
        .spawn(move || accept(&listener, &events))
        .map_err(not_started)?;
    for event in inbox {
        match event {
            Event::Request(request, reply_to) => {
                // The send fails only when the connection's thread is gone, with no one to tell.
                let _ = reply_to.send(daemon.handle(request, now()));
            }
        }
    }
    Ok(())
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
    let reply = match parse(&line) {
        Ok(request) => {
            let (reply_to, reply) = mpsc::channel();
            // Either fails only when the owner loop has ended and the process is exiting. A
            // failed send hands the event back, and its reply sender would keep the wait open,
            // so `ok` drops it first.
            let asked = events.send(Event::Request(request, reply_to)).ok();
            let Some(reply) = asked.and_then(|()| reply.recv().ok()) else {
                return Ok(());
            };
            reply
        }
        Err(reason) => Reply::Failed { reason },
    };
    let mut out = serde_json::to_vec(&reply).expect("a reply serializes");
    out.push(b'\n');
    (&stream).write_all(&out)
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
