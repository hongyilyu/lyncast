//! Clipboard history. [`Clips`] is the only owner of clip rows. It holds every row in memory with
//! a preview, and with a data home one writer thread owns `clips.sqlite3` and `blobs/` and
//! applies each change in order, so no method here waits on the disk. The capture thread hands
//! it what the clipboard offers. docs/design/clipboard-and-paste.md describes it.

mod blob;
mod capture;
mod copy;
mod db;

pub use capture::{Capture, Seen};
pub use copy::{Copier, Payload};

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::process;
use std::sync::Arc;

use lyncast_core::ClipId;
use lyncast_core::clip::{self, Delivery, Handed, Place, Retention, TextKind};
use lyncast_core::rank::Timestamp;

use crate::warning::warn;
use blob::Hash;

/// The family of types that mark lyncast's own writes. Each copy offers [`marker`] with the
/// daemon's process ID and the copy's number.
const OWN: &str = "application/x-lyncast-own";

/// The type that marks this process's copy numbered `number`. It holds the same bytes as the
/// payload, so a reader that takes the first type still gets them.
fn marker(number: u64) -> String {
    format!("{OWN};copy={}.{number}", process::id())
}

/// The number of the copy of this process that `mime` marks. Another lyncast process numbers its
/// copies from 1 too, so its marks name its own process.
fn marked(mime: &str) -> Option<u64> {
    let (pid, number) = mime
        .strip_prefix(OWN)?
        .strip_prefix(";copy=")?
        .split_once('.')?;
    if pid.parse() == Ok(process::id()) {
        number.parse().ok()
    } else {
        None
    }
}

/// The type a copy offers a text as, and a recorded write of text is read as.
const TEXT: &str = "text/plain;charset=utf-8";

/// The two types of a list of files.
const URI_LIST: &str = "text/uri-list";
const GNOME_FILES: &str = "x-special/gnome-copied-files";

/// The most bytes a clip holds. A read that finds more drops the entry, and a blob that holds
/// more does not copy.
const MAX_BYTES: usize = 25 << 20;

/// What a row holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Text, which shows as a link or an email when the whole of it is one.
    Text(TextKind),
    Image,
    /// A list of local files, as `text/uri-list` spells them.
    Files,
}

impl Content {
    const ALL: [Content; 5] = [
        Content::Text(TextKind::Text),
        Content::Text(TextKind::Link),
        Content::Text(TextKind::Email),
        Content::Image,
        Content::Files,
    ];

    /// The kind as `clip list --json` and the database spell it.
    pub fn spelling(self) -> &'static str {
        match self {
            Content::Text(TextKind::Text) => "text",
            Content::Text(TextKind::Link) => "link",
            Content::Text(TextKind::Email) => "email",
            Content::Image => "image",
            Content::Files => "files",
        }
    }

    /// The kind that `spelling` spells.
    fn read(spelling: &str) -> Option<Content> {
        Content::ALL
            .into_iter()
            .find(|content| content.spelling() == spelling)
    }
}

/// What a clip holds: a text or a list of file URIs, or an image's bytes.
#[derive(Clone)]
enum Body {
    Text(Arc<str>),
    Image(Arc<[u8]>),
}

impl Body {
    fn bytes(&self) -> &[u8] {
        match self {
            Body::Text(text) => text.as_bytes(),
            Body::Image(bytes) => bytes,
        }
    }
}

/// What a clip derives from its bytes when they arrive. The database keeps it beside the bytes,
/// so a load reads no text and derives nothing again.
#[derive(Clone)]
struct Meta {
    content: Content,
    /// The type the bytes were read as.
    mime: String,
    bytes: u64,
    /// The first 1,024 bytes of the text or of the paths, cut at a character boundary. An image
    /// has none.
    preview: Option<String>,
    /// An image's blob is named by it.
    hash: Hash,
}

/// One representation of an offer, or a write the daemon made, read whole.
pub struct Entry {
    meta: Meta,
    body: Body,
}

impl Entry {
    /// `text`, read as `mime`. Whether it is a link or an email comes from the whole text.
    fn text(text: &str, mime: String) -> Entry {
        Entry {
            meta: Meta {
                content: Content::Text(clip::text_kind(text)),
                mime,
                bytes: text.len() as u64,
                preview: Some(clip::preview(text).to_owned()),
                hash: Hash::of(text.as_bytes()),
            },
            body: Body::Text(text.into()),
        }
    }

