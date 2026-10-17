//! The copier thread. It runs one copy at a time and numbers each one, and a copy replies once the
//! capture thread reports the marked offer with its number, so the copies' results reach the
//! daemon in the order the compositor took them. docs/design/clipboard-and-paste.md describes a
//! copy.

use std::io;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use lyncast_core::ClipId;
use lyncast_core::clip;
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source as Bytes};

use super::capture::DEADLINE;
use super::db::Reader;
use super::{Body, Content, GNOME_FILES, TEXT, URI_LIST, marker};

/// How long a copy waits for the compositor to take its selection, which wl-clipboard-rs
/// prepares with a roundtrip to the compositor. A copy not prepared by then is never served.
const TAKE: Duration = Duration::from_millis(500);

/// How long a copy the compositor has taken waits for the capture thread to report its marked
/// offer: the longest one offer can hold the thread up, and a margin.
const CONFIRM: Duration = DEADLINE.saturating_add(Duration::from_millis(500));

/// What a copy offers and where its bytes are.
pub struct Payload {
    content: Content,
    mime: String,
    source: Source,
}

/// Where a payload's bytes are.
pub(super) enum Source {
    /// In memory: a text to hand over, or a row's bytes when the rows live in memory.
    Held(Body),
    /// A row's bytes, which the writer of the clips reads.
    Stored(Reader, ClipId),
}

impl Payload {
    pub(super) fn new(content: Content, mime: String, source: Source) -> Payload {
        Payload {
            content,
            mime,
            source,
        }
    }

    /// `text`, which no row may hold yet.
    pub(super) fn text(text: &str) -> Payload {
        Payload {
            content: Content::Text(clip::text_kind(text)),
            mime: TEXT.to_owned(),
            source: Source::Held(Body::Text(text.into())),
        }
    }

    /// The bytes, read through the writer when a row's are stored.
    fn read(self) -> Result<(Content, String, Body), String> {
        let body = match self.source {
            Source::Held(body) => body,
            Source::Stored(reader, id) => reader.read(id)?,
        };
        Ok((self.content, self.mime, body))
    }
}

/// The types a payload offers again, each with its bytes, and the marker of copy `number`. A text
/// offers `text/plain;charset=utf-8`, which wl-clipboard-rs also offers as its other text types.
/// An image offers its type. A file list offers its paths as text, and itself as `text/uri-list`
/// and as a GNOME list. wl-clipboard-rs offers its types in no set order, but it fills its other
/// text types from the first text source, so the paths go first, and a paste into a text field
/// gets them. The marker holds the first source's bytes.
fn offers(content: Content, mime: String, body: &Body, number: u64) -> Vec<(String, Vec<u8>)> {
    let bytes = body.bytes();
    let mut offers = match (content, body) {
        (_, Body::Image(_)) => vec![(mime, bytes.to_vec())],
        (Content::Files, Body::Text(list)) => vec![
            (TEXT.to_owned(), clip::paths(list).into_bytes()),
            (URI_LIST.to_owned(), bytes.to_vec()),
            (
                GNOME_FILES.to_owned(),
                [b"copy\n".as_slice(), bytes].concat(),
            ),
        ],
        (_, Body::Text(_)) => vec![(TEXT.to_owned(), bytes.to_vec())],
    };
    let marked = offers[0].1.clone();
    offers.push((marker(number), marked));
    offers
}

/// A copy queued for the copier, and what to do with its result.
type Queued = (Payload, Box<dyn FnOnce(Result<(), String>) + Send>);

/// The handle to the copier thread.
pub struct Copier {
    queue: Sender<Queued>,
}

impl Copier {
    /// Starts the thread, which numbers its copies from 1 and takes the number of each marked
    /// offer that the capture thread reports from `marks`.
    pub fn start(marks: Receiver<u64>) -> io::Result<Copier> {
        let (queue, copies) = mpsc::channel::<Queued>();
        thread::Builder::new().spawn(move || {
            let mut waiting = None;
            for (number, (payload, done)) in (1..).zip(copies) {
                done(copy(number, payload, &marks, &mut waiting));
            }
        })?;
        Ok(Copier { queue })
    }

