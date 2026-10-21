//! lyncast's client of niri's IPC socket. Each request takes a connection of its own and one
//! compact JSON line each way, typed with niri-ipc's `Request` and `Reply`, and one deadline
//! covers the connect, the write, and the read. niri-ipc's own `Socket::send` waits for its reply
//! with no timeout (niri-ipc 26.4.0 `src/socket.rs:51-61`), so a niri that never answers would
//! hold its caller for good. docs/design/launching.md describes the exchange.

use std::io::{self, ErrorKind, Read, Write};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use niri_ipc::{Action, Reply, Request, Response};

/// How long an action may take in all, from the connect to its answer.
const ACTION_WAIT: Duration = Duration::from_secs(2);

/// How long a question may take in all. niri answers one from state it holds, or after one turn
/// of its event loop.
const ASK_WAIT: Duration = Duration::from_millis(200);

/// The most bytes a reply line may hold, with its newline.
const MAX_REPLY: usize = 1 << 20;

/// Why a request got no answer that lyncast can use.
enum Failure {
    /// The connect failed, so nothing was sent.
    Connect(io::Error),
    /// The write or the read failed. A request cut short is no line, so niri runs none of it.
    Io(io::Error),
    /// The deadline passed. niri may still run a request it has read.
    Timeout,
    /// niri closed the connection before a whole line.
    Closed,
    /// niri answered `Err`.
    Niri(String),
    /// The line is not a reply, or is one to another request.
    Bad(String),
}

/// The line that a request goes to niri as.
pub fn line(request: &Request) -> Vec<u8> {
    let mut line = serde_json::to_vec(request).expect("a niri request serializes");
    line.push(b'\n');
    line
}

/// Sends `action` to the niri at `socket` and waits for its `Handled`, which niri sends once the
/// action has run on its event loop. niri stops its loop in the turn that runs Quit, before it
/// writes the answer, so niri's close is Quit's answer. The error is the reason the CLI prints
/// after `lyncast: `.
pub fn act(socket: Option<&Path>, action: Action) -> Result<(), String> {
    let Some(socket) = socket else {
        return Err("not run: NIRI_SOCKET is not set".to_owned());
    };
    let failed = |failure: Failure| {
        let why = match failure {
            Failure::Connect(error) | Failure::Io(error) => {
                format!("{}: {error}", socket.display())
            }
            Failure::Timeout => format!("niri did not answer within {} s", ACTION_WAIT.as_secs()),
            Failure::Closed => "niri closed the connection without an answer".to_owned(),
            Failure::Niri(message) => format!("niri: {message}"),
            Failure::Bad(error) => format!("niri sent a bad reply: {error}"),
        };
        format!("not run: {why}")
    };
    let quits = matches!(action, Action::Quit { .. });
    match send(socket, &Request::Action(action), ACTION_WAIT) {
        Ok(Response::Handled) => Ok(()),
        // A close with the request unread fails the read with a reset, so niri has read this one.
        Err(Failure::Closed) if quits => Ok(()),
        Ok(_) => Err(failed(Failure::Bad(
            "not an answer to an action".to_owned(),
        ))),
        Err(failure) => Err(failed(failure)),
    }
}

/// The `app_id` of the window that the niri at `socket` says has the focus, or `None` when niri
/// gives no answer within [`ASK_WAIT`], when no window has the focus, as while a layer surface
/// such as the palette has it, or when the window has no app ID.
pub fn focused_app(socket: Option<&Path>) -> Option<String> {
    match send(socket?, &Request::FocusedWindow, ASK_WAIT).ok()? {
        Response::FocusedWindow(window) => window?.app_id,
        _ => None,
    }
}

/// The logical height of the output that the niri at `socket` says has the focus, or `None` when
/// niri gives no answer within [`ASK_WAIT`], when no output has the focus, or when it has no
/// logical output, as when it is off.
pub fn focused_height(socket: Option<&Path>) -> Option<u32> {
    match send(socket?, &Request::FocusedOutput, ASK_WAIT).ok()? {
        Response::FocusedOutput(output) => Some(output?.logical?.height),
        _ => None,
    }
}

