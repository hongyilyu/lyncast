use std::cell::Cell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::rc::Rc;

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer};
use gpui::{
    Bounds, Context, Entity, EntityInputHandler, IntoElement, Pixels, Render, TestAppContext,
    TestDispatcher, VisualTestContext, Window, WindowBounds, WindowHandle, WindowKind, div,
    prelude::*, px, size,
};
use lyncast_core::AppId;
use lyncast_core::clip::Retention;
use lyncast_core::desktop::{self, App as Entry, Session, Verdict};
use lyncast_core::rank::{Index, Visits};

use super::field::MAX_BYTES;
use super::host::Host;
use super::trace::Trace;
use super::view::inspect::Caret;
use super::view::{FIELD, PaletteView};
use super::{Exit, Mount};
use crate::clip::Clips;
use crate::daemon::{self, Bell, Daemon, Events};
use crate::doc::{self, Block};
use crate::vars::Vars;
use crate::wire::Request;

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui::test]
fn a_window_opens_and_closes(cx: &mut TestAppContext) {
    let closed = Rc::new(Cell::new(0));
    let seen = Rc::clone(&closed);
    cx.update(|cx| {
        cx.on_window_closed(move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    let window = cx
        .update(|cx| cx.open_window(super::options(cx), |_, cx| cx.new(|_| Blank)))
        .expect("the test platform opens a window");
    assert_eq!(cx.windows().len(), 1, "the window is open");
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window takes an update");
    cx.run_until_parked();
    assert_eq!(closed.get(), 1, "the close observer saw the window go");
    assert!(cx.windows().is_empty(), "no window is left");
}

#[gpui::test]
fn the_window_is_placed_as_the_geometry_block_says(cx: &mut TestAppContext) {
    let blocks = doc::PALETTE.blocks("geometry");
    let block = blocks.first().expect("palette.md holds a geometry block");
    let (width, height) = block.lines[0]
        .strip_prefix("window ")
        .and_then(|sizes| sizes.split_once('x'))
        .expect("window <w>x<h>");
    let (display, options) = cx.update(|cx| {
        let display = cx.displays()[0].bounds().size.height;
        (display, super::options(cx))
    });
    let top = block.lines[1..]
        .iter()
        .find_map(|line| line.strip_prefix(&format!("{} => ", f32::from(display))))
        .unwrap_or_else(|| panic!("the geometry block has no line for a display {display:?} tall"));
    let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
        panic!("the window has windowed bounds");
    };
    let pixels = |text: &str| px(text.parse::<f32>().expect("pixels"));
    assert_eq!(bounds.size, size(pixels(width), pixels(height)));
    let WindowKind::LayerShell(shell) = options.kind else {
        panic!("the window is a layer-shell surface");
    };
    assert_eq!(shell.margin, Some((pixels(top), px(0.0), px(0.0), px(0.0))));
    assert_eq!(shell.layer, Layer::Overlay);
    assert_eq!(shell.anchor, Anchor::TOP);
    assert_eq!(
        shell.keyboard_interactivity,
        KeyboardInteractivity::Exclusive
    );
    assert_eq!(shell.namespace, "lyncast");
}

/// model.rs and field.rs hold the palette's rules and stay free of GPUI, so the rules read
/// without the framework and a change to GPUI cannot reach them.
#[test]
fn the_model_never_names_gpui() {
    for (file, text) in [
        ("model.rs", include_str!("model.rs")),
        ("field.rs", include_str!("field.rs")),
    ] {
        for (at, line) in text.lines().enumerate() {
            assert!(
                !line.contains("gpui::") && !line.contains("use gpui"),
                "src/palette/{file}:{} names GPUI: {line}",
                at + 1
            );
        }
    }
}

#[test]
fn every_palette_block_runs_as_the_doc_says() {
    let fixtures = doc::PALETTE
        .read(&doc::PALETTE.text())
        .unwrap_or_else(|error| panic!("palette.md: {error}"));
    let blocks: Vec<&Block> = fixtures
        .iter()
        .filter(|block| block.kind == "palette")
        .collect();
    assert_eq!(
        blocks.len(),
        4,
        "palette.md holds 4 palette blocks, so a lost block fails"
    );
    let failures: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| run(apps(&fixtures), block))
        .collect();
    assert!(
        failures.is_empty(),
        "{} palette.md lines fail:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_caret_stays_in_the_field() {
    let long = "x".repeat(MAX_BYTES);
    let mut palette = Palette::new(Vec::new(), "caret");
    let steps = [
        ("open", Caret::AtStart),
        ("insert $LONG", Caret::AtEnd),
        ("press left", Caret::AtEnd),
        ("press home", Caret::AtStart),
        ("press end", Caret::AtEnd),
        ("press escape", Caret::AtStart),
        ("mark $LONG", Caret::AtEnd),
    ];
    for (step, want) in steps {
        palette
            .step(&step.replace("$LONG", &long))
            .expect("the step runs");
        let window = palette.window().expect("an open window");
        let field = VisualTestContext::from_window(window.into(), &palette.cx)
            .debug_bounds(FIELD)
            .unwrap_or_else(|| panic!("{step}: the field has no bounds"));
        let caret = window
            .update(&mut palette.cx, |view, window, cx| {
                view.caret(field, window, cx)
            })
            .expect("the window takes an update");
        match caret {
            Ok(caret) => assert_eq!(caret, want, "{step}"),
            Err(error) => panic!("{step}: {error}"),
        }
    }
}

#[test]
fn the_selection_stays_in_view() {
    let apps = (1..=300)
        .map(|n| {
            let text = format!("[Desktop Entry]\nType=Application\nName=App {n:03}\nExec=app\n");
            entry(&format!("share/applications/app-{n:03}.desktop"), &text)
        })
        .collect();
    let mut palette = Palette::new(apps, "in-view");
    let inside = |bounds: Option<Bounds<Pixels>>, list: Bounds<Pixels>| {
        bounds.is_some_and(|bounds| bounds.top() >= list.top() && bounds.bottom() <= list.bottom())
    };
    let steps = ["open", "type app"].into_iter();
    for step in steps.chain(["press down"; 250]).chain(["press up"; 250]) {
        palette.step(step).expect("the step runs");
        let (row, _, list) = palette
            .view(PaletteView::selected_bounds)
            .expect("an open window");
        assert!(
            inside(row, list),
            "{step}: the row at {row:?} is outside the list at {list:?}"
        );
    }
    let (_, header, list) = palette
        .view(PaletteView::selected_bounds)
        .expect("an open window");
    assert!(inside(header, list), "the header at {header:?} is hidden");
}

fn run(apps: Vec<Entry>, block: &Block) -> Vec<String> {
    let mut palette = Palette::new(apps, &block.line.to_string());
    let mut failures = Vec::new();
    let mut rest = block.lines.as_slice();
    while let [step, after @ ..] = rest {
        let shown = after
            .iter()
            .take_while(|line| !line.starts_with("$ "))
            .count();
        let (want, rest_after) = after.split_at(shown);
        rest = rest_after;
        let Some(step) = step.strip_prefix("$ ") else {
            failures.push(format!("line {}: not a step: {step}", block.line));
            break;
        };
        let before = palette.recorded().len();
        if let Err(error) = palette.step(step) {
            failures.push(format!("line {}: $ {step}\n  {error}", block.line));
            break;
        }
        let record = |line: &&String| line.starts_with("record: ");
        let want_dump: String = want
            .iter()
            .filter(|line| !record(line))
            .map(|line| format!("{line}\n"))
            .collect();
        let want_record: String = want
            .iter()
            .filter(record)
            .map(|line| format!("{}\n", &line["record: ".len()..]))
            .collect();
        let got_dump = palette.dump();
        let recorded = palette.recorded();
        let got_record = String::from_utf8_lossy(recorded.get(before..).unwrap_or_default());
        if got_dump != want_dump || got_record != want_record {
            failures.push(format!(
                "line {}: $ {step}\n  want {want_dump:?} record {want_record:?}\n  got  {got_dump:?} record {got_record:?}",
                block.line
            ));
        }
    }
    failures
}

fn scratch() -> PathBuf {
    let exe = env::current_exe().expect("the test binary's path");
    let dir = exe.with_file_name("palette-tier3");
    fs::create_dir_all(&dir).expect("the scratch directory");
    dir
}

fn apps(fixtures: &[Block]) -> Vec<Entry> {
    fixtures
        .iter()
        .filter(|block| block.kind == "desktop")
        .map(|block| entry(&block.args[0], &block.lines.join("\n")))
        .collect()
}

fn entry(path: &str, text: &str) -> Entry {
    let id = Path::new(path).file_name().unwrap().to_str().unwrap();
    let session = Session::new(&[""; 0], "");
    match desktop::parse(AppId(id.to_owned()), path, text, &session) {
        Ok(Verdict::Shown(app)) => *app,
        _ => panic!("{path} shows"),
    }
}

struct Palette {
    cx: TestAppContext,
    host: Entity<Host>,
    events: Events,
    record: PathBuf,
}

impl Palette {
    fn new(apps: Vec<Entry>, name: &str) -> Palette {
        let cx = TestAppContext::build(TestDispatcher::new(0), None);
        let record = scratch().join(format!("{}-{name}.runs", process::id()));
        let _ = fs::remove_file(&record);
        let clips = Clips::open(None, Retention::DEFAULT, |_| {}, || {}).unwrap();
        let index = Index::new(apps);
        let daemon = Daemon::new(Vars::default(), index, Visits::default(), None, clips, None);
        let (bell, rung) = Bell::new();
        let (owner, events) = daemon::detached(daemon, &record, bell);
        let mounted = Mount {
            owner,
            rung,
            trace: Trace::OFF,
            exit: Exit::default(),
        };
        let host = cx.update(|cx| super::mount(cx, mounted));
        let palette = Palette {
            cx,
            host,
            events,
            record,
        };
        palette.settle();
        palette
    }

    fn step(&mut self, step: &str) -> Result<(), String> {
        let (verb, text) = step.split_once(' ').unwrap_or((step, ""));
        match (verb, text) {
            ("open" | "toggle", "") => {
                self.events
                    .send_or_drop(daemon::from_socket(Request::Toggle));
            }
            ("type", text) => self.visual()?.simulate_input(text),
            ("press", keys) => self.visual()?.simulate_keystrokes(keys),
            ("insert", text) => {
                let text = text.replace("\\n", "\n").replace("\\r", "\r");
                self.input(|view, window, cx| view.replace_text_in_range(None, &text, window, cx))?;
            }
            ("mark", text) => self.input(|view, window, cx| {
                view.replace_and_mark_text_in_range(None, text, None, window, cx);
            })?,
            ("unmark", "") => self.input(|view, window, cx| view.unmark_text(window, cx))?,
            _ => return Err(format!("no step {step}")),
        }
        self.settle();
        Ok(())
    }

    fn settle(&self) {
        self.cx.run_until_parked();
    }

    fn window(&self) -> Option<WindowHandle<PaletteView>> {
        self.host.read_with(&self.cx, |host, _| host.window())
    }

    fn visual(&self) -> Result<VisualTestContext, String> {
        let window = self.window().ok_or("no window is open")?;
        Ok(VisualTestContext::from_window(window.into(), &self.cx))
    }

    fn input(
        &mut self,
        call: impl FnOnce(&mut PaletteView, &mut Window, &mut Context<PaletteView>),
    ) -> Result<(), String> {
        let window = self.window().ok_or("no window is open")?;
        window
            .update(&mut self.cx, call)
            .map_err(|error| error.to_string())
    }

    fn view<R>(&self, read: impl FnOnce(&PaletteView) -> R) -> Option<R> {
        let window = self.window()?;
        window.read_with(&self.cx, |view, _| read(view)).ok()
    }

    fn dump(&self) -> String {
        self.view(PaletteView::dump)
            .unwrap_or_else(|| "closed\n".to_owned())
    }

    fn recorded(&self) -> Vec<u8> {
        fs::read(&self.record).unwrap_or_default()
    }
}

impl Drop for Palette {
    fn drop(&mut self) {
        self.cx.quit();
        let _ = fs::remove_file(&self.record);
        let _ = fs::remove_file(self.record.with_extension("lock"));
    }
}
