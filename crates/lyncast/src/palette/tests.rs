use std::cell::Cell;
use std::collections::HashSet;
use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer};
use gpui::{
    Bounds, Context, Entity, EntityInputHandler, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke,
    Modifiers, Pixels, Render, RenderImage, TestAppContext, TestDispatcher, VisualTestContext,
    Window, WindowBounds, WindowHandle, WindowKind, div, prelude::*, px, size,
};
use lyncast_core::AppId;
use lyncast_core::clip::Retention;
use lyncast_core::desktop::{self, App as Entry, Session, Verdict};
use lyncast_core::rank::{Index, Visits};

use super::field::MAX_BYTES;
use super::host::Host;
use super::icons::{self, CAPACITY, IconRoots, Icons};
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
        17,
        "palette.md holds 17 palette blocks, so a lost block fails"
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
        ("press escape", Caret::AtStart),
        ("press ctrl-k", Caret::AtStart),
        ("mark $LONG", Caret::AtEnd),
        ("insert $LONG", Caret::AtEnd),
        ("press home", Caret::AtStart),
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

#[test]
fn icons_stay_bounded() {
    let dir = scratch().join(format!("{}-icons", process::id()));
    let _ = fs::remove_dir_all(&dir);
    let png = icons::tests::png(48, 48);
    let apps = (1..=400)
        .map(|n| {
            let path = dir.join(format!("icons/hicolor/48x48/apps/zebra-{n:03}.png"));
            fs::create_dir_all(path.parent().expect("a directory")).unwrap();
            fs::write(&path, &png).unwrap();
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=Zebra {n:03}\nIcon=zebra-{n:03}\nExec=zebra\n"
            );
            entry(&format!("share/applications/zebra-{n:03}.desktop"), &text)
        })
        .collect();
    let vars = Vars {
        data_dirs: Some(dir.clone().into_os_string()),
        ..Vars::default()
    };
    let mut palette = Palette::with_icons(apps, IconRoots::from_vars(&vars), "icons");
    let mut laid_out = HashSet::new();
    let mut step = |palette: &mut Palette, step: &str| {
        palette.step(step).expect("the step runs");
        laid_out.extend(palette.view(PaletteView::laid_out).unwrap_or_default());
        let held = palette.icons(Icons::len);
        let apps = laid_out
            .iter()
            .filter(|title| title.starts_with("Zebra"))
            .count();
        assert_eq!(
            held,
            apps.min(CAPACITY),
            "{step}: the cache holds {held} icons for {apps} app rows laid out"
        );
    };
    let first = icons::key("zebra-001");
    step(&mut palette, "open");
    let image = palette
        .icons(|icons| icons.peek(&first))
        .expect("Zebra 001's icon loaded");
    assert!(palette.in_atlas(&image), "Zebra 001's icon is drawn");
    step(&mut palette, "toggle");
    step(&mut palette, "toggle");
    let again = palette.icons(|icons| icons.peek(&first));
    assert!(
        again.is_some_and(|again| Arc::ptr_eq(&again, &image)),
        "the next summon has the same image"
    );
    assert!(palette.in_atlas(&image), "the next summon draws it");
    step(&mut palette, "insert zebra");
    for _ in 1..400 {
        step(&mut palette, "press down");
    }
    assert!(palette.icons(|icons| icons.peek(&first)).is_none());
    assert!(!palette.in_atlas(&image), "Zebra 001's icon left the atlas");
    let last = palette
        .icons(|icons| icons.peek(&icons::key("zebra-400")))
        .expect("Zebra 400's icon loaded");
    assert!(palette.in_atlas(&last), "Zebra 400's icon is drawn");
    drop(palette);
    fs::remove_dir_all(&dir).expect("the icon tree goes");
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

