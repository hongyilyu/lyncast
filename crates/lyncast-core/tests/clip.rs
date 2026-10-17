//! Clipboard history's pure rules. Every `retention`, `kind`, and `uris` block in
//! docs/design/clipboard-and-paste.md runs here, so that page is the only copy.

mod doc;

use lyncast_core::clip::{self, Retention, TextKind};
use lyncast_core::rank::Timestamp;

const DAY: i64 = 86_400_000;

/// The doc's blocks of `kind`, failing when the doc has none, so a lost block fails.
fn lines(kind: &str) -> Vec<String> {
    let lines: Vec<String> = doc::CLIPBOARD
        .blocks(kind)
        .into_iter()
        .flat_map(|block| block.lines)
        .collect();
    assert!(
        !lines.is_empty(),
        "clipboard-and-paste.md has no {kind} lines"
    );
    lines
}

/// Splits a fixture line at ` => `, or at a trailing ` =>` with nothing after it.
fn split(line: &str) -> (&str, &str) {
    line.split_once(" => ")
        .or_else(|| line.strip_suffix(" =>").map(|case| (case, "")))
        .unwrap_or_else(|| panic!("not a fixture line: {line:?}"))
}

/// Asserts that every (line, want, got) agrees, listing each that does not.
fn assert_all(kind: &str, results: Vec<(String, String, String)>) {
    let total = results.len();
    let failures: Vec<String> = results
        .into_iter()
        .filter(|(_, want, got)| want != got)
        .map(|(line, want, got)| format!("{line}\n  want {want:?}\n  got  {got:?}"))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {total} {kind} fixtures fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Each line reads `used <n>d ago, pinned|unpinned, keep <days> => kept|expired`.
#[test]
fn retention_fixtures() {
    let now = Timestamp(1_791_072_000_000);
    let results = lines("retention")
        .into_iter()
        .map(|line| {
            let (case, want) = split(&line);
            let parts: Vec<&str> = case.split(", ").collect();
            let [used, pin, keep] = parts.as_slice() else {
                panic!("not a retention line: {line:?}");
            };
            let days: i64 = used
                .strip_prefix("used ")
                .and_then(|used| used.strip_suffix("d ago"))
                .and_then(|days| days.parse().ok())
                .unwrap_or_else(|| panic!("not used <n>d ago: {line:?}"));
            let pinned = match *pin {
                "pinned" => true,
                "unpinned" => false,
                _ => panic!("not pinned or unpinned: {line:?}"),
            };
            let keep = keep
                .strip_prefix("keep ")
                .and_then(|keep| keep.parse().ok())
                .unwrap_or_else(|| panic!("not keep <days>: {line:?}"));
            let used = Timestamp(now.0 - days * DAY);
            let expired = clip::expired(used, pinned, now, Retention::days(keep));
            let got = if expired { "expired" } else { "kept" };
            (line.clone(), want.to_owned(), got.to_owned())
        })
        .collect();
    assert_all("retention", results);
}

/// Each line reads `<text> => text|link|email`, and the text may write `\u{XXXX}`.
#[test]
fn kind_fixtures() {
    let results = lines("kind")
        .into_iter()
        .map(|line| {
            let (text, want) = split(&line);
            let got = match clip::text_kind(&doc::unescape(text)) {
                TextKind::Text => "text",
                TextKind::Link => "link",
                TextKind::Email => "email",
            };
            (line.clone(), want.to_owned(), got.to_owned())
        })
        .collect();
    assert_all("kind", results);
}

/// Each line reads `<line of a list> => <path>`, or ends at `=>` when the line names no local
/// file, and the list line may write `\u{XXXX}`.
#[test]
fn uris_fixtures() {
    let results = lines("uris")
        .into_iter()
        .map(|line| {
            let (list, want) = split(&line);
            let got = clip::paths(&doc::unescape(list));
            (line.clone(), doc::unescape(want), got)
        })
        .collect();
    assert_all("uris", results);
}

/// A property, so no block holds its cases: the preview is the longest prefix of the text that
/// holds at most 1,024 bytes and ends at a character boundary. The texts run past the limit in
/// characters of each UTF-8 length, shifted by up to three bytes.
#[test]
fn a_preview_is_the_longest_whole_prefix_of_1024_bytes() {
    for filler in ['a', 'é', '€', '😀'] {
        let width = filler.len_utf8();
        for lead in 0..4 {
            for count in 1000 / width..1040 / width {
                let text = "a".repeat(lead) + &filler.to_string().repeat(count);
                let preview = clip::preview(&text);
                let case = format!("{lead} a then {count} {filler}");
                assert!(text.starts_with(preview), "{case}");
                assert!(preview.len() <= 1024, "{case}");
                let next = text[preview.len()..].chars().next();
                assert!(
                    next.is_none_or(|c| preview.len() + c.len_utf8() > 1024),
                    "{case}: the next character fits"
                );
            }
        }
    }
}
