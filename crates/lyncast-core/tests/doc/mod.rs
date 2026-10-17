//! Reads the fixture blocks of docs/design/ranking.md, the only copy of ranking's fixtures.
//! `tests/ranking.rs`, `tests/desktop.rs`, and the binary's `tests/query.rs` share it.

#![allow(dead_code)]

use std::fs;
use std::path::Path;

/// Every fixture kind ranking.md may hold.
pub const KINDS: [&str; 14] = [
    "fold",
    "score",
    "rank",
    "frecency",
    "terms",
    "entry",
    "parsed",
    "exec",
    "env",
    "desktop",
    "executable",
    "symlink",
    "fifo",
    "cli",
];

/// The info strings a prose fence may carry. Any other first word fails the read, so a
/// misspelled kind cannot drop its block.
const PROSE: [&str; 4] = ["", "rust", "sh", "text"];

/// One fixture block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: String,
    /// The words after the kind.
    pub args: Vec<String>,
    /// The lines between the fences, as written.
    pub lines: Vec<String>,
    /// The 1-based line of the opening fence.
    pub line: usize,
}

impl Block {
    /// The value of a `name=value` argument, such as `locale` in an `entry` block.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.args
            .iter()
            .find_map(|arg| arg.strip_prefix(name)?.strip_prefix('='))
    }
}

pub fn ranking_doc() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/design/ranking.md");
    fs::read_to_string(&path).expect("docs/design/ranking.md is readable")
}

/// Every block of `kind` in `doc`, in order. Panics where [`read`] fails.
pub fn blocks(doc: &str, kind: &str) -> Vec<Block> {
    read(doc)
        .unwrap_or_else(|error| panic!("ranking.md: {error}"))
        .into_iter()
        .filter(|block| block.kind == kind)
        .collect()
}

/// Every fixture block in `doc`. Fences follow CommonMark: up to 3 spaces of indent, a run of
/// 3 or more backticks or tildes, and a closing run of the same character at least as long,
/// with only spaces and tabs after it. Fails on a fence whose first word is neither a fixture
/// kind nor a prose language, on a fence naming a kind that is not an unindented ``` (indented,
/// longer, or tildes), on any fence in a list item or blockquote, on a fence naming a kind inside
/// an open block, on a block that never closes, on a carriage return, and on a line outside a
/// block that starts with `<`, which CommonMark may read as the start of an HTML block, where a
/// fence is not a fence.
pub fn read(doc: &str) -> Result<Vec<Block>, String> {
    if let Some(at) = doc.find('\r') {
        let n = doc[..at].matches('\n').count() + 1;
        return Err(format!(
            "line {n}: a carriage return, which CommonMark reads as a line ending"
        ));
    }
    let mut blocks = Vec::new();
    let mut open: Option<Open> = None;
    for (n, line) in doc.lines().enumerate() {
        let n = n + 1;
        if let Some(fence) = &mut open {
            if fence.closes(line) {
                blocks.extend(fence.block.take());
                open = None;
            } else if names_a_kind(line) {
                return Err(format!(
                    "line {n}: {line:?} names a fixture kind inside the block from line {}",
                    fence.line
                ));
            } else if let Some(block) = &mut fence.block {
                block.lines.push(line.to_owned());
            }
            continue;
        }
        if line.trim_start_matches(' ').starts_with('<') && !line.starts_with("    ") {
            return Err(format!(
                "line {n}: {line:?} starts an HTML block, where CommonMark reads no fence"
            ));
        }
        let Some(fence) = Fence::parse(line) else {
            if names_a_kind(line) {
                return Err(format!(
                    "line {n}: {line:?} must be an unindented ``` fence"
                ));
            }
            if let (true, rest) = in_containers(line)
                && Fence::parse(rest).is_some()
            {
                return Err(format!(
                    "line {n}: {line:?} opens a fence in a list item or blockquote, which the reader does not follow"
                ));
            }
            continue;
        };
        let mut words = fence.info.split_whitespace().map(str::to_owned);
        let kind = words.next().unwrap_or_default();
        let block = if KINDS.contains(&kind.as_str()) {
            if (fence.indent, fence.mark, fence.len) != (0, '`', 3) {
                return Err(format!(
                    "line {n}: {line:?} must be an unindented ``` fence"
                ));
            }
            Some(Block {
                kind,
                args: words.collect(),
                lines: Vec::new(),
                line: n,
            })
        } else if PROSE.contains(&kind.as_str()) {
            None
        } else {
            return Err(format!("line {n}: {line:?} is not a fixture kind"));
        };
        open = Some(Open {
            mark: fence.mark,
            len: fence.len,
            line: n,
            block,
        });
    }
    match open {
        Some(fence) => Err(format!("line {}: the block never closes", fence.line)),
        None => Ok(blocks),
    }
}

