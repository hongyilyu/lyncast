//! Ranking's fixtures. Every `fold`, `score`, `frecency`, `terms`, and `rank` block in
//! docs/design/ranking.md runs here, so that page is the only copy.

mod doc;

use std::collections::BTreeSet;

use lyncast_core::desktop::{self, Session, Verdict};
use lyncast_core::rank::{Fit, Index, Timestamp, Visits, score};
use lyncast_core::{AppId, Command, Key, Section, fold};

const DAY: i64 = 86_400_000;

/// Splits a fixture line at ` => `.
fn split(line: &str) -> (&str, &str) {
    line.split_once(" => ")
        .unwrap_or_else(|| panic!("not a fixture line: {line:?}"))
}

/// Asserts that every (fixture, want, got) agrees, listing each that does not.
fn assert_all(kind: &str, results: Vec<(String, String, String)>) {
    let total = results.len();
    let failures: Vec<String> = results
        .into_iter()
        .filter(|(_, want, got)| want != got)
        .map(|(fixture, want, got)| format!("{fixture}\n  want {want}\n  got  {got}"))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {total} {kind} fixtures fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Asserts that some line of a `kind` block starts with each of `starts`, so a lost block fails.
fn assert_present(blocks: &[doc::Block], kind: &str, starts: &[&str]) {
    for start in starts {
        assert!(
            blocks
                .iter()
                .flat_map(|block| &block.lines)
                .any(|line| line.starts_with(start)),
            "ranking.md has no {kind} line starting {start:?}"
        );
    }
}

fn one_rank_block(line: usize) -> Result<Vec<doc::Block>, String> {
    Ok(vec![doc::Block {
        kind: "rank".to_owned(),
        args: vec!["rule=3".to_owned()],
        lines: vec!["query x".to_owned()],
        line,
    }])
}

#[test]
fn the_reader_rejects_fences_that_would_drop_a_block() {
    let unindented = |line: usize, fence: &str| {
        Err(format!(
            "line {line}: {fence:?} must be an unindented ``` fence"
        ))
    };
    assert_eq!(
        doc::RANKING.read("```rnak\nquery x\n```\n"),
        Err(r#"line 1: "```rnak" is not a fixture kind"#.to_owned())
    );
    assert_eq!(
        doc::RANKING.read("~~~rnak\nquery x\n~~~\n"),
        Err(r#"line 1: "~~~rnak" is not a fixture kind"#.to_owned())
    );
    assert_eq!(
        doc::RANKING.read("text\n  ```rank\n```\n"),
        unindented(2, "  ```rank")
    );
    assert_eq!(
        doc::RANKING.read("~~~fold\nx => x | x\n~~~\n"),
        unindented(1, "~~~fold")
    );
    assert_eq!(
        doc::RANKING.read("~~~~rank\nquery x\n~~~~\n"),
        unindented(1, "~~~~rank")
    );
    assert_eq!(
        doc::RANKING.read("````rank\nquery x\n````\n"),
        unindented(1, "````rank")
    );
    assert_eq!(
        doc::RANKING.read(" ````rank\nquery x\n ````\n"),
        unindented(1, " ````rank")
    );
    assert_eq!(
        doc::RANKING.read("text\n\n    ```rank\n    query x\n"),
        unindented(3, "    ```rank")
    );
    assert_eq!(
        doc::RANKING.read("- ```rank rule=3\n  query x\n  ```\n"),
        unindented(1, "- ```rank rule=3")
    );
    assert_eq!(
        doc::RANKING.read("1. ```rank rule=3\n   query x\n"),
        unindented(1, "1. ```rank rule=3")
    );
    assert_eq!(
        doc::RANKING.read("> ```rank rule=3\n> query x\n> ```\n"),
        unindented(1, "> ```rank rule=3")
    );
    assert_eq!(
        doc::RANKING.read("```\nprose\n```\n```fold\nx => x | x\n"),
        Err("line 4: the block never closes".to_owned())
    );
    let html = |line: &str| {
        Err(format!(
            "line 1: {line:?} starts an HTML block, where CommonMark reads no fence"
        ))
    };
    assert_eq!(
        doc::RANKING.read("<!--\n```rank rule=3\nquery x\n```\n-->\n"),
        html("<!--")
    );
    assert_eq!(
        doc::RANKING.read("   <div>\n```rank rule=3\nquery x\n```\n</div>\n"),
        html("   <div>")
    );
    // CommonMark closes a list item's fence at its indented closer, so the bare ``` after it
    // opens prose that holds the rank fence. The reader does not follow containers.
    let container = |line: &str| {
        Err(format!(
            "line 1: {line:?} opens a fence in a list item or blockquote, which the reader does not follow"
        ))
    };
    assert_eq!(
        doc::RANKING.read("- ```text\n  example\n  ```\n\n```\n```rank rule=3\nquery x\n```\n"),
        container("- ```text")
    );
    assert_eq!(doc::RANKING.read("> ~~~\n> x\n> ~~~\n"), container("> ~~~"));
    // CommonMark ends a line at a lone CR, and `str::lines` does not, so the rank fence would
    // hide inside the first line.
    assert_eq!(
        doc::RANKING.read("Example:\r```rank rule=3\rquery x\r```\n"),
        Err("line 1: a carriage return, which CommonMark reads as a line ending".to_owned())
    );
}

#[test]
fn the_reader_fails_on_a_kind_fence_inside_an_open_block() {
    assert_eq!(
        doc::RANKING.read("```fold\nx => x | x\n```desktop a\n[Desktop Entry]\n```\n"),
        Err(
            r#"line 3: "```desktop a" names a fixture kind inside the block from line 1"#
                .to_owned()
        )
    );
    assert_eq!(
        doc::RANKING.read("~~~~text\n~~~\n```rank rule=3\nquery x\n```\n~~~~\n"),
        Err(
            r#"line 3: "```rank rule=3" names a fixture kind inside the block from line 1"#
                .to_owned()
        )
    );
}

/// Fences that CommonMark closes or opens, which a reader that only knew bare ``` lines would
/// read out of step.
#[test]
fn the_reader_follows_commonmark_fences() {
    assert_eq!(
        doc::RANKING.read("```rust\nlet x = 1;\n```\n```rank rule=3\nquery x\n```\n"),
        one_rank_block(4)
    );
    assert_eq!(
        doc::RANKING.read("```text\nprose\n``` \n```rank rule=3\nquery x\n```\n"),
        one_rank_block(4)
    );
    assert_eq!(
        doc::RANKING.read("```rank rule=3\nquery x\n   ````\t\n"),
        one_rank_block(1)
    );
    assert_eq!(
        doc::RANKING.read("  ```\nprose\n```\n```rank rule=3\nquery x\n```\n"),
        one_rank_block(4)
    );
    assert_eq!(
        doc::RANKING.read("~~~text\n```\n~~~\n```rank rule=3\nquery x\n```\n"),
        one_rank_block(4)
    );
    assert_eq!(
        doc::RANKING.read("````\n```\n````\n```rank rule=3\nquery x\n```\n"),
        one_rank_block(4)
    );
    assert_eq!(
        doc::RANKING.read("```x```\n```rank rule=3\nquery x\n```\n"),
        one_rank_block(2)
    );
    // Only spaces and tabs may follow a closing fence, so ``` and a no-break space or U+3000 is
    // a line of the block.
    for after in ['\u{A0}', '\u{3000}'] {
        assert_eq!(
            doc::RANKING.read(&format!("```rank rule=3\nquery x\n```{after}\n```\n")),
            Ok(vec![doc::Block {
                kind: "rank".to_owned(),
                args: vec!["rule=3".to_owned()],
                lines: vec!["query x".to_owned(), format!("```{after}")],
                line: 1,
            }])
        );
    }
}

/// Each line reads `input => case-kept | lowercased`, and either side may write `\u{XXXX}`.
#[test]
fn fold_fixtures() {
    let blocks = doc::RANKING.blocks("fold");
    assert_present(&blocks, "fold", &["Ｆｉｒｅｆｏｘ", "Soft\\u{AD}ware", "ｶﾞ"]);
    let results = blocks
        .iter()
        .flat_map(|block| &block.lines)
        .map(|line| {
            let (input, want) = split(line);
            let folded = fold(&doc::unescape(input));
            let got = format!("{} | {}", folded.kept(), folded.lower());
            (line.clone(), doc::unescape(want), got)
        })
        .collect();
    assert_all("fold", results);
}

/// Each line reads `query | text => N shows`, `N below`, `exact`, or `∅`.
#[test]
fn score_fixtures() {
    let blocks = doc::RANKING.blocks("score");
    assert_present(
        &blocks,
        "score",
        &["code | Xcode", "firef | Firefox", "- | Firefox"],
    );
    let results = blocks
        .iter()
        .flat_map(|block| &block.lines)
        .map(|line| {
            let (pair, want) = split(line);
            let (query, text) = pair
                .split_once(" | ")
                .unwrap_or_else(|| panic!("not a score pair: {line:?}"));
            let got = match score(doc::unquote(query), doc::unquote(text)) {
                None => "∅".to_owned(),
                Some(scored) => match scored.fit {
                    Fit::Exact => "exact".to_owned(),
                    Fit::Score(n) if scored.shows => format!("{n} shows"),
                    Fit::Score(n) => format!("{n} below"),
                },
            };
            (line.clone(), want.to_owned(), got)
        })
        .collect();
    assert_all("score", results);
}

/// `0` or a whole number of days such as `10d`, in milliseconds.
fn offset(text: &str) -> i64 {
    match text.strip_suffix('d') {
        Some(days) => days.parse::<i64>().expect("a whole number of days") * DAY,
        None if text == "0" => 0,
        None => panic!("not an offset: {text:?}"),
    }
}

/// Each line reads `visits <offsets> | at <offset> => <score>`, with the visits recorded on one
/// entry with an empty query.
#[test]
fn frecency_fixtures() {
    let blocks = doc::RANKING.blocks("frecency");
    assert_present(
        &blocks,
        "frecency",
        &["visits 0 | at 66d", "visits 0, 10d |"],
    );
    let start = Timestamp(1_791_072_000_000);
    let at = |offset_text: &str| Timestamp(start.0 + offset(offset_text));
    let key = Key::App(AppId("firefox.desktop".into()));
    let results = blocks
        .iter()
        .flat_map(|block| &block.lines)
        .map(|line| {
            let (case, want) = split(line);
            let (visits_at, read_at) = case
                .strip_prefix("visits ")
                .and_then(|case| case.split_once(" | at "))
                .unwrap_or_else(|| panic!("not a frecency line: {line:?}"));
            let mut visits = Visits::default();
            for visit in visits_at.split(", ") {
                visits.record(key.clone(), "", at(visit));
            }
            let score = visits.get(&key).expect("visited").score(at(read_at));
            (line.clone(), want.to_owned(), format!("{score:.3}"))
        })
        .collect();
    assert_all("frecency", results);
}

/// The double-quoted words of `text`, with `""` as the empty word.
fn quoted(mut text: &str) -> Vec<&str> {
    let mut words = Vec::new();
    while let Some(rest) = text.strip_prefix('"') {
        let (word, after) = rest
            .split_once('"')
            .unwrap_or_else(|| panic!("an unclosed quote in {text:?}"));
        words.push(word);
        text = after.trim_start();
    }
    assert!(text.is_empty(), "not a quoted word: {text:?}");
    words
}

/// Each line reads `"query" ... => term | term | term`: the queries recorded on one entry in
/// order, then its terms, most recent first.
#[test]
fn terms_fixtures() {
    let blocks = doc::RANKING.blocks("terms");
    assert_present(&blocks, "terms", &["\"Fire\" \"fire\""]);
    let key = Key::App(AppId("firefox.desktop".into()));
    let results = blocks
        .iter()
        .flat_map(|block| &block.lines)
        .map(|line| {
            let (queries, want) = split(line);
            let mut visits = Visits::default();
            for query in quoted(queries) {
                visits.record(key.clone(), query, Timestamp(1_791_072_000_000));
            }
            let got = visits.get(&key).expect("visited").terms().join(" | ");
            (line.clone(), want.to_owned(), got)
        })
        .collect();
    assert_all("terms", results);
}

/// TinyCast's rules that lyncast keeps. Aliases (1 and 6) and boosted terms (2) are dropped.
const KEPT: [u32; 11] = [3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14];

const SECTIONS: [Section; 4] = [
    Section::Answer,
    Section::Results,
    Section::Suggestions,
    Section::Commands,
];

/// One `rank` block: apps through the real parser, visits in order, the clock, the query,
/// and the expected `<section> => <titles>` lines.
struct Scenario {
    line: usize,
    locale: String,
    desktop: String,
    apps: Vec<(String, Vec<String>)>,
    visits: Vec<(Key, i64, String)>,
    now: i64,
    query: String,
    want: Vec<String>,
}

impl Scenario {
    fn read(block: &doc::Block) -> Scenario {
        let mut scenario = Scenario {
            line: block.line,
            locale: "C".to_owned(),
            desktop: "niri".to_owned(),
            apps: Vec::new(),
            visits: Vec::new(),
            now: 0,
            query: String::new(),
            want: Vec::new(),
        };
        let day = |text: &str| -> i64 {
            text.parse()
                .unwrap_or_else(|_| panic!("line {}: not a day: {text:?}", block.line))
        };
        for line in &block.lines {
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            match word {
                "session" => {
                    for var in rest.split(' ') {
                        match var.split_once('=') {
                            Some(("LANG", value)) => scenario.locale = value.to_owned(),
                            Some(("XDG_CURRENT_DESKTOP", value)) => {
                                scenario.desktop = value.to_owned()
                            }
                            _ => panic!("line {}: unknown session variable {var:?}", block.line),
                        }
                    }
                }
                "app" => {
                    let mut cells = rest.split(" | ").map(str::to_owned);
                    let id = cells.next().expect("split gives one item");
                    scenario.apps.push((id, cells.collect()));
                }
                "visit" => {
                    let mut parts = rest.splitn(3, ' ');
                    let key = parts.next().expect("splitn gives one item");
                    let key: Key = key
                        .parse()
                        .unwrap_or_else(|_| panic!("line {}: not a key: {key:?}", block.line));
                    let at = day(parts.next().unwrap_or_default());
                    let query = doc::unquote(parts.next().unwrap_or_default()).to_owned();
                    scenario.visits.push((key, at, query));
                }
                "now" => scenario.now = day(rest),
                "query" => scenario.query = doc::unescape(doc::unquote(rest)),
                _ if line.contains(" => ") => scenario.want.push(line.clone()),
                _ => panic!("line {}: not a rank line: {line:?}", block.line),
            }
        }
        scenario
    }

    /// The rows as `<section> => <titles>` lines, for every section that has rows or that
    /// the block names.
    fn run(&self) -> Vec<String> {
        let session = Session::new(&[&self.locale], &self.desktop);
        let apps = self
            .apps
            .iter()
            .map(|(id, cells)| {
                let text = format!(
                    "[Desktop Entry]\nType=Application\nExec=true\n{}\n",
                    cells.join("\n")
                );
                let path = format!("/usr/share/applications/{id}");
                match desktop::parse(AppId(id.clone()), &path, &text, &session) {
                    Ok(Verdict::Shown(app)) => *app,
                    other => panic!("line {}: app {id} does not show: {other:?}", self.line),
                }
            })
            .collect();
        let index = Index::new(apps);
        let mut visits = Visits::default();
        for (key, at, query) in &self.visits {
            visits.record(key.clone(), query, Timestamp(at * DAY));
        }
        let rows = index.rows(&self.query, &visits, Timestamp(self.now * DAY));
        assert!(
            rows.is_sorted_by_key(|row| row.section),
            "line {}: rows interleave sections",
            self.line
        );
        SECTIONS
            .into_iter()
            .filter_map(|section| {
                let name = format!("{section:?}").to_lowercase();
                let titles: Vec<&str> = rows
                    .iter()
                    .filter(|row| row.section == section)
                    .map(|row| row.title.as_str())
                    .collect();
                let named = self
                    .want
                    .iter()
                    .any(|line| line.starts_with(&format!("{name} =>")));
                match titles.as_slice() {
                    [] if !named => None,
                    [] => Some(format!("{name} => none")),
                    titles => Some(format!("{name} => {}", titles.join(", "))),
                }
            })
            .collect()
    }
}

#[test]
fn rank_fixtures() {
    let blocks = doc::RANKING.blocks("rank");
    assert_present(
        &blocks,
        "rank",
        &["query \"\"", "query \"   \"", "query -", "query \\u{AD}"],
    );
    let mut failures = Vec::new();
    for block in &blocks {
        let scenario = Scenario::read(block);
        let got = scenario.run();
        if got != scenario.want {
            failures.push(format!(
                "line {}: query {:?}\n  want {:?}\n  got  {got:?}",
                block.line, scenario.query, scenario.want
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} rank blocks fail:\n{}",
        failures.len(),
        blocks.len(),
        failures.join("\n")
    );
}

/// The tags of the rank blocks that pin no comparator rule.
const NAMED: [&str; 4] = ["empty-list", "keywords", "separators", "whitespace"];

/// Every rank block has one tag, `rule=N` for a kept rule or a named tag, and every kept rule
/// and named tag has a block. Some blocks are the only pin of part of a rule, so the test also
/// counts them, and a lost block fails.
#[test]
fn every_rank_block_has_one_tag_and_every_tag_a_block() {
    let blocks = doc::RANKING.blocks("rank");
    assert_eq!(blocks.len(), 51, "ranking.md's rank blocks");
    let mut tags = BTreeSet::new();
    for block in &blocks {
        let [tag] = block.args.as_slice() else {
            panic!(
                "line {}: a rank block needs one tag, not {:?}",
                block.line, block.args
            );
        };
        let known = match tag.strip_prefix("rule=") {
            Some(rule) => rule.parse().is_ok_and(|rule: u32| KEPT.contains(&rule)),
            None => NAMED.contains(&tag.as_str()),
        };
        assert!(known, "line {}: unknown rank tag {tag:?}", block.line);
        tags.insert(tag.clone());
    }
    let want: BTreeSet<String> = KEPT
        .iter()
        .map(|rule| format!("rule={rule}"))
        .chain(NAMED.map(str::to_owned))
        .collect();
    assert_eq!(tags, want);
}

/// ranking.md's `| Command | Keywords |` table lists every command once, with the keywords
/// `Command::keywords` gives it.
#[test]
fn the_keyword_table_is_command_keywords() {
    let doc = doc::RANKING.text();
    let rows: Vec<(String, Vec<String>)> = doc
        .lines()
        .skip_while(|line| *line != "| Command | Keywords |")
        .skip(2)
        .take_while(|line| line.starts_with('|'))
        .map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            let [title, keywords] = cells.as_slice() else {
                panic!("not a two-cell row: {line:?}");
            };
            let keywords = match *keywords {
                "none" => Vec::new(),
                keywords => keywords.split(", ").map(str::to_owned).collect(),
            };
            ((*title).to_owned(), keywords)
        })
        .collect();
    let want: Vec<(String, Vec<String>)> = Command::ALL
        .iter()
        .map(|command| {
            let keywords = command.keywords().iter().map(|&k| k.to_owned()).collect();
            (command.title().to_owned(), keywords)
        })
        .collect();
    assert_eq!(rows, want);
}

/// An index of apps from `Key=Value` lines, through the real parser.
fn index_of(apps: &[(&str, String)]) -> Index {
    let session = Session::new(&["C"], "niri");
    let apps = apps
        .iter()
        .map(|(id, cells)| {
            let text = format!("[Desktop Entry]\nType=Application\nExec=true\n{cells}\n");
            let path = format!("/usr/share/applications/{id}");
            match desktop::parse(AppId((*id).to_owned()), &path, &text, &session) {
                Ok(Verdict::Shown(app)) => *app,
                other => panic!("app {id} does not show: {other:?}"),
            }
        })
        .collect();
    Index::new(apps)
}

fn result_titles(index: &Index, query: &str) -> Vec<String> {
    index
        .rows(query, &Visits::default(), Timestamp(0))
        .into_iter()
        .filter(|row| row.section == Section::Results)
        .map(|row| row.title)
        .collect()
}

/// A field is scored on its first 1,024 bytes. The short field's best alignment is the word
/// start of ` Browser`, 15 points. The long field adds 401 bytes that end in `r`, which would
/// stretch frizbee's match window past 1,024 bytes.
#[test]
fn a_field_over_1024_bytes_scores_as_its_first_1024() {
    let short: String = "b".to_owned()
        + &"rowser"
            .chars()
            .map(|c| "x".repeat(100) + &c.to_string())
            .collect::<String>()
        + &"x".repeat(100)
        + " Browser";
    let long = short.clone() + &"x".repeat(400) + "r";
    assert_eq!((short.len(), long.len()), (715, 1116));
    let fifteen = Some(lyncast_core::rank::Scored {
        fit: Fit::Score(15),
        shows: true,
    });
    assert_eq!(score("browser", &short), fifteen);
    assert_eq!(score("browser", &long), fifteen);
    let index = index_of(&[(
        "firefox.desktop",
        format!("Name=Firefox\nGenericName={long}"),
    )]);
    assert_eq!(result_titles(&index, "browser"), ["Firefox"]);
}

/// A field cut to its first 1,024 bytes is exact only when the whole field equals the query.
/// 1,024 `x`s align with all of a 1,500-`x` Name's cut and score 4 for the first `x` and 2 for
/// each of the other 1,023. So the Name does not take rule 3's standing, and Subtitle, whose
/// subtitle equals the query, comes first by rule 5. 512 `ж`s, 1,024 bytes, score the same way
/// against 600.
#[test]
fn a_field_cut_at_1024_bytes_is_not_exact() {
    let query = "x".repeat(1024);
    let long = "x".repeat(1500);
    let score_of = |fit| {
        Some(lyncast_core::rank::Scored {
            fit: Fit::Score(fit),
            shows: true,
        })
    };
    assert_eq!(score(&query, &long), score_of(2050));
    assert_eq!(score(&"ж".repeat(512), &"ж".repeat(600)), score_of(1026));
    let index = index_of(&[
        ("long.desktop", format!("Name={long}")),
        (
            "subtitle.desktop",
            format!("Name=Subtitle\nGenericName={query}"),
        ),
    ]);
    assert_eq!(result_titles(&index, &query), ["Subtitle".to_owned(), long]);
}

/// A needle longer than every field cannot align, so it shows nothing.
#[test]
fn a_query_longer_than_every_field_shows_nothing() {
    let index = index_of(&[
        ("firefox.desktop", "Name=Firefox".to_owned()),
        ("long.desktop", format!("Name={}", "x".repeat(1024))),
    ]);
    assert_eq!(result_titles(&index, &"x".repeat(1024)), ["x".repeat(1024)]);
    assert_eq!(
        result_titles(&index, &"x".repeat(1025)),
        Vec::<String>::new()
    );
    assert_eq!(
        result_titles(&index, &"firefox ".repeat(128)),
        Vec::<String>::new()
    );
}

/// One visit puts the anchor ln(101)/λ ahead, in whole milliseconds, with λ = ln 2 per 10 days.
/// An anchor at or before now scores 1, so the entry leaves Suggestions' warm rows at its anchor
/// and is warm 1 ms before it.
#[test]
fn a_visit_is_cold_at_its_anchor() {
    let index = index_of(&[("zed.desktop", "Name=Zed".to_owned())]);
    let lambda = std::f64::consts::LN_2 / (10 * DAY) as f64;
    let lead = (101f64.ln() / lambda).round() as i64;
    let start = 1_791_072_000_000;
    let mut visits = Visits::default();
    visits.record(Key::App(AppId("zed.desktop".into())), "", Timestamp(start));
    let suggestions = |now: i64| -> Vec<String> {
        index
            .rows("", &visits, Timestamp(now))
            .into_iter()
            .filter(|row| row.section == Section::Suggestions)
            .map(|row| row.title)
            .collect()
    };
    assert_eq!(suggestions(start + lead - 1), ["Zed", "Clipboard History"]);
    assert_eq!(suggestions(start + lead), ["Clipboard History", "Zed"]);
}

/// Query text over 1,024 bytes gives no rows, before the fold. Both long queries below fold to
/// `firefox`, because the fold drops the combining acutes, and the shorter one still finds
/// Firefox. Text that would fold to the empty query gives no rows either. A full-width letter is
/// 3 bytes and folds to 1, so 342 of them show nothing, although a field holds their fold.
#[test]
fn a_query_over_1024_bytes_shows_nothing() {
    let index = index_of(&[
        ("firefox.desktop", "Name=Firefox".to_owned()),
        ("long.desktop", format!("Name={}", "x".repeat(342))),
    ]);
    let marks = "\u{301}".repeat(508);
    let at_cap = format!("firefox {marks}");
    let over_cap = format!("firefox  {marks}");
    assert_eq!((at_cap.len(), over_cap.len()), (1024, 1025));
    assert_eq!(result_titles(&index, &at_cap), ["Firefox"]);
    assert_eq!(result_titles(&index, &over_cap), Vec::<String>::new());
    let wide = |n: usize| result_titles(&index, &"\u{FF58}".repeat(n));
    assert_eq!(wide(341), ["x".repeat(342)]);
    assert_eq!(wide(342), Vec::<String>::new());
    let rows = |query: &str| index.rows(query, &Visits::default(), Timestamp(0)).len();
    // Clipboard History and the two apps in Suggestions, then the other 8 commands.
    assert_eq!(rows(&" ".repeat(1024)), 11);
    assert_eq!(rows(&" ".repeat(1025)), 0);
}
