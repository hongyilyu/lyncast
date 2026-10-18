//! Offsets are UTF-8 byte offsets on char boundaries. GPUI's input handler speaks UTF-16, so the
//! field converts at its edge. This is lyncast's own code. GPUI's
//! `examples/input.rs` (Apache-2.0) was read as documentation of the input handler, and none of
//! it is copied.

use std::ops::Range;

/// The most bytes the field holds, as many as ranking and the calculator read.
pub const MAX_BYTES: usize = 1024;

#[derive(Default)]
pub struct Field {
    text: String,
    cursor: usize,
    /// The text an input method is composing. It may be empty: a compose sequence that has
    /// begun shows nothing yet.
    marked: Option<Range<usize>>,
}

impl Field {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// An input method is composing, so the placeholder stays hidden even with no text.
    pub fn composing(&self) -> bool {
        self.marked.is_some()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    /// Writes `text` over `utf16`, else over the composing text, else at the caret, and ends any
    /// composition. Gives whether the text changed.
    pub fn replace(&mut self, utf16: Option<Range<usize>>, text: &str) -> bool {
        let range = self.target(utf16);
        let removed = !range.is_empty();
        let written = self.splice(range, text);
        self.cursor = written.end;
        self.marked = None;
        removed || !written.is_empty()
    }

    /// [`Field::replace`] that marks what it wrote as composing. `selected` is the caret's
    /// place inside the new text, in UTF-16 from its start.
    pub fn mark(
        &mut self,
        utf16: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
    ) -> bool {
        let range = self.target(utf16);
        let removed = !range.is_empty();
        let written = self.splice(range, text);
        self.cursor = match selected {
            Some(selected) => {
                written.start + utf16_to_byte(&self.text[written.clone()], selected.end)
            }
            None => written.end,
        };
        let changed = removed || !written.is_empty();
        self.marked = Some(written);
        changed
    }

    /// Keeps the composing text as typed text.
    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Removes the char before the caret.
    pub fn backspace(&mut self) -> bool {
        self.marked = None;
        let Some((at, _)) = self.text[..self.cursor].char_indices().next_back() else {
            return false;
        };
        self.text.replace_range(at..self.cursor, "");
        self.cursor = at;
        true
    }

    /// Removes the char after the caret.
    pub fn delete(&mut self) -> bool {
        self.marked = None;
        let Some(next) = self.text[self.cursor..].chars().next() else {
            return false;
        };
        self.text
            .replace_range(self.cursor..self.cursor + next.len_utf8(), "");
        true
    }

    pub fn left(&mut self) {
        self.marked = None;
        if let Some((at, _)) = self.text[..self.cursor].char_indices().next_back() {
            self.cursor = at;
        }
    }

    pub fn right(&mut self) {
        self.marked = None;
        if let Some(next) = self.text[self.cursor..].chars().next() {
            self.cursor += next.len_utf8();
        }
    }

    pub fn home(&mut self) {
        self.marked = None;
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.marked = None;
        self.cursor = self.text.len();
    }

    /// Empties the field. Gives whether it held text.
    pub fn clear(&mut self) -> bool {
        let changed = !self.text.is_empty();
        *self = Field::default();
        changed
    }

    /// The UTF-16 offset of byte `offset`.
    pub fn utf16_offset(&self, offset: usize) -> usize {
        self.text[..offset].chars().map(char::len_utf16).sum()
    }

    /// The byte offset of UTF-16 offset `offset`, on the char boundary at or before it.
    pub fn byte_offset(&self, offset: usize) -> usize {
        utf16_to_byte(&self.text, offset)
    }

    fn target(&self, utf16: Option<Range<usize>>) -> Range<usize> {
        match utf16 {
            Some(range) => {
                let (start, end) = (self.byte_offset(range.start), self.byte_offset(range.end));
                start.min(end)..start.max(end)
            }
            None => self.marked.clone().unwrap_or(self.cursor..self.cursor),
        }
    }

    fn splice(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        let line = text.replace("\r\n", " ").replace(['\n', '\r'], " ");
        let room = MAX_BYTES.saturating_sub(self.text.len() - range.len());
        let kept = &line[..line.floor_char_boundary(room)];
        let start = range.start;
        self.text.replace_range(range, kept);
        start..start + kept.len()
    }
}

fn utf16_to_byte(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (at, char) in text.char_indices() {
        units += char.len_utf16();
        if units > offset {
            return at;
        }
    }
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_survive_utf16() {
        for text in ["", "firefox", "é8×8", "日本語", "a😀b", "😀😀", "x\u{301}y"] {
            let field = Field {
                text: text.to_owned(),
                ..Field::default()
            };
            let mut units = 0;
            for (at, char) in text.char_indices().chain([(text.len(), ' ')]) {
                assert_eq!(field.utf16_offset(at), units, "{text:?} at byte {at}");
                assert_eq!(field.byte_offset(units), at, "{text:?} at unit {units}");
                if char.len_utf16() == 2 && at < text.len() {
                    assert_eq!(field.byte_offset(units + 1), at, "{text:?} inside a pair");
                }
                units += char.len_utf16();
            }
        }
    }

    #[test]
    fn the_field_holds_at_most_max_bytes() {
        for unit in ["a", "é", "日", "😀", "a\nb"] {
            let mut field = Field::default();
            field.replace(None, "xy");
            field.left();
            let long = unit.repeat(MAX_BYTES);
            assert!(
                field.replace(None, &long),
                "{unit:?}: the insert changes the text"
            );
            let text = field.text();
            let line = long.replace('\n', " ");
            let room = MAX_BYTES - 2;
            let kept = &line[..line.floor_char_boundary(room)];
            assert_eq!(text, format!("x{kept}y"), "{unit:?}");
            assert!(text.len() <= MAX_BYTES && text.len() + unit.len() > MAX_BYTES);
            assert_eq!(
                field.cursor(),
                1 + kept.len(),
                "{unit:?}: the caret follows the insert"
            );
            assert!(
                !field.replace(None, unit),
                "{unit:?}: a full field takes no more"
            );
        }
    }
}