/// Sends one request on a connection of its own and reads its one reply line, all within `wait`.
fn send(socket: &Path, request: &Request, wait: Duration) -> Result<Response, Failure> {
    let deadline = Instant::now() + wait;
    let left = || {
        let left = deadline.saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left).ok_or(Failure::Timeout)
    };
    let mut stream = connect(socket, left()?).map_err(|error| match error.kind() {
        // The listener's backlog stayed full until the deadline.
        ErrorKind::WouldBlock => Failure::Timeout,
        _ => Failure::Connect(error),
    })?;
    let bytes = line(request);
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        stream
            .set_write_timeout(Some(left()?))
            .map_err(Failure::Io)?;
        match stream.write(rest) {
            Ok(0) => return Err(Failure::Io(ErrorKind::WriteZero.into())),
            Ok(written) => rest = &rest[written..],
            Err(error) if waits(&error) => {}
            Err(error) => return Err(Failure::Io(error)),
        }
    }
    let mut reply = Vec::new();
    let mut chunk = vec![0; 64 << 10];
    let line = loop {
        if let Some(end) = reply.iter().position(|&byte| byte == b'\n') {
            break &reply[..end];
        }
        if reply.len() == MAX_REPLY {
            return Err(Failure::Bad("no newline in the first 1 MiB".to_owned()));
        }
        stream
            .set_read_timeout(Some(left()?))
            .map_err(Failure::Io)?;
        let room = (MAX_REPLY - reply.len()).min(chunk.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) => return Err(Failure::Closed),
            Ok(read) => reply.extend_from_slice(&chunk[..read]),
            Err(error) if waits(&error) => {}
            Err(error) => return Err(Failure::Io(error)),
        }
    };
    match serde_json::from_slice::<Reply>(line) {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(message)) => Err(Failure::Niri(message)),
        Err(error) => Err(Failure::Bad(error.to_string())),
    }
}