/// A fence line as CommonMark reads it.
struct Fence<'l> {
    indent: usize,
    /// `` ` `` or `~`.
    mark: char,
    len: usize,
    info: &'l str,
}

impl Fence<'_> {
    fn parse(line: &str) -> Option<Fence<'_>> {
        let rest = line.trim_start_matches(' ');
        let indent = line.len() - rest.len();
        let mark = rest.chars().next().filter(|&c| matches!(c, '`' | '~'))?;
        let info = rest.trim_start_matches(mark);
        let len = rest.len() - info.len();
        // CommonMark trims only spaces and tabs, so ``` and a no-break space closes nothing.
        let info = info.trim_matches([' ', '\t']);
        // A backtick fence's info string holds no backtick, so ```x``` is inline code.
        let fence = indent <= 3 && len >= 3 && !(mark == '`' && info.contains('`'));
        fence.then_some(Fence {
            indent,
            mark,
            len,
            info,
        })
    }
}

/// The open fence, and its block when it holds a fixture.
struct Open {
    mark: char,
    len: usize,
    line: usize,
    block: Option<Block>,
}

impl Open {
    fn closes(&self, line: &str) -> bool {
        Fence::parse(line).is_some_and(|close| {
            close.mark == self.mark && close.len >= self.len && close.info.is_empty()
        })
    }
}

/// The line opens a fence whose first word is a fixture kind once its indent, blockquote
/// markers, and list markers are gone. CommonMark renders such a fence as a fixture, so a reader
/// that skipped it would drop the block.
fn names_a_kind(line: &str) -> bool {
    let rest = in_containers(line).1;
    let Some(mark) = rest.chars().next().filter(|&c| matches!(c, '`' | '~')) else {
        return false;
    };
    let info = rest.trim_start_matches(mark);
    rest.len() - info.len() >= 3
        && info
            .split_whitespace()
            .next()
            .is_some_and(|word| KINDS.contains(&word))
}

/// The line without its indent, blockquote markers, and list markers, and whether it had a
/// marker.
fn in_containers(line: &str) -> (bool, &str) {
    let mut rest = line.trim_start();
    let mut marked = false;
    loop {
        let marker = if let Some(inner) = rest.strip_prefix('>') {
            Some(inner)
        } else {
            let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            let bullet = if digits.len() < rest.len() {
                digits.strip_prefix(['.', ')'])
            } else {
                rest.strip_prefix(['-', '*', '+'])
            };
            bullet.filter(|inner| inner.starts_with([' ', '\t']))
        };
        match marker {
            Some(inner) => {
                rest = inner.trim_start();
                marked = true;
            }
            None => return (marked, rest),
        }
    }
}

/// Replaces each exact `\u{XXXX}` with its character, so a fixture can show an invisible one.
/// Every other backslash stays, so `C:\Users` reads as written.
pub fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("\\u{") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 3..];
        let escaped = after.find('}').and_then(|end| {
            let c = u32::from_str_radix(&after[..end], 16).ok()?;
            Some((char::from_u32(c)?, end))
        });
        match escaped {
            Some((c, end)) => {
                out.push(c);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("\\u{");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `""` reads as the empty string and `"x"` as `x`. Anything else reads as written.
pub fn unquote(text: &str) -> &str {
    text.strip_prefix('"')
        .and_then(|quoted| quoted.strip_suffix('"'))
        .unwrap_or(text)
}
