use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::ptr;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{Daemon, Ran, Runtime, poll};

/// What a block's `hold-connections`, `half-connection`, `close-connection`, `deny-threads`, and
/// `allow-threads` lines keep from one line to the next.
#[derive(Default)]
pub struct Connections {
    held: Option<Held>,
    /// The soft limit that `deny-threads` took from the newest daemon.
    allowed: Option<libc::rlim_t>,
}

impl Connections {
    /// Runs one of those lines against the newest daemon and gives what it printed, or `None` for
    /// any other line.
    pub fn line(&mut self, line: &str, newest: Option<&Daemon>, runtime: &Runtime) -> Option<Ran> {
        let words: Vec<&str> = line.strip_prefix("$ ")?.split(' ').collect();
        let printed = match (words.as_slice(), newest) {
            (["hold-connections", n], Some(daemon)) => {
                let n = n
                    .parse()
                    .unwrap_or_else(|_| panic!("not hold-connections <n>: {line}"));
                self.held
                    .get_or_insert_with(Held::new)
                    .hold(daemon.id(), &runtime.socket(), n)
                    .map(|()| Vec::new())
            }
            (["half-connection"], Some(daemon)) => self
                .held
                .get_or_insert_with(Held::new)
                .half_one(daemon.id(), &runtime.socket()),
            (["close-connection"], Some(daemon)) => match &self.held {
                Some(held) => held.close_one(daemon.id()).map(|()| Vec::new()),
                None => Err("no connection is held".to_owned()),
            },
            ([verb @ ("deny-threads" | "allow-threads")], Some(daemon)) => {
                let limited = match (*verb, self.allowed.take()) {
                    ("deny-threads", None) => {
                        limit_threads(daemon.id(), 0).map(|had| self.allowed = Some(had))
                    }
                    ("allow-threads", Some(had)) => limit_threads(daemon.id(), had).map(drop),
                    _ => panic!("deny-threads and allow-threads take turns: {line}"),
                };
                limited
                    .map(|()| Vec::new())
                    .map_err(|error| error.to_string())
            }
            (
                ["hold-connections", _]
                | ["half-connection"]
                | ["close-connection"]
                | ["deny-threads"]
                | ["allow-threads"],
                None,
            ) => Err("no daemon".to_owned()),
            _ => return None,
        };
        Some(match printed {
            Ok(stdout) => (stdout, Vec::new(), Some(0)),
            Err(why) => (Vec::new(), format!("{why}\n").into_bytes(), Some(1)),
        })
    }
}

