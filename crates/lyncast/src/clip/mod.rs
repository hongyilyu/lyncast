//! Clipboard history. [`Clips`] is the only owner of clip rows. It holds every row in memory with
//! a preview, and with a data home one writer thread owns `clips.sqlite3` and applies each change
//! in order, so no method here waits on the disk. docs/design/clipboard-and-paste.md describes it.

mod db;

use std::io;
use std::path::Path;

use lyncast_core::ClipId;
use lyncast_core::clip::{self, Place, Retention, TextKind};
use lyncast_core::rank::Timestamp;

use crate::warning::warn;

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

/// What a clip derived from its bytes when they arrived. The database keeps it beside the bytes,
/// so a load reads no text and derives nothing again.
struct Meta {
    content: Content,
    /// The type the bytes were read as.
    mime: String,
    bytes: u64,
    /// The first 1,024 bytes of the text or of the paths, cut at a character boundary. An image
    /// has none.
    preview: Option<String>,
}

/// One clip as the daemon holds it. What it holds lives in the database.
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
    pub used: Timestamp,
    pub pinned: bool,
    /// The row's number in the one sequence that orders the rows.
    pub seq: u64,
}

impl Row {
    /// A row that the load found.
    fn new(id: ClipId, meta: Meta, used: Timestamp, pinned: bool, seq: u64) -> Row {
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
        Place::of(self.pinned, self.seq)
    }
}

/// Every clip row, and where the changes go.
pub struct Clips {
    rows: Vec<Row>,
    /// The number the next pin takes.
    next_seq: u64,
    retention: Retention,
    /// Where each change goes. `None` without a data home, when the database did not load, and
    /// once the writer has ended after Quit.
    writer: Option<db::Writer>,
}

impl Clips {
    /// Starts the writer under an absolute data home, which loads the rows, and removes those
    /// that have expired at `now`. The writer calls `ended` when it ends. Without a data home
    /// there are none. A database that does not load in time warns on stderr, and the rows stay
    /// in memory, so the file is never written over. Fails only when the writer's thread cannot
    /// start.
    pub fn open(
        data_home: Option<&Path>,
        retention: Retention,
        now: Timestamp,
        ended: impl FnOnce() + Send + 'static,
    ) -> io::Result<Clips> {
        let mut clips = Clips {
            rows: Vec::new(),
            next_seq: 0,
            retention,
            writer: None,
        };
        let Some(data_home) = data_home else {
            return Ok(clips);
        };
        let path = db::path(data_home);
        let (writer, loaded) = match db::Writer::open(path.clone(), ended)? {
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
        clips.next_seq = loaded.next_seq;
        clips.writer = Some(writer);
        clips.prune(now);
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

    /// Removes the row.
    pub fn delete(&mut self, id: ClipId) {
        self.rows.retain(|row| row.id != id);
        self.send(db::Change::Delete { id });
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
    pub fn ended(&self) -> bool {
        self.writer.is_none()
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
        if let Some(writer) = &self.writer {
            writer.send(change);
        }
    }
}
