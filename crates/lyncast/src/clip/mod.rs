mod blob;
mod capture;
mod db;
mod load;
mod schema;

pub use capture::{Capture, Seen};

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use lyncast_core::ClipId;
use lyncast_core::clip::{self, PREVIEW_BYTES, Place, Retention, TextKind};
use lyncast_core::rank::Timestamp;

use crate::files::Unsaved;
use crate::permit::Permit;
use crate::warning::warn;
use blob::Hash;

const OWN: &str = "application/x-lyncast-own";

const URI_LIST: &str = "text/uri-list";
const GNOME_FILES: &str = "x-special/gnome-copied-files";

const MAX_BYTES: usize = 25 << 20;

/// How long the reads of one offer may take in all, so a source that never answers holds the
/// capture thread no longer.
const OFFER_READ_DEADLINE: Duration = Duration::from_secs(2);

pub fn no_clip(id: ClipId) -> String {
    format!("no clip {}", id.0)
}

const WRITER_ENDED: &str = "the writer of the clips has ended";

/// Why no new row takes another id, after the last one used when it is known.
fn no_id(after: Option<ClipId>) -> String {
    let after = after.map(|id| format!(" after {}", id.0));
    format!("no clip id is left{}", after.unwrap_or_default())
}

/// Why no row takes another sequence number, after the last one taken when it is known.
fn no_seq(after: Option<Seq>) -> String {
    let after = after.map(|seq| format!(" after {}", seq.0));
    format!("no sequence number is left{}", after.unwrap_or_default())
}

/// Warns that a capture or a copy's write is not recorded, and why.
fn not_recorded(why: impl fmt::Display) {
    warn(format_args!("{why}; the clip was not recorded"));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
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

    fn read(spelling: &str) -> Option<Content> {
        Content::ALL
            .into_iter()
            .find(|content| content.spelling() == spelling)
    }
}

enum Body {
    Text(Arc<str>),
    Image(Arc<[u8]>),
}

#[derive(Clone)]
struct Meta {
    content: Content,
    mime: String,
    bytes: u64,
    preview: Option<String>,
    hash: Hash,
}

pub struct Entry {
    meta: Meta,
    body: Body,
    share: Option<Permit>,
}

impl Entry {
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
            share: None,
        }
    }

    fn files(list: &str, mime: String) -> Option<Entry> {
        let mut body = String::new();
        for uri in clip::local_uris(list) {
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(uri);
        }
        if body.is_empty() {
            return None;
        }
        let paths = clip::paths(&body, PREVIEW_BYTES);
        Some(Entry {
            meta: Meta {
                content: Content::Files,
                mime,
                bytes: body.len() as u64,
                preview: Some(clip::preview(&paths).to_owned()),
                hash: Hash::of(body.as_bytes()),
            },
            body: Body::Text(body.into()),
            share: None,
        })
    }

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
            share: None,
        }
    }

    /// The entry with its share of what waits for the writer, or `None`, with a warning, when
    /// it would bring that past [`db::MAX_PENDING`]. Without a writer it needs none.
    fn admitted(mut self, pending: Option<&db::Pending>) -> Option<Entry> {
        if let Some(pending) = pending {
            self.share = Some(pending.take(self.meta.bytes)?);
        }
        Some(self)
    }
}

pub struct Row {
    pub id: ClipId,
    pub content: Content,
    pub mime: String,
    pub bytes: u64,
    /// The first 1,024 bytes of the text or of the paths, cut at a character boundary. An image
    /// has none.
    pub preview: Option<String>,
    hash: Hash,
    used: Timestamp,
    pub pinned: bool,
    seq: Seq,
}

impl Row {
    fn new(id: ClipId, meta: Meta, used: Timestamp, pinned: bool, seq: Seq) -> Row {
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
        Place::of(self.pinned, self.seq.get())
    }
}

/// A number in the one sequence that orders the rows. It starts at 0, and only [`Seq::next`]
/// advances it, so it never passes `i64::MAX`, the largest number SQLite stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Seq(i64);

impl Seq {
    const FIRST: Seq = Seq(0);

    /// The number after this one, or `None` past `i64::MAX`.
    fn next(self) -> Option<Seq> {
        self.0.checked_add(1).map(Seq)
    }

    /// The number as [`lyncast_core::clip::Place`] sorts it.
    fn get(self) -> u64 {
        self.0.cast_unsigned()
    }
}

pub struct Clips {
    rows: Vec<Row>,
    next_id: Option<ClipId>,
    next_seq: Option<Seq>,
    retention: Retention,
    bodies: Bodies,
}

/// Where the rows' text and image bytes live, and so where each change to a row goes.
enum Bodies {
    /// In the file, which the writer owns, and to which each change goes in order.
    Disk(db::Writer),
    Memory(HashMap<ClipId, Body>),
    /// The writer has ended, after Quit. Each change is refused then, as when the writer has no
    /// room, though none comes: the daemon closes the writer only once the capture thread has
    /// ended and no job is out, and it fails every request from Quit on.
    Ended,
}

