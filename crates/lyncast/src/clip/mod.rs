mod db;
mod load;
mod schema;

use std::collections::HashSet;
use std::io;
use std::path::Path;

use lyncast_core::ClipId;
use lyncast_core::clip::{self, Place, Retention, TextKind};
use lyncast_core::rank::Timestamp;

use crate::warning::warn;

pub fn no_clip(id: ClipId) -> String {
    format!("no clip {}", id.0)
}

const WRITER_ENDED: &str = "the writer of the clips has ended";

/// Why no row takes another sequence number, after the last one taken when it is known.
fn no_seq(after: Option<Seq>) -> String {
    let after = after.map(|seq| format!(" after {}", seq.0));
    format!("no sequence number is left{}", after.unwrap_or_default())
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

struct Meta {
    content: Content,
    mime: String,
    bytes: u64,
    preview: Option<String>,
}

pub struct Row {
    pub id: ClipId,
    pub content: Content,
    pub mime: String,
    pub bytes: u64,
    /// The first 1,024 bytes of the text or of the paths, cut at a character boundary. An image
    /// has none.
    pub preview: Option<String>,
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
    next_seq: Option<Seq>,
    retention: Retention,
    /// Where each change goes. `None` without a data home, when the database did not load, and
    /// once the writer has ended after Quit.
    writer: Option<db::Writer>,
}

impl Clips {
    /// Starts the writer under an absolute data home, which loads the rows. The writer calls
    /// `ended` when it ends after [`Clips::close`]. Without a data home there are none. A database
    /// that does not load, for any reason, warns on stderr, and the rows stay in memory, so the
    /// file is never written over. Fails only when the writer's thread cannot start.
    pub fn open(
        data_home: Option<&Path>,
        retention: Retention,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Clips> {
        let mut clips = Clips {
            rows: Vec::new(),
            next_seq: Some(Seq::FIRST),
            retention,
            writer: None,
        };
        let Some(data_home) = data_home else {
            return Ok(clips);
        };
        let path = load::path(data_home);
        let (writer, loaded) = match db::Writer::open(path.clone(), ended)? {
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
        clips.next_seq = Some(loaded.next_seq);
        clips.writer = Some(writer);
        Ok(clips)
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
    /// number when it is unpinned. Fails, changing nothing, without such a row or once the
    /// numbers have run out.
    pub fn pin(&mut self, id: ClipId, now: Timestamp) -> Result<(), String> {
        let at = self.at(id)?;
        let seq = self.take_seq().ok_or_else(|| no_seq(None))?;
        let row = &mut self.rows[at];
        row.pinned = true;
        row.used = now;
        row.seq = seq;
        self.send(db::Change::Pin { id, used: now, seq });
        Ok(())
    }

    /// Unpins the row. It keeps its place, and its age starts again at `now`, so a row pinned
    /// for longer than the retention does not expire as soon as it is unpinned. Fails without
    /// such a row.
    pub fn unpin(&mut self, id: ClipId, now: Timestamp) -> Result<(), String> {
        let at = self.at(id)?;
        let row = &mut self.rows[at];
        row.pinned = false;
        row.used = now;
        self.send(db::Change::Unpin { id, used: now });
        Ok(())
    }

    /// Removes the row. Fails without such a row.
    pub fn delete(&mut self, id: ClipId) -> Result<(), String> {
        self.at(id)?;
        self.rows.retain(|row| row.id != id);
        self.send(db::Change::Delete { ids: vec![id] });
        Ok(())
    }

    fn at(&self, id: ClipId) -> Result<usize, String> {
        self.rows
            .iter()
            .position(|row| row.id == id)
            .ok_or_else(|| no_clip(id))
    }

    /// Tells the writer to end once it has applied every change.
    pub fn close(&self) {
        if let Some(writer) = &self.writer {
            writer.close();
        }
    }

    /// The writer has ended. It ends only after [`Clips::close`].
    pub fn writer_ended(&mut self) {
        self.writer = None;
    }

    /// No writer is left to wait for.
    pub fn has_no_writer(&self) -> bool {
        self.writer.is_none()
    }

    /// Removes the rows that have expired at `now`.
    pub fn prune(&mut self, now: Timestamp) {
        let retention = self.retention;
        let expired: HashSet<ClipId> = self
            .rows
            .iter()
            .filter(|row| clip::expired(row.used, row.pinned, now, retention))
            .map(|row| row.id)
            .collect();
        if !expired.is_empty() {
            self.rows.retain(|row| !expired.contains(&row.id));
            let ids = expired.into_iter().collect();
            self.send(db::Change::Delete { ids });
        }
    }

    fn take_seq(&mut self) -> Option<Seq> {
        let seq = self.next_seq?;
        self.next_seq = seq.next();
        Some(seq)
    }

    fn send(&self, change: db::Change) {
        if let Some(writer) = &self.writer {
            writer.send(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_to_an_id_with_no_row_fails() {
        let mut clips = Clips::open(None, Retention::DEFAULT, || {}).unwrap();
        let id = ClipId(1);
        assert_eq!(clips.pin(id, Timestamp(0)), Err(no_clip(id)));
        assert_eq!(clips.unpin(id, Timestamp(0)), Err(no_clip(id)));
        assert_eq!(clips.delete(id), Err(no_clip(id)));
    }
}