    /// The local files that `list`, read as `mime`, names, one URI per line. `None` when it
    /// names none.
    fn files(list: &str, mime: String) -> Option<Entry> {
        let uris: Vec<&str> = clip::local_files(list)
            .into_iter()
            .map(|file| file.uri)
            .collect();
        if uris.is_empty() {
            return None;
        }
        let body = uris.join("\n");
        Some(Entry {
            meta: Meta {
                content: Content::Files,
                mime,
                bytes: body.len() as u64,
                preview: Some(clip::preview(&clip::paths(&body)).to_owned()),
                hash: Hash::of(body.as_bytes()),
            },
            body: Body::Text(body.into()),
        })
    }

    /// An image's `bytes`, read as `mime`.
    fn image(bytes: Vec<u8>, mime: String) -> Entry {
        Entry {
            meta: Meta {
                content: Content::Image,
                mime,
                bytes: bytes.len() as u64,
                preview: None,
                hash: Hash::of(&bytes),
            },
            body: Body::Image(bytes.into()),
        }
    }
}

/// One clip as the daemon holds it. What it holds lives in the database and `blobs/`.
pub struct Row {
    pub id: ClipId,
    pub content: Content,
    /// The type the row was read as. A copy of an image offers it again.
    pub mime: String,
    /// The size of what the row holds.
    pub bytes: u64,
    /// The first 1,024 bytes of the text or of the paths, cut at a character boundary. An image
    /// has none.
    pub preview: Option<String>,
    hash: Hash,
    pub used: Timestamp,
    pub pinned: bool,
    /// The row's number in the one sequence that orders the rows.
    pub seq: u64,
}

impl Row {
    /// A row that the load found or that an entry adds.
    fn new(id: ClipId, meta: Meta, used: Timestamp, pinned: bool, seq: u64) -> Row {
        Row {
            id,
            content: meta.content,
            mime: meta.mime,
            bytes: meta.bytes,
            preview: meta.preview,
            hash: meta.hash,
            used,
            pinned,
            seq,
        }
    }

    fn place(&self) -> Place {
        Place::of(self.pinned, self.seq)
    }
}

/// Every clip row, and where the changes go.
pub struct Clips {
    rows: Vec<Row>,
    /// The id the next new row takes, one above every id used, so a reply can name a new row
    /// before the database has it. `None` once the ids have run out.
    next_id: Option<ClipId>,
    /// The number the next insert, promotion, or pin takes.
    next_seq: u64,
    retention: Retention,
    store: Store,
}

/// Where what the rows hold lives.
enum Store {
    /// In `clips.sqlite3` and `blobs/`, through the writer. `None` once the writer has ended
    /// after Quit.
    Disk(Option<db::Writer>),
    /// In memory, without a data home or when the database did not load.
    Memory(HashMap<ClipId, Body>),
}

