use std::io;
use std::process;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::clip;
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

use super::db::Reader;
use super::{Body, Content, GNOME_FILES, OFFER_READ_DEADLINE, OWN, TEXT, URI_LIST, not_copied};
use crate::permit::Permits;

/// How long a copy waits for the compositor to take its selection, which wl-clipboard-rs
/// prepares with a roundtrip to the compositor.
const TAKE: Duration = Duration::from_millis(500);

const CONFIRM: Duration = OFFER_READ_DEADLINE.saturating_add(Duration::from_millis(500));

/// The most copies that serve their pastes at once. wl-clipboard-rs writes a paste in one blocking
/// copy, so a serving thread sees that another selection replaced it only once the reader reads or
/// closes its pipe.
const MAX_SERVING_COPIES: usize = 4;

/// The most copies not finished, the one in hand among them, so copies that wait behind a stalled
/// writer hold that many payloads at most.
const MAX_UNFINISHED_COPIES: u64 = 4;

pub struct Payload {
    content: Content,
    mime: String,
    bytes: BytesAt,
}

pub(super) enum BytesAt {
    Held(Body),
    Stored(Reader, ClipId),
}

impl Payload {
    pub(super) fn new(content: Content, mime: String, bytes: BytesAt) -> Payload {
        Payload {
            content,
            mime,
            bytes,
        }
    }

    pub(super) fn text(text: &str) -> Payload {
        Payload {
            content: Content::Text(clip::text_kind(text)),
            mime: TEXT.to_owned(),
            bytes: BytesAt::Held(Body::Text(text.into())),
        }
    }

    fn read(self) -> Result<(Content, String, Body), String> {
        let body = match self.bytes {
            BytesAt::Held(body) => body,
            BytesAt::Stored(reader, id) => reader.read(id)?,
        };
        Ok((self.content, self.mime, body))
    }

    /// Reads the bytes as a copy does, and gives what `look` makes of them. A stored row's read
    /// waits for the writer, so the palette calls this off its main thread.
    pub(crate) fn blocking_read_with<T>(self, look: impl FnOnce(&[u8]) -> T) -> Result<T, String> {
        let (_, _, body) = self.read()?;
        Ok(look(body.bytes()))
    }
}

/// A text offers `text/plain;charset=utf-8`, which wl-clipboard-rs also offers as its other text
/// types. wl-clipboard-rs offers its types in no set order, but it fills its other text types from
/// the first text source, so the paths go first, and a paste into a text field gets them. The
/// marker holds the first offer's bytes, so a reader that takes the first type still gets them.
fn offers(content: Content, mime: String, body: &Body, marker: String) -> Vec<(String, Vec<u8>)> {
    let bytes = body.bytes();
    let mut offers = match (content, body) {
        (_, Body::Image(_)) => vec![(mime, bytes.to_vec())],
        (Content::Files, Body::Text(list)) => vec![
            (TEXT.to_owned(), clip::paths(list, usize::MAX).into_bytes()),
            (URI_LIST.to_owned(), bytes.to_vec()),
            (
                GNOME_FILES.to_owned(),
                [b"copy\n".as_slice(), bytes].concat(),
            ),
        ],
        (_, Body::Text(_)) => vec![(TEXT.to_owned(), bytes.to_vec())],
    };
    let marked = offers[0].1.clone();
    offers.push((marker, marked));
    offers
}

