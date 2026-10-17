//! The rules of clipboard history that need no store: the kind a text shows as, a row's
//! preview, the paths of the files a list of URIs names, when a row expires, and the order of the
//! clipboard screen. The binary's `Clips` owns the rows and applies these rules to them.
//! docs/design/clipboard-and-paste.md holds the fixtures.

use std::cmp::Reverse;
use std::num::NonZeroU32;

use crate::rank::Timestamp;

/// What a text row shows as. A link or an email is text, derived from the whole text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Text,
    Link,
    Email,
}

/// A link when the whole trimmed text is one `http://` or `https://` URL with no whitespace,
/// an email when it is one address of the form `local@domain.tld`, and text otherwise.
pub fn text_kind(text: &str) -> TextKind {
    let text = text.trim();
    if text.contains(char::is_whitespace) {
        return TextKind::Text;
    }
    let url = ["http://", "https://"].iter().any(|scheme| {
        text.strip_prefix(scheme)
            .is_some_and(|rest| !rest.is_empty())
    });
    if url {
        return TextKind::Link;
    }
    let address = text.split_once('@').is_some_and(|(local, domain)| {
        let tld = domain.rsplit_once('.');
        !local.is_empty()
            && !domain.contains('@')
            && tld.is_some_and(|(name, tld)| !name.is_empty() && !tld.is_empty())
    });
    if address {
        TextKind::Email
    } else {
        TextKind::Text
    }
}

/// The most bytes a preview holds.
pub const PREVIEW_BYTES: usize = 1024;

/// The first [`PREVIEW_BYTES`] bytes of `text`, cut back to a character boundary.
pub fn preview(text: &str) -> &str {
    let mut end = text.len().min(PREVIEW_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The lines of a `text/uri-list` or `x-special/gnome-copied-files` list that name a local file,
/// in order, each without a trailing carriage return. A local file is a `file://` line whose host
/// is empty or `localhost`, and every other line goes: a comment, another scheme, another host,
/// and the `copy` or `cut` that starts a GNOME list.
pub fn local_uris(list: &str) -> impl Iterator<Item = &str> {
    lines(list).filter(|line| local_path(line).is_some())
}

/// The paths of the local files that a list names, one per line, as a file entry shows and pastes
/// them, with each `%XX` decoded. A decoded byte that is not UTF-8 reads as U+FFFD. The decoding
/// stops once the paths hold `max` bytes and 3 more, inside a line too, which ends every character
/// that starts before `max`, so a preview of `max` bytes decodes only what it shows.
pub fn paths(list: &str, max: usize) -> String {
    let limit = max.saturating_add(3);
    let mut paths = Vec::new();
    for path in lines(list).filter_map(local_path) {
        if paths.len() >= limit {
            break;
        }
        if !paths.is_empty() {
            paths.push(b'\n');
        }
        percent_decode(path, &mut paths, limit);
    }
    String::from_utf8_lossy(&paths).into_owned()
}

fn lines(list: &str) -> impl Iterator<Item = &str> {
    list.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

fn local_path(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    path.starts_with('/').then_some(path)
}

fn percent_decode(text: &str, out: &mut Vec<u8>, limit: usize) {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() && out.len() < limit {
        // from_str_radix also reads a sign, as in `%+1`, so the digits are checked first.
        let hex = bytes
            .get(at + 1..at + 3)
            .filter(|pair| pair.iter().all(u8::is_ascii_hexdigit))
            .and_then(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok());
        match (bytes[at], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                at += 3;
            }
            (byte, _) => {
                out.push(byte);
                at += 1;
            }
        }
    }
}

/// How long an unpinned row lives after its last use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retention {
    Forever,
    Days(NonZeroU32),
}

impl Retention {
    /// The default, about 3 months.
    pub const DEFAULT: Retention = Retention::Days(NonZeroU32::new(90).unwrap());

    /// `retention-days` as `config.toml` spells it, where 0 keeps rows forever.
    pub fn days(days: u32) -> Retention {
        NonZeroU32::new(days).map_or(Retention::Forever, Retention::Days)
    }
}

const DAY_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// Whether a row last used at `used` has expired at `now`. A pin never expires, and an unpinned
/// row expires once more than the retention's days have passed since its last use.
pub fn expired(used: Timestamp, pinned: bool, now: Timestamp, retention: Retention) -> bool {
    match retention {
        Retention::Days(days) if !pinned => {
            now.0.saturating_sub(used.0) > i64::from(days.get()) * DAY_MILLIS
        }
        Retention::Days(_) | Retention::Forever => false,
    }
}

/// Where a row sorts on the clipboard screen. Pins come first, in the order they were pinned,
/// and then the other rows, newest use first. Each insert, promotion, and pin takes the next
/// number of one sequence, so two in the same millisecond keep their order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place(Order);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Order {
    Pinned(u64),
    Unpinned(Reverse<u64>),
}

impl Place {
    /// The place of a row with sequence number `seq`.
    pub fn of(pinned: bool, seq: u64) -> Place {
        Place(if pinned {
            Order::Pinned(seq)
        } else {
            Order::Unpinned(Reverse(seq))
        })
    }
}