impl Clips {
    /// Starts the writer under an absolute data home, which loads the rows, and removes those
    /// that have expired at `now`. The writer calls `unsaved` with each new row it could not save,
    /// which [`Clips::unsaved`] then takes out, and `ended` when it ends. Without a data home
    /// there are none. A database that does not load in time warns on stderr, and the rows stay
    /// in memory, so the file is never written over. Fails only when the writer's thread cannot
    /// start.
    pub fn open(
        data_home: Option<&Path>,
        retention: Retention,
        now: Timestamp,
        unsaved: impl Fn(ClipId) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Clips> {
        let mut clips = Clips {
            rows: Vec::new(),
            next_id: Some(ClipId(1)),
            next_seq: 0,
            retention,
            store: Store::Memory(HashMap::new()),
        };
        let Some(data_home) = data_home else {
            return Ok(clips);
        };
        let path = db::path(data_home);
        let (writer, loaded) = match db::Writer::open(path.clone(), unsaved, ended)? {
            Ok(opened) => opened,
            Err(error) => {
                warn(format_args!(
                    "{}: {error}; the clips stay in memory",
                    path.display()
                ));
                return Ok(clips);
            }
        };
        clips.rows = loaded.rows;
        clips.next_id = Some(loaded.next_id);
        clips.next_seq = loaded.next_seq;
        clips.store = Store::Disk(Some(writer));
        clips.prune(now);
        Ok(clips)
    }

    /// Records what the clipboard offered at `now`. A row of the same kind with the same bytes
    /// moves to the front, unless it is pinned, and keeps its id. Anything else is a new row
    /// with the next id, and the rows that have expired go.
    pub fn record(&mut self, entry: Entry, now: Timestamp) {
        let same = |row: &&Row| row.content == entry.meta.content && row.hash == entry.meta.hash;
        if let Some(id) = self.rows.iter().find(same).map(|row| row.id) {
            self.touch(id, now);
            return;
        }
        self.insert(entry, now);
        self.prune(now);
    }

    /// Records a write that a copy has seen the compositor take. A row of the history moves to the
    /// front unless it is pinned, and text is recorded as a capture of it would be.
    pub fn wrote(&mut self, delivery: &Delivery, now: Timestamp) {
        match delivery.handed() {
            Handed::Clip(id) => self.touch(id, now),
            Handed::Text(text) => self.record(Entry::text(text, TEXT.to_owned()), now),
        }
    }

    /// What a copy of `delivery` offers, and where its bytes are, or why it cannot copy.
    pub fn payload(&self, delivery: &Delivery) -> Result<Payload, String> {
        let id = match delivery.handed() {
            Handed::Clip(id) => id,
            Handed::Text(text) => return Ok(Payload::text(text)),
        };
        let row = self.get(id).ok_or_else(|| format!("no clip {}", id.0))?;
        let source = match &self.store {
            Store::Disk(Some(writer)) => copy::Source::Stored(writer.reader(), id),
            Store::Disk(None) => return Err("not copied: the writer of the clips has ended".into()),
            Store::Memory(bodies) => match bodies.get(&id) {
                Some(body) => copy::Source::Held(body.clone()),
                None => return Err(format!("no clip {}", id.0)),
            },
        };
        Ok(Payload::new(row.content, row.mime.clone(), source))
    }

    /// Every row in screen order.
    pub fn rows(&self) -> Vec<&Row> {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by_key(|row| row.place());
        rows
    }

    pub fn get(&self, id: ClipId) -> Option<&Row> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// Pins the row. A pin is a use, so the row takes the next number, and it keeps that
    /// number when it is unpinned.
    pub fn pin(&mut self, id: ClipId, now: Timestamp) {
        let seq = self.next_seq();
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.pinned = true;
            row.used = now;
            row.seq = seq;
            self.send(db::Change::Pin { id, used: now, seq });
        }
    }

