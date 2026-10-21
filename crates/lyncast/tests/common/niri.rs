//! The fake niri of a block's `$ fake-niri` lines: a listener at `$RUNTIME/niri.sock` that reads
//! each request line as niri does, appends it to `$RUNTIME/niri.log` before it answers, and
//! answers as its line says.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// How the fake answers each request it reads.
#[derive(Clone, Debug)]
pub enum Answer {
    /// With this line.
    Line(String),
    /// Never, holding the connection open.
    Stall,
    /// By closing the connection.
    Hangup,
    /// By closing the connection unread, once a request has come.
    Reset,
    /// With this line, one byte every 150 ms.
    Drip(String),
    /// With 2 MiB and no newline, and then by closing the connection.
    Flood,
}

impl Answer {
    /// The answer that the rest of a `$ fake-niri` line names. With nothing, niri's `Handled`.
    pub fn read(rest: &str) -> Answer {
        match rest {
            "" => Answer::Line(r#"{"Ok":"Handled"}"#.to_owned()),
            "stall" => Answer::Stall,
            "hangup" => Answer::Hangup,
            "reset" => Answer::Reset,
            "flood" => Answer::Flood,
            _ => match rest.strip_prefix("drip ") {
                Some(line) => Answer::Drip(line.to_owned()),
                None => Answer::Line(rest.to_owned()),
            },
        }
    }
}

/// A fake niri that listens until it is dropped.
pub struct FakeNiri {
    socket: PathBuf,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    connections: Arc<Mutex<Vec<Connection>>>,
}

/// A connection's thread, and a clone of its stream, which the drop shuts down.
struct Connection {
    stream: UnixStream,
    serving: JoinHandle<()>,
}

impl FakeNiri {
    /// Listens at `socket`, logging to `log`, after removing a fake that left its socket there.
    pub fn start(socket: &Path, log: &Path, answer: Answer) -> FakeNiri {
        let _ = fs::remove_file(socket);
        let listener = UnixListener::bind(socket).expect("the fake niri binds");
        let stop = Arc::new(AtomicBool::new(false));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let accept = {
            let (stop, connections, log) =
                (Arc::clone(&stop), Arc::clone(&connections), log.to_owned());
            thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    if let Answer::Reset = answer {
                        unread(&stream);
                        continue;
                    }
                    let held = stream.try_clone().expect("the stream clones");
                    let (log, answer) = (log.clone(), answer.clone());
                    let serving = thread::spawn(move || serve(stream, &log, &answer));
                    connections.lock().unwrap().push(Connection {
                        stream: held,
                        serving,
                    });
                }
            })
        };
        FakeNiri {
            socket: socket.to_owned(),
            stop,
            accept: Some(accept),
            connections,
        }
    }

    /// Leaves a socket file at `socket` that nothing listens on, as a niri that has gone leaves.
    pub fn gone(socket: &Path) {
        let _ = fs::remove_file(socket);
        drop(UnixListener::bind(socket).expect("the fake niri binds"));
    }
}

impl Drop for FakeNiri {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wakes the accept thread, which then sees the stop.
        let _ = UnixStream::connect(&self.socket);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for connection in self.connections.lock().unwrap().drain(..) {
            let _ = connection.stream.shutdown(Shutdown::Both);
            let _ = connection.serving.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

/// Waits until the client has written, without reading, so that the close finds bytes unread,
/// which resets the connection where a close after the read ends it (measured).
fn unread(stream: &UnixStream) {
    let mut byte = 0u8;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    // SAFETY: a valid descriptor, and a buffer of one byte that lives through the call.
    unsafe {
        libc::recv(
            stream.as_raw_fd(),
            (&raw mut byte).cast(),
            1,
            libc::MSG_PEEK,
        )
    };
}

/// Reads each request line until the client closes, as niri reads them, and answers each one.
fn serve(stream: UnixStream, log: &Path, answer: &Answer) {
    let mut reader = BufReader::new(&stream);
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let mut logged = fs::File::options()
            .create(true)
            .append(true)
            .open(log)
            .expect("the fake niri's log opens");
        logged.write_all(&line).expect("the fake niri logs");
        let mut out = &stream;
        let sent = match answer {
            Answer::Line(reply) => out.write_all(format!("{reply}\n").as_bytes()),
            Answer::Stall => continue,
            Answer::Reset => unreachable!("the accept thread closes these unread"),
            // The connection list holds a clone of the stream, so only a shutdown closes it.
            Answer::Hangup => {
                let _ = stream.shutdown(Shutdown::Both);
                return;
            }
            Answer::Drip(reply) => format!("{reply}\n").bytes().try_for_each(|byte| {
                thread::sleep(Duration::from_millis(150));
                out.write_all(&[byte])
            }),
            Answer::Flood => {
                let _ = out.write_all(&vec![b'x'; 2 << 20]);
                let _ = stream.shutdown(Shutdown::Both);
                return;
            }
        };
        if sent.is_err() {
            return;
        }
    }
}
