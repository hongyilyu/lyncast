//! The client half of the socket: connect, write one request line, read one reply line.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;

use crate::wire::{self, Reply, Request};

/// Sends `request` to the daemon and returns its reply. A `failed` reply is still a reply.
pub fn send(request: &Request) -> Result<Reply, String> {
    let socket = wire::runtime_dir()?.join(wire::SOCKET);
    let failed = |error: std::io::Error| format!("{}: {error}", socket.display());
    let mut stream = UnixStream::connect(&socket).map_err(|error| match error.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
            format!("no daemon is listening on {}", socket.display())
        }
        _ => failed(error),
    })?;
    let mut line = serde_json::to_vec(request).expect("a request serializes");
    line.push(b'\n');
    stream.write_all(&line).map_err(failed)?;
    let mut reply = Vec::new();
    BufReader::new(stream)
        .read_until(b'\n', &mut reply)
        .map_err(failed)?;
    if reply.is_empty() {
        return Err("the daemon closed the connection without a reply".to_owned());
    }
    serde_json::from_slice(&reply).map_err(|error| format!("bad reply: {error}"))
}

/// The reason a reply that the command does not expect gives.
pub fn refusal(reply: Reply) -> String {
    match reply {
        Reply::Failed { reason } => reason,
        _ => "the daemon sent a reply this command does not expect".to_owned(),
    }
}
