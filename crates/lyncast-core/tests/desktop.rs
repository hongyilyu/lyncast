//! The desktop-entry fixtures: every `entry` and `parsed` pair and every `exec` block in
//! docs/design/ranking.md.

mod doc;

use lyncast_core::AppId;
use lyncast_core::desktop::{self, ExecContext, Session, Unparseable, Verdict, tokenize};

/// Each line reads `value => ["arg", ...]` or `value => error: <ExecError>`, expanded with
/// Name `Firefox`, Icon `firefox` (none in an `exec no-icon` block), and the path below.
#[test]
fn exec_fixtures() {
    let blocks = doc::blocks(&doc::ranking_doc(), "exec");
    let lines: Vec<&str> = blocks
        .iter()
        .flat_map(|block| &block.lines)
        .map(String::as_str)
        .collect();
    for start in ["firefox %u =>", "wine C:", "%U =>", "app %i =>"] {
        assert!(
            lines.iter().any(|line| line.starts_with(start)),
            "ranking.md has no exec line starting {start:?}"
        );
    }
    let mut failures = Vec::new();
    for block in &blocks {
        let icon = match block.args.as_slice() {
            [] => Some("firefox"),
            [arg] if arg == "no-icon" => None,
            args => panic!("line {}: unknown exec arguments {args:?}", block.line),
        };
        let context = ExecContext {
            name: "Firefox",
            icon,
            path: "/usr/share/applications/firefox.desktop",
        };
        for line in &block.lines {
            let (value, want) = line
                .split_once(" => ")
                .unwrap_or_else(|| panic!("not an exec line: {line:?}"));
            let got = match tokenize(&doc::unescape(value), &context) {
                Ok(argv) => format!("{argv:?}"),
                Err(error) => format!("error: {error:?}"),
            };
            if got != want {
                failures.push(format!("{value}\n  want {want}\n  got  {got}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "exec fixtures fail:\n{}",
        failures.join("\n")
    );
}

/// Each `entry locale=... desktop=...` block is a file's text, and the `parsed` block after it
/// is the verdict. The file is parsed as `/usr/share/applications/test.desktop`.
#[test]
fn entry_fixtures() {
    let doc = doc::ranking_doc();
    let blocks: Vec<doc::Block> = doc::read(&doc)
        .expect("ranking.md reads")
        .into_iter()
        .filter(|block| block.kind == "entry" || block.kind == "parsed")
        .collect();
    for line in [
        "name: Rechner",
        "name: Sistemski monitor",
        "hidden: NotShownIn",
    ] {
        assert!(
            blocks
                .iter()
                .any(|block| block.kind == "parsed" && block.lines.iter().any(|l| l == line)),
            "ranking.md has no parsed line {line:?}"
        );
    }
    let mut failures = Vec::new();
    for pair in blocks.chunks(2) {
        let [entry, parsed] = pair else {
            panic!("line {}: an entry block needs a parsed block", pair[0].line);
        };
        assert!(
            entry.kind == "entry" && parsed.kind == "parsed",
            "line {}: entry and parsed blocks must alternate",
            entry.line
        );
        let attr = |name: &str| {
            entry
                .attr(name)
                .unwrap_or_else(|| panic!("line {}: no {name}=", entry.line))
        };
        let session = Session::new(&[attr("locale")], attr("desktop"));
        let text = doc::unescape(&format!("{}\n", entry.lines.join("\n")));
        let verdict = desktop::parse(
            AppId("test.desktop".into()),
            "/usr/share/applications/test.desktop",
            &text,
            &session,
        );
        let got = describe(verdict);
        if got != parsed.lines {
            failures.push(format!(
                "line {}\n  want {:?}\n  got  {got:?}",
                entry.line, parsed.lines
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "entry fixtures fail:\n{}",
        failures.join("\n")
    );
}

/// A NUL in a value is kept with the text after it, where GLib ends the value at the NUL. `%c`
/// expands to the whole Name.
#[test]
fn a_nul_in_a_value_is_kept() {
    let verdict = desktop::parse(
        AppId("test.desktop".into()),
        "/usr/share/applications/test.desktop",
        "[Desktop Entry]\nType=Application\nName=Nul\0Tail\nExec=app %c\n",
        &Session::new(&["C"], "niri"),
    );
    assert_eq!(
        describe(verdict),
        [
            "name: Nul\0Tail",
            "unlocalized name: Nul\0Tail",
            "generic name: none",
            "keywords: none",
            "icon: none",
            "argv: [\"app\", \"Nul\\0Tail\"]",
            "terminal: false",
            "try-exec: none",
        ]
    );
}

/// The verdict in the `parsed` spelling.
fn describe(verdict: Result<Verdict, Unparseable>) -> Vec<String> {
    let app = match verdict {
        Err(error) => return vec![format!("unparseable: {error:?}")],
        Ok(Verdict::Hidden(skip)) => return vec![format!("hidden: {skip:?}")],
        Ok(Verdict::Shown(app)) => app,
    };
    let keywords = if app.keywords().is_empty() {
        "none".to_owned()
    } else {
        app.keywords().join(" | ")
    };
    let mut lines = vec![
        format!("name: {}", app.name()),
        format!("unlocalized name: {}", app.unlocalized_name()),
        format!("generic name: {}", app.generic_name().unwrap_or("none")),
        format!("keywords: {keywords}"),
        format!("icon: {}", app.icon().unwrap_or("none")),
        format!("argv: {:?}", app.argv()),
        format!("terminal: {}", app.terminal()),
        format!("try-exec: {}", app.try_exec().unwrap_or("none")),
    ];
    lines.extend(app.actions().iter().map(|action| {
        format!(
            "action {}: {} {:?}",
            action.action.id, action.action.name, action.argv
        )
    }));
    lines
}