    /// Unpins the row. It keeps its place, and its age starts again at `now`, so a row pinned
    /// for longer than the retention does not expire as soon as it is unpinned.
    pub fn unpin(&mut self, id: ClipId, now: Timestamp) {
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.pinned = false;
            row.used = now;
            self.send(db::Change::Unpin { id, used: now });
        }
    }

    /// Removes the row. The writer removes an image's blob once no saved row names it.
    pub fn delete(&mut self, id: ClipId) {
        let Some(at) = self.rows.iter().position(|row| row.id == id) else {
            return;
        };
        self.rows.remove(at);
        match &mut self.store {
            Store::Disk(writer) => {
                if let Some(writer) = writer {
                    writer.send(db::Change::Delete { id });
                }
            }
            Store::Memory(bodies) => {
                bodies.remove(&id);
            }
        }
    }

    /// The writer could not save the new row `id`, so the row goes, and a later capture of the
    /// same bytes adds it again. Its id stays used.
    pub fn unsaved(&mut self, id: ClipId) {
        self.rows.retain(|row| row.id != id);
    }

    /// Tells the writer to end once it has applied every change.
    pub fn close(&self) {
        if let Store::Disk(Some(writer)) = &self.store {
            writer.close();
        }
    }

    /// The writer has ended. It ends only after [`Clips::close`].
    pub fn writer_ended(&mut self) {
        if let Store::Disk(writer) = &mut self.store {
            *writer = None;
        }
    }

    /// No writer is left to wait for.
    pub fn ended(&self) -> bool {
        !matches!(self.store, Store::Disk(Some(_)))
    }

    /// Moves the row to the front, unless it is pinned, where a pin stays.
    fn touch(&mut self, id: ClipId, now: Timestamp) {
        if self.get(id).is_none_or(|row| row.pinned) {
            return;
        }
        let seq = self.next_seq();
        if let Some(row) = self.rows.iter_mut().find(|row| row.id == id) {
            row.used = now;
            row.seq = seq;
            self.send(db::Change::Promote { id, used: now, seq });
        }
    }

    /// Adds a row for `entry` with the next id. Without an id left, or when its bytes would bring
    /// what waits for the writer past [`db::MAX_PENDING`], the entry is dropped with a warning.
    fn insert(&mut self, entry: Entry, now: Timestamp) {
        let Some(id) = self.next_id else {
            warn(format_args!(
                "no clip id is left; the clip was not recorded"
            ));
            return;
        };
        if let Store::Disk(Some(writer)) = &self.store
            && !writer.has_room(entry.meta.bytes)
        {
            warn(format_args!(
                "{}: not saved: 64 MiB of clips wait for the disk; the clip was not recorded",
                writer.path().display()
            ));
            return;
        }
        let seq = self.next_seq;
        let Entry { meta, body } = entry;
        match &mut self.store {
            Store::Disk(Some(writer)) => writer.send(db::Change::Insert {
                id,
                meta: meta.clone(),
                body,
                at: now,
                seq,
            }),
            Store::Memory(bodies) => {
                bodies.insert(id, body);
            }
            // The writer ends only once nothing records again. A row here would hold no bytes.
            Store::Disk(None) => return,
        }
        self.next_id = id.0.checked_add(1).map(ClipId);
        self.next_seq += 1;
        self.rows.push(Row::new(id, meta, now, false, seq));
    }

    /// Removes every row that has expired at `now`.
    fn prune(&mut self, now: Timestamp) {
        let retention = self.retention;
        let expired: Vec<ClipId> = self
            .rows
            .iter()
            .filter(|row| clip::expired(row.used, row.pinned, now, retention))
            .map(|row| row.id)
            .collect();
        for id in expired {
            self.delete(id);
        }
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn send(&self, change: db::Change) {
        if let Store::Disk(Some(writer)) = &self.store {
            writer.send(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    const DAY: i64 = 86_400_000;

    /// Runs one `store` block of docs/design/clipboard-and-paste.md on a history in memory, with
    /// the default retention and a clock that starts at 0, and gives each line that fails.
    fn run(block: &doc::Block) -> Vec<String> {
        let mut clips = Clips::open(None, Retention::DEFAULT, Timestamp(0), |_| {}, || {}).unwrap();
        let mut now = Timestamp(0);
        let mut failures = Vec::new();
        for line in &block.lines {
            let held = |clips: &Clips, text: &str| {
                let row = clips
                    .rows()
                    .into_iter()
                    .find(|row| row.preview.as_deref() == Some(text));
                row.map(|row| row.id)
                    .unwrap_or_else(|| panic!("line {}: no row holds {text}", block.line))
            };
            match line.split_once(' ') {
                Some(("capture", text)) => clips.record(Entry::text(text, TEXT.to_owned()), now),
                Some(("pin", text)) => clips.pin(held(&clips, text), now),
                Some(("unpin", text)) => clips.unpin(held(&clips, text), now),
                Some(("wait", wait)) => {
                    let millis = match (wait.strip_suffix('d'), wait.strip_suffix("ms")) {
                        (Some(days), _) => days.parse::<i64>().map(|days| days * DAY),
                        (_, Some(millis)) => millis.parse(),
                        _ => panic!("line {}: not wait <n>d or <n>ms: {line}", block.line),
                    };
                    now.0 += millis.unwrap_or_else(|_| panic!("line {}: {line}", block.line));
                }
                Some(("=>", want)) => {
                    let rows = clips.rows();
                    let got: Vec<&str> = rows
                        .iter()
                        .filter_map(|row| row.preview.as_deref())
                        .collect();
                    if got.join(", ") != want {
                        failures.push(format!(
                            "line {}: {line}\n  got => {}",
                            block.line,
                            got.join(", ")
                        ));
                    }
                }
                _ => panic!("line {}: not a store line: {line}", block.line),
            }
        }
        failures
    }

    /// Every `store` block, which must number two, so a lost block fails.
    #[test]
    fn every_store_block_runs_as_the_doc_says() {
        let blocks = doc::CLIPBOARD.blocks("store");
        assert_eq!(
            blocks.len(),
            2,
            "clipboard-and-paste.md holds 2 store blocks"
        );
        let failures: Vec<String> = blocks.iter().flat_map(run).collect();
        assert!(
            failures.is_empty(),
            "{} store lines fail:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
