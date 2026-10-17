//! The one text fold. The index folds every field once when it is built, and a query is folded
//! once per keystroke, so every comparison in ranking sees the same text. The calculator runs
//! only the width step, [`narrow`].

use std::borrow::Cow;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

/// A text in the fold, in the two forms ranking reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folded {
    kept: String,
    lower: String,
}

impl Folded {
    /// Width-, format-, and diacritic-folded, with ASCII capitals kept and every other letter
    /// lowercased. frizbee scores this form. Its word starts read only ASCII capitals (`stack` on
    /// `OrbStack`), and it cannot pair a non-ASCII capital with a lowercase of another UTF-8
    /// length, such as `ẞ` with `ß`.
    pub fn kept(&self) -> &str {
        &self.kept
    }

    /// `kept`, lowercased. Exactness, prefixes, and past search terms compare this form.
    pub fn lower(&self) -> &str {
        &self.lower
    }

    pub(crate) fn into_parts(self) -> (String, String) {
        (self.kept, self.lower)
    }
}

/// Folds text: full-width forms narrowed, format characters (general category Cf) dropped,
/// then NFD, every character with a nonzero canonical combining class dropped, and NFC. `kept`
/// then lowercases every non-ASCII character, with final sigma `ς` as `σ`, and `lower`
/// lowercases the rest.
///
/// `Ｆｉｒｅｆｏｘ` folds to `Firefox` and `firefox`, and `Ångström` to `Angstrom` and `angstrom`.
pub fn fold(text: &str) -> Folded {
    if text.is_ascii() {
        return Folded {
            kept: text.to_owned(),
            lower: text.to_ascii_lowercase(),
        };
    }
    let narrowed = narrow(text);
    let stripped = narrowed
        .chars()
        .filter(|&c| !is_format(c))
        .nfd()
        .filter(|&c| canonical_combining_class(c) == 0)
        .nfc();
    let mut kept = String::new();
    for c in stripped {
        if c.is_ascii() {
            kept.push(c);
        } else {
            kept.extend(c.to_lowercase().map(|c| if c == 'ς' { 'σ' } else { c }));
        }
    }
    let lower = kept.to_ascii_lowercase();
    Folded { kept, lower }
}

/// Narrows the full-width forms U+FF01 to U+FF5E to ASCII and U+3000 to a space.
pub(crate) fn narrow(text: &str) -> Cow<'_, str> {
    let wide = |c: char| matches!(c, '\u{FF01}'..='\u{FF5E}' | '\u{3000}');
    if !text.chars().any(wide) {
        return Cow::Borrowed(text);
    }
    let narrow = |c: char| match c {
        // The full-width forms sit 0xFEE0 above ASCII 0x21 to 0x7E.
        '\u{FF01}'..='\u{FF5E}' => char::from((u32::from(c) - 0xFEE0) as u8),
        '\u{3000}' => ' ',
        c => c,
    };
    Cow::Owned(text.chars().map(narrow).collect())
}

fn is_format(c: char) -> bool {
    FORMAT
        .iter()
        .any(|&(first, last)| (first..=last).contains(&c))
}

/// Every Cf character in Unicode 15.0, as ranges. unicode-normalization has no general
/// category data.
const FORMAT: [(char, char); 21] = [
    ('\u{00AD}', '\u{00AD}'),
    ('\u{0600}', '\u{0605}'),
    ('\u{061C}', '\u{061C}'),
    ('\u{06DD}', '\u{06DD}'),
    ('\u{070F}', '\u{070F}'),
    ('\u{0890}', '\u{0891}'),
    ('\u{08E2}', '\u{08E2}'),
    ('\u{180E}', '\u{180E}'),
    ('\u{200B}', '\u{200F}'),
    ('\u{202A}', '\u{202E}'),
    ('\u{2060}', '\u{2064}'),
    ('\u{2066}', '\u{206F}'),
    ('\u{FEFF}', '\u{FEFF}'),
    ('\u{FFF9}', '\u{FFFB}'),
    ('\u{110BD}', '\u{110BD}'),
    ('\u{110CD}', '\u{110CD}'),
    ('\u{13430}', '\u{1343F}'),
    ('\u{1BCA0}', '\u{1BCA3}'),
    ('\u{1D173}', '\u{1D17A}'),
    ('\u{E0001}', '\u{E0001}'),
    ('\u{E0020}', '\u{E007F}'),
];