impl Bodies {
    /// Whether `changes` more changes have room, or why not.
    fn room(&self, changes: u64) -> Result<(), String> {
        match self {
            Bodies::Disk(writer) => writer.room(changes),
            Bodies::Memory(_) => Ok(()),
            Bodies::Ended => Err(WRITER_ENDED.to_owned()),
        }
    }

    /// Whether `changes` more changes have room, with a warning that the clip was not recorded
    /// when they do not.
    fn admit(&self, changes: u64) -> bool {
        self.room(changes).map_err(|why| self.refuse(why)).is_ok()
    }

    /// Warns that a clip was not recorded, and why, with the file's path while the writer runs.
    fn refuse(&self, why: String) {
        match self {
            Bodies::Disk(writer) => not_recorded(Unsaved::new(writer.path(), why)),
            Bodies::Memory(_) | Bodies::Ended => not_recorded(why),
        }
    }

    /// Keeps the body of a new row and gives back its meta, or gives `None`, with a warning,
    /// when the `changes` it makes have no room or its bytes find no share of what waits for the
    /// writer. The entry brings the share that the capture thread took for it, if any.
    fn insert(
        &mut self,
        changes: u64,
        id: ClipId,
        entry: Entry,
        at: Timestamp,
        seq: Seq,
    ) -> Option<Meta> {
        if !self.admit(changes) {
            return None;
        }
        let Entry { meta, body, share } = entry;
        match self {
            Bodies::Disk(writer) => {
                let share = share.or_else(|| writer.pending().take(meta.bytes))?;
                writer.send(db::Change::Insert {
                    id,
                    meta: meta.clone(),
                    body,
                    at,
                    seq,
                    share,
                });
            }
            Bodies::Memory(bodies) => {
                bodies.insert(id, body);
            }
            Bodies::Ended => return None,
        }
        Some(meta)
    }

    /// Hands a change to a row to the writer. In memory the row holds the change already.
    fn send(&self, change: db::Change) {
        if let Bodies::Disk(writer) = self {
            writer.send(change);
        }
    }

    fn forget(&mut self, ids: Vec<ClipId>) {
        match self {
            Bodies::Disk(writer) => writer.send(db::Change::Delete { ids }),
            Bodies::Memory(bodies) => {
                for id in ids {
                    bodies.remove(&id);
                }
            }
            Bodies::Ended => {}
        }
    }

    fn pending(&self) -> Option<db::Pending> {
        match self {
            Bodies::Disk(writer) => Some(writer.pending().clone()),
            Bodies::Memory(_) | Bodies::Ended => None,
        }
    }

    fn close(&self) {
        if let Bodies::Disk(writer) = self {
            writer.close();
        }
    }

    fn has_writer(&self) -> bool {
        matches!(self, Bodies::Disk(_))
    }
}

impl Clips {
    /// Starts the writer under an absolute data home, which loads the rows. The writer calls
    /// `unsaved` with each new row it could not save, which [`Clips::unsaved`] then takes out, and
    /// `ended` when it ends after [`Clips::close`]. Without a data home there are none. A database
    /// that does not load, for any reason, warns on stderr, and the rows stay in memory, so the
    /// file is never written over. Fails only when the writer's thread cannot start.
    pub fn open(
        data_home: Option<&Path>,
        retention: Retention,
        unsaved: impl Fn(ClipId) + Send + 'static,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Clips> {
        let mut clips = Clips {
            rows: Vec::new(),
            next_id: Some(ClipId(1)),
            next_seq: Some(Seq::FIRST),
            retention,
            bodies: Bodies::Memory(HashMap::new()),
        };
        let Some(data_home) = data_home else {
            return Ok(clips);
        };
        let path = load::path(data_home);
        let (writer, loaded) = match db::Writer::open(path.clone(), unsaved, ended)? {
            db::Opened::Loaded(writer, loaded) => (writer, loaded),
            db::Opened::InMemory(why) => {
                warn(format_args!(
                    "{}: {why}; the clips stay in memory",
                    path.display()
                ));
                return Ok(clips);
            }
        };
        clips.rows = loaded.rows;
        clips.next_id = Some(loaded.next_id);
        clips.next_seq = Some(loaded.next_seq);
        clips.bodies = Bodies::Disk(writer);
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
    }