fn keystroke(key: &str) -> Result<Keystroke, String> {
    Keystroke::parse(key).map_err(|error| error.to_string())
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

const BUDGET: Duration = Duration::from_secs(60);

struct Palette {
    cx: TestAppContext,
    host: Entity<Host>,
    events: Events,
    exit: Exit,
    record: PathBuf,
    stalled: Option<Vec<u8>>,
}

impl Palette {
    fn new(apps: Vec<Entry>, name: &str) -> Palette {
        Palette::with_icons(apps, IconRoots::default(), name)
    }

    fn with_icons(apps: Vec<Entry>, roots: IconRoots, name: &str) -> Palette {
        let cx = TestAppContext::build(TestDispatcher::new(0), None);
        // Jobs run on real worker threads, and each wakes the owner task from outside the test
        // scheduler. With parking allowed, the scheduler takes such a wake without recording
        // it as non-determinism, which captures a backtrace each time.
        cx.executor().allow_parking();
        let record = scratch().join(format!("{}-{name}.runs", process::id()));
        let _ = fs::remove_file(&record);
        let clips = Clips::open(None, Retention::DEFAULT, |_| {}, || {}).unwrap();
        let index = Index::new(apps);
        let daemon = Daemon::new(Vars::default(), index, Visits::default(), None, clips, None);
        let (bell, rung) = Bell::new();
        let (owner, events) = daemon::detached(daemon, &record, bell);
        let exit = Exit::default();
        let mounted = Mount {
            owner,
            roots,
            rung,
            trace: Trace::OFF,
            exit: exit.clone(),
        };
        let host = cx.update(|cx| super::mount(cx, mounted));
        let palette = Palette {
            cx,
            host,
            events,
            exit,
            record,
            stalled: None,
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
            ("hold", key) => self.visual()?.simulate_event(KeyDownEvent {
                keystroke: keystroke(key)?,
                is_held: true,
                prefer_character_input: false,
            }),
            ("release", key) => self.visual()?.simulate_event(KeyUpEvent {
                keystroke: keystroke(key)?,
            }),
            ("mods", held) => {
                let modifiers = match held {
                    "none" => Modifiers::none(),
                    "ctrl" => Modifiers::control(),
                    "shift" => Modifiers::shift(),
                    "alt" => Modifiers::alt(),
                    _ => return Err(format!("no modifiers {held}")),
                };
                self.visual()?.simulate_modifiers_change(modifiers);
            }
            ("stall-runner", "") => self.stall()?,
            ("resume-runner", "") => self.resume()?,
            ("focus", "") => self.visual()?.update(|window, _| window.activate_window()),
            ("blur", "") => self.visual()?.deactivate_window(),
            _ => return Err(format!("no step {step}")),
        }
        self.settle();
        Ok(())
    }

    fn settle(&self) {
        let deadline = Instant::now() + BUDGET;
        self.cx.run_until_parked();
        while self.stalled.is_none() && !self.idle() {
            assert!(
                Instant::now() < deadline,
                "a job ran over the test's budget"
            );
            thread::sleep(Duration::from_millis(1));
            self.cx.run_until_parked();
        }
        self.cx.run_until_parked();
    }

    fn idle(&self) -> bool {
        self.host.read_with(&self.cx, |host, _| host.idle())
    }

    /// Makes the runner's file a FIFO, so the next program the daemon starts waits in the
    /// runner, opening the file to write its line, and its activation stays out.
    fn stall(&mut self) -> Result<(), String> {
        if self.stalled.is_some() {
            return Err("the runner is stalled already".to_owned());
        }
        let kept = self.recorded();
        fs::remove_file(&self.record).or_else(|error| match error.kind() {
            ErrorKind::NotFound => Ok(()),
            _ => Err(error.to_string()),
        })?;
        let path = CString::new(self.record.as_os_str().as_bytes()).expect("a path with no NUL");
        // SAFETY: `path` is a NUL-terminated string that lives through the call.
        if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
            return Err(format!("mkfifo: {}", io::Error::last_os_error()));
        }
        self.stalled = Some(kept);
        Ok(())
    }

    /// Lets the waiting program write its line, and makes the runner's file a file again. The
    /// open for reading waits for the program's open for writing, and the read ends once the
    /// program has written its line and closed the file.
    fn resume(&mut self) -> Result<(), String> {
        let mut kept = self.stalled.take().ok_or("the runner is not stalled")?;
        if !self.idle() {
            let mut fifo = File::open(&self.record).map_err(|error| error.to_string())?;
            fifo.read_to_end(&mut kept)
                .map_err(|error| error.to_string())?;
        }
        fs::remove_file(&self.record).map_err(|error| error.to_string())?;
        fs::write(&self.record, kept).map_err(|error| error.to_string())
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

    fn icons<R>(&self, read: impl FnOnce(&Icons) -> R) -> R {
        let icons = self.host.read_with(&self.cx, |host, _| host.icons());
        icons.read_with(&self.cx, |icons, _| read(icons))
    }

    fn in_atlas(&mut self, image: &RenderImage) -> bool {
        let window = self.window().expect("an open window");
        window
            .update(&mut self.cx, |_, window, _| {
                window.has_image_atlas_entry(image)
            })
            .expect("the window takes an update")
    }

    fn view<R>(&self, read: impl FnOnce(&PaletteView) -> R) -> Option<R> {
        let window = self.window()?;
        window.read_with(&self.cx, |view, _| read(view)).ok()
    }

    fn dump(&self) -> String {
        self.view(PaletteView::dump)
            .unwrap_or_else(|| match &*self.exit.outcome.borrow() {
                None => "closed\n".to_owned(),
                Some(Ok(())) => "stopped\n".to_owned(),
                Some(Err(error)) => format!("stopped: {error}\n"),
            })
    }

    fn recorded(&self) -> Vec<u8> {
        match &self.stalled {
            Some(kept) => kept.clone(),
            None => fs::read(&self.record).unwrap_or_default(),
        }
    }
}

impl Drop for Palette {
    fn drop(&mut self) {
        self.cx.quit();
        let _ = fs::remove_file(&self.record);
        let _ = fs::remove_file(self.record.with_extension("lock"));
    }
}