/// A timeout of one read or write, or a signal: the caller checks the deadline and goes on.
fn waits(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

/// Connects to `path`, waiting at most `wait` while the listener's backlog is full. A connect to a
/// full backlog waits for the socket's `SO_SNDTIMEO` and then fails with `EAGAIN` (measured), and
/// std's `UnixStream::connect` sets no timeout first, so it waits for good. A niri whose event
/// loop is stuck accepts nothing.
fn connect(path: &Path, wait: Duration) -> io::Result<UnixStream> {
    let (address, length) = address(path)?;
    // SAFETY: socket takes no pointer.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the descriptor is new, and nothing else owns it.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };
    // A zero timeout means none.
    let wait = wait.max(Duration::from_micros(1));
    let timeout = libc::timeval {
        tv_sec: libc::time_t::try_from(wait.as_secs()).unwrap_or(libc::time_t::MAX),
        tv_usec: libc::suseconds_t::from(wait.subsec_micros()),
    };
    // SAFETY: a valid descriptor, and a timeval that lives through the call.
    let set = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDTIMEO,
            (&raw const timeout).cast(),
            mem::size_of::<libc::timeval>() as libc::socklen_t,
        )
    };
    if set != 0 {
        return Err(io::Error::last_os_error());
    }
    loop {
        // SAFETY: a valid descriptor, and an address of `length` bytes that lives through the
        // call.
        let connected =
            unsafe { libc::connect(socket.as_raw_fd(), (&raw const address).cast(), length) };
        if connected == 0 {
            return Ok(UnixStream::from(socket));
        }
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// The socket address of `path`, and its length, after std's check that the path fits.
fn address(path: &Path) -> io::Result<(libc::sockaddr_un, libc::socklen_t)> {
    SocketAddr::from_pathname(path)?;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: an all-zero sockaddr_un is a valid value.
    let mut address: libc::sockaddr_un = unsafe { mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (to, &from) in address.sun_path.iter_mut().zip(bytes) {
        *to = from as libc::c_char;
    }
    let length = mem::size_of::<libc::sa_family_t>() + bytes.len() + 1;
    Ok((address, length as libc::socklen_t))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    /// A fresh directory under `/tmp`, whose socket path fits in `sun_path` wherever the target
    /// directory is.
    fn scratch() -> PathBuf {
        let mut template = b"/tmp/lyncast-niri.XXXXXX\0".to_vec();
        // SAFETY: `template` is NUL-terminated, and mkdtemp rewrites its X's in place.
        let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
        assert!(!made.is_null(), "mkdtemp: {}", io::Error::last_os_error());
        template.pop();
        PathBuf::from(OsString::from_vec(template))
    }

    fn socket(flags: libc::c_int) -> OwnedFd {
        // SAFETY: socket takes no pointer.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | flags, 0) };
        assert!(fd >= 0, "socket: {}", io::Error::last_os_error());
        // SAFETY: the descriptor is new, and nothing else owns it.
        unsafe { OwnedFd::from_raw_fd(fd) }
    }

    /// A listener at `path` that accepts nothing, with a backlog of `backlog`.
    fn listener(path: &Path, backlog: libc::c_int) -> OwnedFd {
        let (address, length) = address(path).unwrap();
        let listener = socket(libc::SOCK_CLOEXEC);
        // SAFETY: a valid descriptor, and an address of `length` bytes that lives through the
        // call.
        let bound =
            unsafe { libc::bind(listener.as_raw_fd(), (&raw const address).cast(), length) };
        assert_eq!(bound, 0, "bind: {}", io::Error::last_os_error());
        // SAFETY: listen takes no pointer.
        let listening = unsafe { libc::listen(listener.as_raw_fd(), backlog) };
        assert_eq!(listening, 0, "listen: {}", io::Error::last_os_error());
        listener
    }

    /// Connects without waiting until a connect finds the backlog of `path` full, and gives the
    /// connections, which hold the backlog while they live.
    fn fill(path: &Path) -> Vec<OwnedFd> {
        let (address, length) = address(path).unwrap();
        let mut queued = Vec::new();
        loop {
            let queuing = socket(libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK);
            // SAFETY: a valid descriptor, and an address of `length` bytes that lives through the
            // call.
            let connected =
                unsafe { libc::connect(queuing.as_raw_fd(), (&raw const address).cast(), length) };
            if connected != 0 {
                let error = io::Error::last_os_error();
                assert_eq!(error.kind(), ErrorKind::WouldBlock, "connect: {error}");
                return queued;
            }
            queued.push(queuing);
            assert!(queued.len() < 64, "a backlog of 0 held 64 connections");
        }
    }

    /// Runs `request` on a thread of its own, which sends what it gave and how long it took.
    fn timed<T: Send + 'static>(
        request: impl FnOnce() -> T + Send + 'static,
    ) -> mpsc::Receiver<(T, Duration)> {
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let asked = Instant::now();
            let answer = request();
            let _ = done.send((answer, asked.elapsed()));
        });
        result
    }

    /// docs/design/launching.md: a niri whose event loop is stuck accepts no connection, and no
    /// block can fill a backlog of 4,097, so this niri's backlog is as small as it gets. The
    /// test fills it with connects that give up at once, and then an action fails at its
    /// deadline, where std's connect would wait for good, and each question gives up in under
    /// 1 s.
    #[test]
    fn a_niri_that_accepts_nothing_fails_at_the_deadline() {
        let dir = scratch();
        let socket = dir.join("niri.sock");
        let _niri = listener(&socket, 0);
        let _queued = fill(&socket);
        let action = Action::Spawn {
            command: vec!["firefox".to_owned()],
        };
        let sent = socket.clone();
        let failed = timed(move || act(Some(&sent), action)).recv_timeout(ACTION_WAIT * 2);
        let asked = socket.clone();
        let window = timed(move || focused_app(Some(&asked))).recv_timeout(ACTION_WAIT * 2);
        let asked = socket.clone();
        let output = timed(move || focused_height(Some(&asked))).recv_timeout(ACTION_WAIT * 2);
        fs::remove_dir_all(&dir).unwrap();
        let (failed, _) = failed.expect("the action returned");
        assert_eq!(
            failed,
            Err("not run: niri did not answer within 2 s".to_owned())
        );
        let (window, took) = window.expect("the question returned");
        assert_eq!(window, None);
        assert!(took < Duration::from_secs(1), "FocusedWindow took {took:?}");
        let (output, took) = output.expect("the question returned");
        assert_eq!(output, None);
        assert!(took < Duration::from_secs(1), "FocusedOutput took {took:?}");
    }
}