/// Gives process `pid` a soft `RLIMIT_NPROC` of `soft`, which the kernel checks at each thread the
/// process starts from then on, and gives back the soft limit it had.
fn limit_threads(pid: u32, soft: libc::rlim_t) -> io::Result<libc::rlim_t> {
    let pid = libc::pid_t::try_from(pid).map_err(io::Error::other)?;
    let mut had = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: prlimit reads `limit` and writes `had`, which live through the calls.
    unsafe {
        if libc::prlimit(pid, libc::RLIMIT_NPROC, ptr::null(), &mut had) != 0 {
            return Err(io::Error::last_os_error());
        }
        let limit = libc::rlimit {
            rlim_cur: soft,
            rlim_max: had.rlim_max,
        };
        if libc::prlimit(pid, libc::RLIMIT_NPROC, &limit, ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(had.rlim_cur)
}

/// What each held connection sends: a request with no newline.
const HALF_REQUEST: &[u8] = b"{\"request\":\"status\"";

/// Connections that each sent half a request, which a daemon reads until the block ends. A thread
/// of the test sends each a space every second, so the 5 s that the daemon waits for each byte
/// never run out.
struct Held {
    fed: Arc<Mutex<Fed>>,
    stop: Option<Sender<()>>,
    feeder: Option<thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Fed {
    /// The connections of [`Held::hold`], each with the thread of the daemon that reads it, which
    /// [`Held::close_one`] closes from the newest.
    held: Vec<(UnixStream, String)>,
    /// The connections of [`Held::half_one`], which stay open until the block ends.
    halves: Vec<UnixStream>,
}

impl Held {
    fn new() -> Held {
        let fed: Arc<Mutex<Fed>> = Arc::default();
        let (stop, stopped) = mpsc::channel::<()>();
        let feeding = Arc::clone(&fed);
        let feeder = thread::spawn(move || {
            while stopped.recv_timeout(Duration::from_secs(1)) == Err(RecvTimeoutError::Timeout) {
                let fed = feeding.lock().unwrap();
                for stream in fed.held.iter().map(|(stream, _)| stream).chain(&fed.halves) {
                    let _ = (&*stream).write_all(b" ");
                }
            }
        });
        Held {
            fed,
            stop: Some(stop),
            feeder: Some(feeder),
        }
    }

    /// Opens `n` connections to `socket`, one at a time, each of which a new thread of process
    /// `pid` reads, as [`open`] waits. A connection that the daemon answers instead, as it answers
    /// one past its cap, is opened again, since the connection of [`Daemon::serving`]'s wait for
    /// the socket can hold its place for a while.
    fn hold(&self, pid: u32, socket: &Path, n: usize) -> Result<(), String> {
        for _ in 0..n {
            let held = poll(|| match open(pid, socket) {
                Ok((stream, Answer::Reader(reader))) => Some(Ok((stream, reader))),
                Ok((_, Answer::Reply(_))) => None,
                Err(error) => Some(Err(error)),
            })
            .unwrap_or_else(|| Err(format!("process {pid} answers each new connection")))?;
            self.fed.lock().unwrap().held.push(held);
        }
        Ok(())
    }

    /// Opens one more connection to `socket`, which stays open until the block ends, and gives
    /// the reply line that the daemon sends it, or nothing once a new thread of process `pid`
    /// reads it, as [`open`] waits.
    fn half_one(&self, pid: u32, socket: &Path) -> Result<Vec<u8>, String> {
        let (stream, answer) = open(pid, socket)?;
        self.fed.lock().unwrap().halves.push(stream);
        Ok(match answer {
            Answer::Reader(_) => Vec::new(),
            Answer::Reply(reply) => reply,
        })
    }

    /// Closes the newest connection of [`Held::hold`] and waits, as [`poll`] does, until the
    /// thread of process `pid` that read it has ended.
    fn close_one(&self, pid: u32) -> Result<(), String> {
        let (stream, reader) = self
            .fed
            .lock()
            .unwrap()
            .held
            .pop()
            .ok_or_else(|| "no connection is held".to_owned())?;
        drop(stream);
        let task = format!("/proc/{pid}/task/{reader}");
        poll(|| (!Path::new(&task).exists()).then_some(()))
            .ok_or_else(|| format!("thread {reader} of process {pid} has not ended"))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(feeder) = self.feeder.take() {
            let _ = feeder.join();
        }
    }
}

/// What the daemon did with a new connection.
enum Answer {
    /// A thread that the daemon did not have before the connection waits to read it.
    Reader(String),
    /// The daemon sent this line, or closed the connection with nothing.
    Reply(Vec<u8>),
}

/// Opens a connection to `socket` that sends [`HALF_REQUEST`], and waits, as [`poll`] does, until
/// process `pid` reads it on a new thread or answers it.
fn open(pid: u32, socket: &Path) -> Result<(UnixStream, Answer), String> {
    let before = threads(pid);
    let stream =
        UnixStream::connect(socket).map_err(|error| format!("{}: {error}", socket.display()))?;
    stream.set_nonblocking(true).unwrap();
    let _ = (&stream).write_all(HALF_REQUEST);
    let mut reply = Vec::new();
    let answer = poll(|| {
        let mut chunk = [0; 256];
        match (&stream).read(&mut chunk) {
            Ok(count @ 1..) => {
                reply.extend_from_slice(&chunk[..count]);
                reply
                    .ends_with(b"\n")
                    .then(|| Answer::Reply(mem::take(&mut reply)))
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => threads(pid)
                .into_iter()
                .find(|tid| !before.contains(tid) && reads(pid, tid))
                .map(Answer::Reader),
            Ok(0) | Err(_) => Some(Answer::Reply(mem::take(&mut reply))),
        }
    })
    .ok_or_else(|| "no reply came, and no new thread reads the connection".to_owned())?;
    Ok((stream, answer))
}

/// The IDs of the threads of process `pid`.
fn threads(pid: u32) -> HashSet<String> {
    fs::read_dir(format!("/proc/{pid}/task"))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect()
}

/// Whether thread `tid` of process `pid` waits in `recvfrom`, as the thread of a connection waits
/// for the rest of its request.
fn reads(pid: u32, tid: &str) -> bool {
    let call = fs::read_to_string(format!("/proc/{pid}/task/{tid}/syscall"));
    let number = call
        .ok()
        .and_then(|call| call.split(' ').next()?.parse().ok());
    number == Some(libc::SYS_recvfrom)
}