/// The type that marks copy `number` of this process, with a token of 64 random bits that no
/// other client knows before the copy offers it.
fn marker(number: u64) -> io::Result<String> {
    let mut token = [0; 8];
    // SAFETY: getrandom writes at most `token.len()` bytes to `token`, which lives through the
    // call. A read of 256 bytes or fewer fills the buffer or fails (getrandom(2)).
    if unsafe { libc::getrandom(token.as_mut_ptr().cast(), token.len(), 0) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let (pid, token) = (process::id(), u64::from_ne_bytes(token));
    Ok(format!("{OWN};copy={pid}.{number}.{token:016x}"))
}

/// The marker of the copy that waits for the capture thread to see its marked offer, if a copy
/// waits. Copies run one at a time, so one marker is all a copy needs, and a marker that no copy
/// waits for is dropped.
#[derive(Default)]
pub struct Marks {
    awaited: Mutex<Option<String>>,
    seen: Condvar,
}

impl Marks {
    /// Confirms the copy that waits for `marker`, if one does.
    pub fn see(&self, marker: &str) {
        let mut awaited = self.awaited();
        if awaited.as_deref() == Some(marker) {
            *awaited = None;
            self.seen.notify_one();
        }
    }

    fn expect(&self, marker: String) {
        *self.awaited() = Some(marker);
    }

    fn seen(&self, wait: Duration) -> bool {
        let (mut awaited, _) = self
            .seen
            .wait_timeout_while(self.awaited(), wait, |awaited| awaited.is_some())
            .unwrap_or_else(PoisonError::into_inner);
        awaited.take().is_none()
    }

    fn awaited(&self) -> MutexGuard<'_, Option<String>> {
        self.awaited.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

type Queued = (Payload, Box<dyn FnOnce(Result<(), String>) + Send>);

pub struct Copier {
    queue: Sender<Queued>,
    unfinished: Permits,
}

impl Copier {
    /// Starts the thread, which numbers its copies from 1 and waits for each one's mark in
    /// `marks`, which the capture thread hands the marker of each marked offer.
    pub fn start(marks: Arc<Marks>) -> io::Result<Copier> {
        let (queue, copies) = mpsc::channel::<Queued>();
        thread::Builder::new().spawn(move || {
            let mut threads = Threads::default();
            for (number, (payload, done)) in (1..).zip(copies) {
                done(copy(number, payload, &marks, &mut threads));
            }
        })?;
        Ok(Copier {
            queue,
            unfinished: Permits::new(MAX_UNFINISHED_COPIES),
        })
    }

    /// Queues a copy of `payload`, or fails it at once while [`MAX_UNFINISHED_COPIES`] earlier
    /// copies have not finished. `done` gets its result once the capture thread has seen the
    /// compositor take the selection, or once the copy has failed.
    pub fn copy(&self, payload: Payload, done: impl FnOnce(Result<(), String>) + Send + 'static) {
        let Some(place) = self.unfinished.take(1) else {
            return done(Err(not_copied(format_args!(
                "{MAX_UNFINISHED_COPIES} earlier copies have not finished"
            ))));
        };
        let done = move |copied| {
            drop(place);
            done(copied);
        };
        if let Err(mpsc::SendError((_, done))) = self.queue.send((payload, Box::new(done))) {
            done(Err(not_copied("the copier has stopped")));
        }
    }
}

type Prepared = Result<(), String>;

#[derive(Default)]
struct Threads {
    waiting: Option<Receiver<Prepared>>,
    serving: Vec<Receiver<()>>,
}

impl Threads {
    /// Lets one more copy start, or says why not, so a stalled compositor holds one payload, and
    /// paste readers that do not read hold [`MAX_SERVING_COPIES`] at most. It forgets each thread
    /// that has ended.
    fn admit(&mut self) -> Result<(), String> {
        if let Some(ready) = &self.waiting
            && let Err(TryRecvError::Empty) = ready.try_recv()
        {
            return Err(not_copied("an earlier copy still waits for the compositor"));
        }
        self.waiting = None;
        self.serving
            .retain(|serving| serving.try_recv() == Err(TryRecvError::Empty));
        if self.serving.len() >= MAX_SERVING_COPIES {
            return Err(not_copied(format_args!(
                "{MAX_SERVING_COPIES} earlier copies still serve their pastes"
            )));
        }
        Ok(())
    }
}

/// A source not served never reaches the compositor, because wl-clipboard-rs flushes
/// `set_selection` only once it serves.
fn copy(number: u64, payload: Payload, marks: &Marks, threads: &mut Threads) -> Result<(), String> {
    threads.admit()?;
    let (content, mime, body) = payload.read()?;
    let marker = marker(number).map_err(not_copied)?;
    let sources: Vec<MimeSource> = offers(content, mime, &body, marker.clone())
        .into_iter()
        .map(|(mime, bytes)| MimeSource {
            source: Source::Bytes(bytes.into_boxed_slice()),
            mime_type: MimeType::Specific(mime),
        })
        .collect();
    drop(body);
    let (prepared, ready) = mpsc::channel();
    let (go, serve) = mpsc::channel::<()>();
    let (alive, serving) = mpsc::channel::<()>();
    thread::Builder::new()
        .spawn(move || {
            let _alive = alive;
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
        .map_err(not_copied)?;
    match ready.recv_timeout(TAKE) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(not_copied(error)),
        Err(_) => {
            threads.waiting = Some(ready);
            return Err(not_copied("the compositor did not take the selection"));
        }
    }
    marks.expect(marker);
    go.send(())
        .map_err(|_| not_copied("the copy's thread ended"))?;
    threads.serving.push(serving);
    if !marks.seen(CONFIRM) {
        return Err(not_copied(
            "the clipboard watcher did not confirm the selection",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn a_mark_confirms_only_the_copy_that_waits_for_it() {
        let mark = |number| marker(number).unwrap();
        let marks = Marks::default();
        let one = mark(1);
        marks.see(&one);
        marks.expect(one.clone());
        assert!(
            !marks.seen(Duration::ZERO),
            "a mark seen before copy 1 waited confirmed it"
        );
        marks.expect(mark(2));
        marks.see(&one);
        assert!(
            !marks.seen(Duration::ZERO),
            "the mark of copy 1 confirmed copy 2"
        );
        let three = mark(3);
        marks.expect(three.clone());
        marks.see(&mark(3));
        assert!(
            !marks.seen(Duration::ZERO),
            "a mark of copy 3 with another token confirmed it"
        );
        marks.expect(three.clone());
        marks.see(&three);
        assert!(marks.seen(Duration::ZERO), "copy 3's own mark");
    }

    #[test]
    fn a_mark_wakes_the_copy_that_waits_for_it() {
        let marks = Arc::new(Marks::default());
        let five = marker(5).unwrap();
        marks.expect(five.clone());
        let capture = Arc::clone(&marks);
        let marked = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            for stale in 1..=5 {
                capture.see(&marker(stale).unwrap());
            }
            capture.see(&five);
            Instant::now()
        });
        assert!(
            marks.seen(Duration::from_secs(10)),
            "copy 5 missed its mark"
        );
        let woke = Instant::now();
        let marked = marked.join().unwrap();
        let late = woke.saturating_duration_since(marked);
        assert!(
            late < Duration::from_secs(2),
            "copy 5 woke {late:?} after its mark"
        );
    }
}
