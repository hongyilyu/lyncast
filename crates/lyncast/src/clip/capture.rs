//! The capture thread. It owns the `wl-clipboard-rs` `Watcher`, whose `next_event` blocks with
//! no timeout, so no other thread waits on the compositor. It drops lyncast's own writes and the
//! password manager's, reads one representation of every other offer, up to 25 MiB, and hands
//! each entry to the daemon. It neither writes nor syncs.
//! docs/design/clipboard-and-paste.md describes the order.

use std::io::{self, ErrorKind, Read};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use wl_clipboard_rs::paste::{self, Seat};
use wl_clipboard_rs::utils::PASSWORD_MANAGER_HINT_MIME_TYPE;
use wl_clipboard_rs::watch::{CancelHandle, ClipboardEvent, ClipboardType, Watcher};

use super::{Entry, GNOME_FILES, MAX_BYTES, OWN, URI_LIST};

/// How long the reads of one offer may take in all before the entry is dropped.
const DEADLINE: Duration = Duration::from_secs(2);

/// What the capture thread tells the daemon.
pub enum Seen {
    Entry(Entry),
    /// The `Watcher` was cancelled, after Quit, and the thread has ended.
    Ended,
    /// The `Watcher` failed, so the compositor has gone, and the thread has ended.
    Closed,
}

/// The handle to the capture thread.
pub struct Capture {
    cancel: CancelHandle,
    /// Set once Quit has cancelled the thread, which then reads no offer that waited behind the
    /// one in hand. `Watcher::next_event` hands back an offer it has queued without looking at
    /// its cancel pipe.
    cancelled: Arc<AtomicBool>,
}

impl Capture {
    /// Connects to the compositor that `WAYLAND_DISPLAY` names and starts the capture thread,
    /// which calls `seen` with each entry. Fails when the compositor cannot be reached or the
    /// thread cannot start.
    pub fn start(mut seen: impl FnMut(Seen) + Send + 'static) -> Result<Capture, String> {
        let mut watcher = Watcher::new(ClipboardType::Regular, Seat::Unspecified)
            .map_err(|error| error.to_string())?;
        let cancel = watcher.cancel_handle();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        thread::Builder::new()
            .spawn(move || {
                let ending = watch(&mut watcher, &stop, &mut seen);
                seen(ending);
            })
            .map_err(|error| error.to_string())?;
        Ok(Capture { cancel, cancelled })
    }

    /// Stops the thread after the offer in hand.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.cancel.cancel();
    }
}

/// Hands every entry to `seen` until the `Watcher` is cancelled or fails.
fn watch(watcher: &mut Watcher, cancelled: &AtomicBool, seen: &mut impl FnMut(Seen)) -> Seen {
    loop {
        let event = watcher.next_event();
        if cancelled.load(Ordering::Acquire) {
            return Seen::Ended;
        }
        match event {
            Ok(Some(ClipboardEvent::Changed {
                mime_types,
                mut offer,
                ..
            })) => {
                let skipped = |mime: &String| {
                    mime.starts_with(OWN) || mime == PASSWORD_MANAGER_HINT_MIME_TYPE
                };
                if mime_types.iter().any(skipped) {
                    continue;
                }
                if let Some(entry) = entry(&mime_types, |mime| offer.receive(mime).ok()) {
                    seen(Seen::Entry(entry));
                }
            }
            Ok(Some(ClipboardEvent::Cleared { .. })) => {}
            Ok(None) => return Seen::Ended,
            Err(_) => return Seen::Closed,
        }
    }
}