    /// What waits for the writer, which a capture takes its share of before the owner loop has
    /// the entry. `None` without a writer, since the history in memory holds every row's bytes.
    pub fn pending(&self) -> Option<db::Pending> {
        self.bodies.pending()
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
    /// number when it is unpinned. Fails, changing nothing, without such a row, while the writer
    /// is too far behind, or once the numbers have run out.
    pub fn pin(&mut self, id: ClipId, now: Timestamp) -> Result<(), String> {
        let at = self.at(id)?;
        self.room()?;
        let seq = self.take_seq().ok_or_else(|| no_seq(None))?;
        let row = &mut self.rows[at];
        row.pinned = true;
        row.used = now;
        row.seq = seq;
        self.bodies.send(db::Change::Pin { id, used: now, seq });
        Ok(())
    }

    /// Unpins the row. It keeps its place, and its age starts again at `now`, so a row pinned
    /// for longer than the retention does not expire as soon as it is unpinned. Fails, changing
    /// nothing, without such a row or while the writer is too far behind.
    pub fn unpin(&mut self, id: ClipId, now: Timestamp) -> Result<(), String> {
        let at = self.at(id)?;
        self.room()?;
        let row = &mut self.rows[at];
        row.pinned = false;
        row.used = now;
        self.bodies.send(db::Change::Unpin { id, used: now });
        Ok(())
    }

    /// Removes the row. The writer removes an image's blob once no saved row names it. Fails,
    /// changing nothing, without such a row or while the writer is too far behind.
    pub fn delete(&mut self, id: ClipId) -> Result<(), String> {
        self.at(id)?;
        self.room()?;
        self.remove(vec![id]);
        Ok(())
    }

    fn at(&self, id: ClipId) -> Result<usize, String> {
        self.rows
            .iter()
            .position(|row| row.id == id)
            .ok_or_else(|| no_clip(id))
    }

    fn room(&self) -> Result<(), String> {
        self.bodies
            .room(1)
            .map_err(|full| Unsaved::refused(full).to_string())
    }

    fn remove(&mut self, ids: Vec<ClipId>) {
        let gone: HashSet<ClipId> = ids.iter().copied().collect();
        self.rows.retain(|row| !gone.contains(&row.id));
        self.bodies.forget(ids);
    }

    /// The writer could not save the new row `id`, so the row goes, and a later capture of the
    /// same bytes adds it again. Its id stays used.
    pub fn unsaved(&mut self, id: ClipId) {
        self.rows.retain(|row| row.id != id);
    }

    /// Tells the writer to end once it has applied every change.
    pub fn close(&self) {
        self.bodies.close();
    }

    /// The writer has ended. It ends only after [`Clips::close`].
    pub fn writer_ended(&mut self) {
        self.bodies = Bodies::Ended;
    }

    /// No writer is left to wait for.
    pub fn has_no_writer(&self) -> bool {
        !self.bodies.has_writer()
    }

    fn touch(&mut self, id: ClipId, now: Timestamp) {
        let Ok(at) = self.at(id) else { return };
        if self.rows[at].pinned || !self.bodies.admit(1) {
            return;
        }
        let Some(seq) = self.take_seq() else {
            return not_recorded(no_seq(None));
        };
        let row = &mut self.rows[at];
        row.used = now;
        row.seq = seq;
        self.bodies.send(db::Change::Promote { id, used: now, seq });
    }

    fn insert(&mut self, entry: Entry, now: Timestamp) {
        let Some(id) = self.next_id else {
            return not_recorded(no_id(None));
        };
        let Some(seq) = self.next_seq else {
            return not_recorded(no_seq(None));
        };
        let expired = self.expired(now);
        let changes = 1 + u64::from(!expired.is_empty());
        let Some(meta) = self.bodies.insert(changes, id, entry, now, seq) else {
            return;
        };
        self.next_id = id.0.checked_add(1).map(ClipId);
        self.next_seq = seq.next();
        self.rows.push(Row::new(id, meta, now, false, seq));
        if !expired.is_empty() {
            self.remove(expired);
        }
    }

    fn expired(&self, now: Timestamp) -> Vec<ClipId> {
        self.rows
            .iter()
            .filter(|row| clip::expired(row.used, row.pinned, now, self.retention))
            .map(|row| row.id)
            .collect()
    }

    /// Removes the rows that have expired at `now`.
    pub fn prune(&mut self, now: Timestamp) {
        let expired = self.expired(now);
        if !expired.is_empty() && self.bodies.room(1).is_ok() {
            self.remove(expired);
        }
    }

    fn take_seq(&mut self) -> Option<Seq> {
        let seq = self.next_seq?;
        self.next_seq = seq.next();
        Some(seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    const DAY: i64 = 86_400_000;

    fn run(block: &doc::Block) -> Vec<String> {
        let mut clips = Clips::open(None, Retention::DEFAULT, |_| {}, || {}).unwrap();
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
                Some(("capture", text)) => clips.record(
                    Entry::text(text, "text/plain;charset=utf-8".to_owned()),
                    now,
                ),
                Some(("pin", text)) => clips.pin(held(&clips, text), now).unwrap(),
                Some(("unpin", text)) => clips.unpin(held(&clips, text), now).unwrap(),
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

    #[test]
    fn a_change_to_an_id_with_no_row_fails() {
        let mut clips = Clips::open(None, Retention::DEFAULT, |_| {}, || {}).unwrap();
        let id = ClipId(1);
        assert_eq!(clips.pin(id, Timestamp(0)), Err(no_clip(id)));
        assert_eq!(clips.unpin(id, Timestamp(0)), Err(no_clip(id)));
        assert_eq!(clips.delete(id), Err(no_clip(id)));
    }

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
