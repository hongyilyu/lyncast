//! The client half of the socket: connect, write one request line, read one reply line.

use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;

use crate::wire::{self, Reply, Request};

/// Sends `request` to the daemon and returns its reply. A `failed` reply is still a reply.
pub fn send(request: &Request) -> Result<Reply, String> {
    let socket = wire::runtime_dir()?.join(wire::SOCKET);
    let failed = |error: io::Error| format!("{}: {error}", socket.display());
    let stream = UnixStream::connect(&socket).map_err(|error| match error.kind() {
        ErrorKind::NotFound | ErrorKind::ConnectionRefused => {
            format!("no daemon is listening on {}", socket.display())
        }
        _ => failed(error),
    })?;
    let reply = exchange(stream, request).map_err(failed)?;
    if reply.is_empty() {
        return Err("the daemon closed the connection without a reply".to_owned());
    }
    serde_json::from_slice(&reply).map_err(|error| format!("bad reply: {error}"))
}

/// Writes the request line and reads the reply line, empty when the daemon sent none. A daemon at
/// its cap of connections replies and closes before it reads, so a write that fails still reads.
fn exchange(mut stream: UnixStream, request: &Request) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(request).expect("a request serializes");
    line.push(b'\n');
    let wrote = stream.write_all(&line);
    let mut reply = Vec::new();
    let read = BufReader::new(stream).read_until(b'\n', &mut reply);
    match (wrote, read) {
        (Err(error), _) if reply.is_empty() => Err(error),
        (_, Err(error)) => Err(error),
        _ => Ok(reply),
    }
}

/// The reason a reply that the command does not expect gives.
pub fn refusal(reply: Reply) -> String {
    match reply {
        Reply::Failed { reason } => reason,
        _ => "the daemon sent a reply this command does not expect".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// docs/design/daemon.md's session of 128 connections shows the busy reply, and no block can
    /// time the daemon's close before the client's write.
    #[test]
    fn a_reply_sent_before_the_request_reads_after_the_write_fails() {
        let busy = b"{\"reply\":\"failed\",\"reason\":\"busy: 128 connections are open\"}\n";
        let (client, daemon) = UnixStream::pair().unwrap();
        (&daemon).write_all(busy).unwrap();
        drop(daemon);
        assert_eq!(exchange(client, &Request::Status).unwrap(), busy);
    }
}
