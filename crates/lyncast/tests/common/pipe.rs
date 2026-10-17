use std::io::{ErrorKind, Write};
use std::os::fd::AsRawFd;

/// Writes `byte` to `pipe` until it is full, and gives how many it wrote. Big writes fill the
/// pipe's pages, and single bytes the room that a big write leaves.
pub fn fill(pipe: &mut (impl Write + AsRawFd), byte: u8) -> usize {
    let fd = pipe.as_raw_fd();
    // SAFETY: fcntl on a descriptor the pipe owns, which lives through each call.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    // SAFETY: as above.
    assert_eq!(
        unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    let mut filled = 0;
    for chunk in [vec![byte; 4096], vec![byte]] {
        loop {
            match pipe.write(&chunk) {
                Ok(written) => filled += written,
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => panic!("filling a pipe: {error}"),
            }
        }
    }
    // SAFETY: as above.
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) }, 0);
    filled
}
