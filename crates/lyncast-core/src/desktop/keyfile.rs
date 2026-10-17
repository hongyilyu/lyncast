//! The key-file layer of the Desktop Entry Specification: groups, `key=value` lines, localized
//! keys, and value unescaping, read the way GLib reads them.

use std::collections::HashMap;

/// Why a text is not a desktop entry. Its ID shows nothing, as GLib reads no later file with the
/// same ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unparseable {
    /// The first group is not `[Desktop Entry]`, or a key comes before any group.
    NoDesktopEntryGroup,
    /// This 1-based line is not blank, a comment, a `[group]` header, or `key=value`.
    BadLine(usize),
}

/// A parsed file. The first group is always `[Desktop Entry]`. Values stay raw, borrowed from
/// the text, until a lookup decodes the one it needs.
pub(crate) struct KeyFile<'t> {
    groups: Vec<Group<'t>>,
}

/// One `[name]` group. A repeated group header continues the first group of that name, and a
/// repeated key keeps its last value, as GLib does.
pub(crate) struct Group<'t> {
    name: &'t str,
    keys: HashMap<&'t str, Values<'t>>,
}

/// The values one key has in a group: the unlocalized one and the `Key[locale]` ones.
#[derive(Default)]
struct Values<'t> {
    base: Option<&'t str>,
    localized: Vec<(&'t str, &'t str)>,
}

impl<'t> KeyFile<'t> {
    /// Splits lines on `\n` and drops one trailing `\r`. A leading BOM is skipped, and leading
    /// whitespace is ignored, so an indented `#` is a comment. `Key[locale]` may come before
    /// `Key` or without it.
    pub(crate) fn parse(text: &'t str) -> Result<KeyFile<'t>, Unparseable> {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let mut groups: Vec<Group<'t>> = Vec::new();
        let mut current = None;
        for (n, line) in text.split('\n').enumerate() {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line = line.trim_start_matches(|c: char| c.is_ascii_whitespace());
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                let name = header
                    .trim_end_matches([' ', '\t'])
                    .strip_suffix(']')
                    .ok_or(Unparseable::BadLine(n + 1))?;
                if groups.is_empty() && name != "Desktop Entry" {
                    return Err(Unparseable::NoDesktopEntryGroup);
                }
                current = Some(match groups.iter().position(|group| group.name == name) {
                    Some(at) => at,
                    None => {
                        groups.push(Group {
                            name,
                            keys: HashMap::new(),
                        });
                        groups.len() - 1
                    }
                });
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .map(|(key, value)| {
                    let key = key.trim_end_matches(|c: char| c.is_ascii_whitespace());
                    (
                        key,
                        value.trim_start_matches(|c: char| c.is_ascii_whitespace()),
                    )
                })
                .filter(|(key, _)| !key.is_empty())
                .ok_or(Unparseable::BadLine(n + 1))?;
            let group = current.ok_or(Unparseable::NoDesktopEntryGroup)?;
            let (key, locale) = match key.strip_suffix(']').and_then(|key| key.split_once('[')) {
                Some((key, locale)) => (key, Some(locale)),
                None => (key, None),
            };
            let values = groups[group].keys.entry(key).or_default();
            match locale {
                None => values.base = Some(value),
                Some(locale) => match values
                    .localized
                    .iter_mut()
                    .find(|(seen, _)| *seen == locale)
                {
                    Some(slot) => slot.1 = value,
                    None => values.localized.push((locale, value)),
                },
            }
        }
        if groups.is_empty() {
            return Err(Unparseable::NoDesktopEntryGroup);
        }
        Ok(KeyFile { groups })
    }

    /// The `[Desktop Entry]` group.
    pub(crate) fn entry(&self) -> &Group<'t> {
        &self.groups[0]
    }

    /// The group with this name, such as `Desktop Action new-window`.
    pub(crate) fn group(&self, name: &str) -> Option<&Group<'t>> {
        self.groups.iter().find(|group| group.name == name)
    }
}

impl Group<'_> {
    /// The unlocalized value, decoded.
    pub(crate) fn string(&self, key: &str) -> Option<String> {
        self.localized(key, &[])
    }

    /// The value for the first locale candidate whose value is not empty, else the
    /// unlocalized value. Decoded.
    pub(crate) fn localized(&self, key: &str, locales: &[String]) -> Option<String> {
        self.raw(key, locales, true).map(|raw| decode(raw, false))
    }

    /// The unlocalized value as a `;` list, with empty items dropped.
    pub(crate) fn list(&self, key: &str) -> Option<Vec<String>> {
        self.raw(key, &[], true).map(decode_list)
    }

    /// The list for the first locale candidate whose value is not empty, without the
    /// unlocalized list.
    pub(crate) fn localized_list(&self, key: &str, locales: &[String]) -> Option<Vec<String>> {
        self.raw(key, locales, false).map(decode_list)
    }

    /// `true` for `true` or `1` before trailing ASCII whitespace, as GLib reads a boolean. GLib's
    /// `g_ascii_isspace` is the set `char::is_ascii_whitespace` holds.
    pub(crate) fn boolean(&self, key: &str) -> bool {
        self.raw(key, &[], true).is_some_and(|raw| {
            matches!(
                raw.trim_end_matches(|c: char| c.is_ascii_whitespace()),
                "true" | "1"
            )
        })
    }

    fn raw(&self, key: &str, locales: &[String], base: bool) -> Option<&str> {
        let values = self.keys.get(key)?;
        locales
            .iter()
            .find_map(|locale| {
                values
                    .localized
                    .iter()
                    .find(|&&(candidate, raw)| candidate == locale && !raw.is_empty())
                    .map(|&(_, raw)| raw)
            })
            .or(if base { values.base } else { None })
    }
}

/// Decodes `\s`, `\n`, `\t`, `\r`, and `\\`, and in a list item `\;`. A backslash before any
/// other character stays, so `C:\Users` reads as written.
fn decode(raw: &str, in_list: bool) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') if in_list => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a list on every unescaped `;`, decodes each item, and drops the empty ones.
fn decode_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (at, c) in raw.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ';' => {
                items.push(decode(&raw[start..at], true));
                start = at + 1;
            }
            _ => {}
        }
    }
    items.push(decode(&raw[start..], true));
    items.retain(|item| !item.is_empty());
    items
}