    /// Queues a copy of `payload`. `done` gets its result once the capture thread has seen the
    /// compositor take the selection, or once the copy has failed.
    pub fn copy(&self, payload: Payload, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        if let Err(mpsc::SendError((_, done))) = self.queue.send((payload, Box::new(done))) {
            done(Err("not copied: the copier has stopped".to_owned()));
        }
    }
}

/// What a copy's own thread says once wl-clipboard-rs has prepared its source.
type Prepared = Result<(), String>;

/// Reads the payload, prepares a source with its types and the marker of copy `number` on a
/// thread of its own, and serves it only if the compositor answered within [`TAKE`]. A source not
/// served never reaches the compositor, because wl-clipboard-rs flushes `set_selection` only once
/// it serves. Then it waits for the capture thread to confirm the copy. A preparation that ran
/// over keeps its thread and its payload until the compositor answers, so it stays in `waiting`,
/// and until it ends every copy fails at once: a stalled compositor holds one payload and one
/// thread, however many copies come.
fn copy(
    number: u64,
    payload: Payload,
    marks: &Receiver<u64>,
    waiting: &mut Option<Receiver<Prepared>>,
) -> Result<(), String> {
    if let Some(ready) = waiting
        && let Err(TryRecvError::Empty) = ready.try_recv()
    {
        return Err("not copied: an earlier copy still waits for the compositor".to_owned());
    }
    *waiting = None;
    let (content, mime, body) = payload.read()?;
    let sources: Vec<MimeSource> = offers(content, mime, &body, number)
        .into_iter()
        .map(|(mime, bytes)| MimeSource {
            source: Bytes::Bytes(bytes.into_boxed_slice()),
            mime_type: MimeType::Specific(mime),
        })
        .collect();
    drop(body);
    let (prepared, ready) = mpsc::channel();
    let (go, serve) = mpsc::channel::<()>();
    thread::Builder::new()
        .spawn(move || {
            let mut options = Options::new();
            options.foreground(true);
            match options.prepare_copy_multi(sources) {
                Ok(source) => {
                    if prepared.send(Ok(())).is_ok() && serve.recv().is_ok() {
                        let _ = source.serve();
                    }
                }
                Err(error) => {
                    let _ = prepared.send(Err(error.to_string()));
                }
            }
        })
        .map_err(|error| format!("not copied: {error}"))?;
    match ready.recv_timeout(TAKE) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(format!("not copied: {error}")),
        Err(_) => {
            *waiting = Some(ready);
            return Err("not copied: the compositor did not take the selection".to_owned());
        }
    }
    go.send(())
        .map_err(|_| "not copied: the copy's thread ended".to_owned())?;
    confirm(number, marks)
}

/// Waits [`CONFIRM`] at most for the capture thread to report copy `number`. The numbers of
/// copies that gave up earlier go by.
fn confirm(number: u64, marks: &Receiver<u64>) -> Result<(), String> {
    let deadline = Instant::now() + CONFIRM;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match marks.recv_timeout(left) {
            Ok(seen) if seen == number => return Ok(()),
            Ok(_) => {}
            Err(_) => {
                return Err(
                    "not copied: the clipboard watcher did not confirm the selection".to_owned(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block cannot hold a copy's flush back until the mark of a copy that gave up comes, so
    /// this hands the marks to the wait itself. The failure is the one the doc's block shows.
    #[test]
    fn a_mark_of_another_copy_confirms_nothing() {
        let (marks, marked) = mpsc::channel();
        marks.send(1).unwrap();
        drop(marks);
        assert_eq!(
            confirm(2, &marked),
            Err("not copied: the clipboard watcher did not confirm the selection".to_owned())
        );
        let (marks, marked) = mpsc::channel();
        marks.send(1).unwrap();
        marks.send(2).unwrap();
        assert_eq!(confirm(2, &marked), Ok(()));
    }
}