/// The offer's one representation: files when its list names a local file, else an image, else
/// text, with the pipe that `receive` gives for each type it reads. `None` when it offers none of
/// them, or when a read fails, the reads run past [`DEADLINE`] in all, or they find nothing to
/// keep.
fn entry<P: Read + AsFd>(
    types: &[String],
    mut receive: impl FnMut(&str) -> Option<P>,
) -> Option<Entry> {
    let deadline = Instant::now() + DEADLINE;
    let mut read = |mime: &str| read_pipe(receive(mime)?, deadline);
    let has = |mime: &str| types.iter().any(|offered| offered == mime);
    if let Some(list_type) = [URI_LIST, GNOME_FILES].into_iter().find(|&mime| has(mime)) {
        let list = String::from_utf8(read(list_type)?).ok()?;
        if let Some(files) = Entry::files(&list, list_type.to_owned()) {
            return Some(files);
        }
    }
    let image = types
        .iter()
        .find(|mime| mime.as_str() == "image/png")
        .or_else(|| types.iter().find(|mime| mime.starts_with("image/")));
    if let Some(mime) = image {
        let bytes = read(mime).filter(|bytes| !bytes.is_empty())?;
        return Some(Entry::image(bytes, mime.clone()));
    }
    let mime = paste::select_mime_type(types.to_vec(), paste::MimeType::Text)?;
    let text = String::from_utf8(read(&mime)?).ok()?;
    if text.is_empty() {
        return None;
    }
    Some(Entry::text(&text, mime))
}

/// The bytes the source writes to `pipe`, read until it closes its end. `None` past
/// [`MAX_BYTES`], past `deadline`, or on an error, because a source that is stopped or replaced
/// never closes it.
fn read_pipe(mut pipe: impl Read + AsFd, deadline: Instant) -> Option<Vec<u8>> {
    set_nonblocking(&pipe).ok()?;
    let mut bytes = Vec::new();
    let mut chunk = vec![0; 64 << 10];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || !readable(&pipe, left).ok()? {
            return None;
        }
        match pipe.read(&mut chunk) {
            Ok(0) => return Some(bytes),
            Ok(count) if bytes.len() + count > MAX_BYTES => return None,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
            Err(_) => return None,
        }
    }
}

fn set_nonblocking(pipe: &impl AsFd) -> io::Result<()> {
    let fd = pipe.as_fd().as_raw_fd();
    // SAFETY: fcntl on a descriptor the pipe owns, which lives through both calls.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Whether the pipe may have bytes or an end to read within `wait`. A signal ends the wait early
/// with `true`, and the read that follows finds nothing and waits again.
fn readable(pipe: &impl AsFd, wait: Duration) -> io::Result<bool> {
    let mut poll = libc::pollfd {
        fd: pipe.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = libc::c_int::try_from(wait.as_millis().max(1)).unwrap_or(libc::c_int::MAX);
    // SAFETY: one pollfd that lives through the call.
    match unsafe { libc::poll(&mut poll, 1, millis) } {
        ready if ready < 0 => {
            let error = io::Error::last_os_error();
            match error.kind() {
                ErrorKind::Interrupted => Ok(true),
                _ => Err(error),
            }
        }
        ready => Ok(ready > 0),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// The reads of one offer share its 2 s. Here the list answers after 1.5 s and names no local
    /// file, and the text never answers, so the entry is dropped once 2 s have passed in all, not
    /// 2 s after the text's read began. A seed's source answers every read of its types or none,
    /// so no block can hold up one read and not the other, and this test gives the pipes itself.
    #[test]
    fn the_reads_of_one_offer_share_its_deadline() {
        let (list, mut list_end) = io::pipe().unwrap();
        let (text, text_end) = io::pipe().unwrap();
        let answer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(1500));
            list_end.write_all(b"https://example.org/a").unwrap();
        });
        let mut pipes = vec![(URI_LIST, list), ("text/plain;charset=utf-8", text)];
        let types: Vec<String> = pipes.iter().map(|(mime, _)| mime.to_string()).collect();
        let mut read = Vec::new();
        let started = Instant::now();
        let entry = entry(&types, |mime| {
            read.push(mime.to_owned());
            let at = pipes.iter().position(|(offered, _)| *offered == mime)?;
            Some(pipes.remove(at).1)
        });
        let took = started.elapsed();
        assert!(entry.is_none(), "the offer gives no entry");
        assert_eq!(read, types, "the list is read, then the text");
        assert!(
            took < DEADLINE + Duration::from_millis(500),
            "the offer's reads took {took:?}"
        );
        drop(text_end);
        answer.join().unwrap();
    }
}
