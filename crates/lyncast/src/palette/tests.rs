use std::cell::Cell;
use std::collections::HashSet;
use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, ErrorKind, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command as Process, Stdio};
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer};
use gpui::{
    App, Bounds, Context, Entity, EntityInputHandler, IntoElement, KeyDownEvent, KeyUpEvent,
    Keystroke, Modifiers, Pixels, Render, RenderImage, TestAppContext, TestDispatcher,
    VisualTestContext, Window, WindowBounds, WindowHandle, WindowKind, div, prelude::*, px, size,
};
use lyncast_core::clip::Retention;
use lyncast_core::desktop::{self, App as Entry, Session, Verdict};
use lyncast_core::rank::{Index, Timestamp, Visits};
use lyncast_core::{AppId, Chord, ClipId, Command, Key};

use super::clips::Civil;
use super::field::MAX_BYTES;
use super::host::Host;
use super::icons::{self, CAPACITY, IconRoots, Icons};
use super::preview::Preview;
use super::trace::Trace;
use super::view::inspect::Caret;
use super::view::{FIELD, PaletteView};
use super::zone::Zone;
use super::{Exit, Mount};
use crate::clip::{self, Clips, Seen};
use crate::daemon::{self, Bell, Daemon, Event, Events};
use crate::doc::{self, Block};
use crate::steps::{Printed, Step, steps};
use crate::vars::Vars;
use crate::wire::{Request, Via};

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

#[test]
fn the_model_never_names_gpui() {
    for (file, text) in [
        ("model.rs", include_str!("model.rs")),
        ("clips.rs", include_str!("clips.rs")),
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

const PALETTE_BLOCKS: usize = 29;

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
        PALETTE_BLOCKS,
        "palette.md holds {PALETTE_BLOCKS} palette blocks, so a lost block fails"
    );
    let failures: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| run(&fixtures, block))
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
        ("press escape", Caret::AtStart),
        ("press tab", Caret::AtStart),
        ("insert $LONG", Caret::AtEnd),
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

fn inside(bounds: Option<Bounds<Pixels>>, list: Bounds<Pixels>) -> bool {
    bounds.is_some_and(|bounds| bounds.top() >= list.top() && bounds.bottom() <= list.bottom())
}

/// The bounds of the selected line, of the line before it, and of the list.
fn selected(
    palette: &Palette,
) -> (
    Option<Bounds<Pixels>>,
    Option<Bounds<Pixels>>,
    Bounds<Pixels>,
) {
    palette
        .view(|view, _| view.selected_bounds())
        .expect("an open window")
}

/// Runs each step and checks that the selected line is inside the list after it.
fn keeps_in_view<'s>(palette: &mut Palette, steps: impl IntoIterator<Item = &'s str>) {
    for step in steps {
        palette.step(step).expect("the step runs");
        let (row, _, list) = selected(palette);
        assert!(
            inside(row, list),
            "{step}: the line at {row:?} is outside the list at {list:?}"
        );
    }
}

/// 20 clips that are pinned, then the newest one that is not.
fn pins() -> Vec<String> {
    let pins = (1..=20).map(|n| format!("text pin {n:02} 2026-10-17T08:{n:02}"));
    let pinned = (1..=20).map(|n| format!("pin {n} 2026-10-17T09:{n:02}"));
    pins.chain(pinned)
        .chain(["text landing 2026-10-17T10:00".to_owned()])
        .collect()
}

#[test]
fn the_root_list_keeps_the_selection_in_view() {
    let apps = (1..=300)
        .map(|n| {
            let text = format!("[Desktop Entry]\nType=Application\nName=App {n:03}\nExec=app\n");
            entry(&format!("share/applications/app-{n:03}.desktop"), &text)
        })
        .collect();
    let mut palette = Palette::new(apps, "in-view");
    let steps = ["open", "type app"].into_iter();
    keeps_in_view(
        &mut palette,
        steps.chain(["press down"; 250]).chain(["press up"; 250]),
    );
    let (_, header, list) = selected(&palette);
    assert!(inside(header, list), "the header at {header:?} is hidden");
}

#[test]
fn the_clipboard_screen_keeps_the_pinned_header_in_view() {
    let mut palette = Palette::build(Vec::new(), IconRoots::default(), &pins(), "in-view-pins");
    keeps_in_view(
        &mut palette,
        ["history"].into_iter().chain(["press up"; 20]),
    );
    let (_, header, list) = selected(&palette);
    assert!(
        inside(header, list),
        "the Pinned header at {header:?} is hidden"
    );
}

#[test]
fn a_refresh_keeps_the_scroll_and_the_selection_in_view() {
    let mut palette = Palette::build(Vec::new(), IconRoots::default(), &pins(), "in-view-refresh");
    let steps = ["history"].into_iter().chain(["press up"; 20]);
    for step in steps.chain(["press down"; 10]).chain(["press up"; 3]) {
        palette.step(step).expect("the step runs");
    }
    let (before, _, _) = selected(&palette);
    palette.step("capture text below").expect("the step runs");
    let (after, _, list) = selected(&palette);
    assert_eq!(before, after, "a capture below the selection moves its row");
    assert!(inside(after, list), "the clip at {after:?} left the list");
    for _ in 0..14 {
        palette.step("press down").expect("the step runs");
    }
    palette.step("capture text above").expect("the step runs");
    let (row, _, list) = selected(&palette);
    assert!(
        inside(row, list),
        "a capture above the selection pushed its row to {row:?}, outside the list at {list:?}"
    );
}

#[test]
fn the_clipboard_screen_keeps_the_day_header_in_view() {
    let today: Vec<String> = (1..=30)
        .map(|n| format!("text clip {n:02} 2026-10-17T09:{n:02}"))
        .collect();
    let mut days = Palette::build(Vec::new(), IconRoots::default(), &today, "in-view-days");
    let steps = ["history"].into_iter();
    keeps_in_view(
        &mut days,
        steps.chain(["press down"; 20]).chain(["press up"; 20]),
    );
    let (_, header, list) = selected(&days);
    assert!(
        inside(header, list),
        "the Today header at {header:?} is hidden"
    );
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
    let mut palette = Palette::build(apps, IconRoots::from_vars(&vars), &[], "icons");
    let mut laid_out = HashSet::new();
    let mut step = |palette: &mut Palette, step: &str| {
        palette.step(step).expect("the step runs");
        laid_out.extend(palette.view(|view, _| view.laid_out()).unwrap_or_default());
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

#[test]
fn the_preview_holds_one_image() {
    let clips = clips_block("atlas");
    let mut palette = Palette::build(Vec::new(), IconRoots::default(), &clips, "preview");
    let image = |palette: &Palette| {
        palette
            .view(|view, cx| view.preview_image(cx))
            .flatten()
            .expect("the pane shows an image")
    };
    palette.step("history").expect("the step runs");
    let newest = image(&palette);
    assert!(palette.in_atlas(&newest), "the newest image is drawn");
    palette.step("press down").expect("the step runs");
    let older = image(&palette);
    assert!(palette.in_atlas(&older), "the older image is drawn");
    assert!(
        !palette.in_atlas(&newest),
        "the newest image left the atlas"
    );
    palette.step("press down").expect("the step runs");
    assert!(
        palette
            .view(|view, cx| view.preview_image(cx))
            .flatten()
            .is_none()
    );
    assert!(!palette.in_atlas(&older), "the older image left the atlas");
}

#[test]
fn the_revision_counts_what_is_not_a_read() {
    let mut palette = Palette::new(Vec::new(), "revision");
    let revision = |palette: &Palette| {
        palette
            .host
            .read_with(&palette.cx, |host, _| host.revision())
    };
    palette.step("open").expect("the step runs");
    let opened = revision(&palette);
    palette.step("type abc").expect("the step runs");
    for read in [Request::Status, Request::Clips] {
        palette.events.send_or_drop(daemon::from_socket(read));
    }
    palette.settle();
    assert_eq!(revision(&palette), opened, "reads leave the revision");
    palette.step("capture text one").expect("the step runs");
    assert_eq!(revision(&palette), opened + 1, "a capture moves it");
}

#[test]
fn every_zone_reads_as_the_doc_says() {
    const CHILD: &str = "LYNCAST_ZONE_CHILD";
    if env::var_os(CHILD).is_some() {
        let wall = Zone::load().date(NOW);
        println!("wall {:02}:{:02}", wall.hour, wall.minute);
        return;
    }
    let blocks = doc::PALETTE.blocks("zone");
    let [block] = blocks.as_slice() else {
        panic!("palette.md holds one zone block");
    };
    let fifo = scratch().join(format!("{}-zone.fifo", process::id()));
    let _ = fs::remove_file(&fifo);
    mkfifo(&fifo).expect("the zone FIFO");
    let exe = env::current_exe().expect("the test binary's path");
    let mut failures = Vec::new();
    for line in &block.lines {
        let (tz, want) = line.split_once(" => ").expect("<TZ> => <wall>");
        let tz = tz.replace("$FIFO", fifo.to_str().expect("a UTF-8 path"));
        let mut child = Process::new(&exe)
            .args([
                "--exact",
                "palette::tests::every_zone_reads_as_the_doc_says",
                "--nocapture",
            ])
            .env_clear()
            .env("TZ", tz)
            .env(CHILD, "1")
            .stdout(Stdio::piped())
            .spawn()
            .expect("the test binary runs");
        let deadline = Instant::now() + BUDGET;
        while child.try_wait().expect("the child's status").is_none() {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let out = child.wait_with_output().expect("the child's output");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if !stdout.lines().any(|got| got == format!("wall {want}")) {
            failures.push(format!(
                "line {}: {line}\n  the child printed {stdout}",
                block.line
            ));
        }
    }
    fs::remove_file(&fifo).expect("the FIFO goes");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn clips_block(name: &str) -> Vec<String> {
    let blocks = doc::PALETTE.blocks("clips");
    let block = blocks
        .iter()
        .find(|block| block.args.first().is_some_and(|arg| arg == name))
        .unwrap_or_else(|| panic!("palette.md holds no clips {name} block"));
    block.lines.clone()
}

fn run(fixtures: &[Block], block: &Block) -> Vec<String> {
    let clips = block
        .args
        .iter()
        .find_map(|arg| arg.strip_prefix("clips="))
        .map(|name| {
            let seeds = fixtures.iter().find(|seeds| {
                seeds.kind == "clips" && seeds.args.first().is_some_and(|arg| arg == name)
            });
            seeds.unwrap_or_else(|| panic!("line {}: no clips {name} block", block.line))
        });
    let clips = clips.map_or(&[][..], |clips| clips.lines.as_slice());
    let counts = block.args.iter().any(|arg| arg == "counts");
    let mut palette = Palette::build(
        apps(fixtures),
        IconRoots::default(),
        clips,
        &block.line.to_string(),
    );
    let mut failures = Vec::new();
    for Step {
        line,
        command,
        after,
    } in steps(block)
    {
        let Some(step) = command.strip_prefix("$ ") else {
            failures.push(format!("line {line}: not a step: {command}"));
            break;
        };
        let before = palette.recorded().len();
        if let Err(error) = palette.step(step) {
            failures.push(format!("line {line}: {command}\n  {error}"));
            break;
        }
        let want = Printed::expected(after);
        let mut dump = palette.dump();
        if counts {
            let counts = palette.preview(Preview::counts);
            dump += &format!(
                "loads: {} started, {} stopped, {} decoded, at most {} out\n",
                counts.started, counts.stopped, counts.decoded, counts.most_out
            );
        }
        let recorded = palette.recorded();
        let record = String::from_utf8_lossy(recorded.get(before..).unwrap_or_default());
        let got = Printed {
            stdout: dump,
            record: record.replace(&*palette.tree.to_string_lossy(), "$TREE"),
            ..Printed::default()
        };
        if got != want {
            failures.push(format!(
                "line {line}: {command}\n  want {want:?}\n  got  {got:?}"
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

/// The Tier 3 daemon's clock: 2026-10-17 at 10:00 UTC.
const NOW: Timestamp = Timestamp(1_792_231_200_000);
/// The wall that the Tier 3 blocks read their days on, two hours east of UTC.
const WALL_EAST: i64 = 2 * 3600;
const WALL: Zone = Zone::East(WALL_EAST);

struct Palette {
    cx: TestAppContext,
    host: Entity<Host>,
    events: Events,
    exit: Exit,
    record: PathBuf,
    stalled: Option<Vec<u8>>,
    tree: PathBuf,
}

impl Palette {
    fn new(apps: Vec<Entry>, name: &str) -> Palette {
        Palette::build(apps, IconRoots::default(), &[], name)
    }

    fn build(apps: Vec<Entry>, roots: IconRoots, clips: &[String], name: &str) -> Palette {
        let cx = TestAppContext::build(TestDispatcher::new(0), None);
        // Jobs run on real worker threads, and each wakes the owner task from outside the test
        // scheduler. With parking allowed, the scheduler takes such a wake without recording
        // it as non-determinism, which captures a backtrace each time.
        cx.executor().allow_parking();
        let record = scratch().join(format!("{}-{name}.runs", process::id()));
        let _ = fs::remove_file(&record);
        let tree = tree(&format!("{}-{name}-", process::id()));
        let _ = fs::remove_dir_all(&tree);
        fs::create_dir_all(&tree).expect("the block's tree");
        for program in doc::PALETTE.blocks("executable") {
            let path = tree.join(&program.args[0]);
            fs::create_dir_all(path.parent().expect("a program in a directory")).unwrap();
            fs::write(&path, program.lines.join("\n")).expect("the program");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("its mode");
        }
        fs::write(tree.join("photo.png"), icons::tests::png(3, 2)).expect("photo.png");
        mkfifo(&tree.join("fifo.png")).expect("fifo.png");
        let big = File::create(tree.join("big.png")).expect("big.png");
        big.set_len(super::preview::MAX_FILE_BYTES + 1)
            .expect("big.png's size");
        let mut history = Clips::open(None, Retention::Forever, |_| {}, || {}).unwrap();
        for line in clips {
            let line = line.replace("$TREE", &tree.to_string_lossy());
            seed(&mut history, &line).unwrap_or_else(|error| panic!("clips line {line}: {error}"));
        }
        let index = Index::new(apps);
        let vars = Vars {
            path: Some(tree.join("bin").into_os_string()),
            ..Vars::default()
        };
        let daemon = Daemon::new(vars, None, index, Visits::default(), None, history, None);
        let (bell, rung) = Bell::new();
        let (owner, events) = daemon::detached(daemon, &record, bell, || NOW);
        let exit = Exit::default();
        let mounted = Mount {
            owner,
            roots,
            zone: WALL,
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
            tree,
        };
        palette.settle();
        palette
    }

    fn step(&mut self, step: &str) -> Result<(), String> {
        for part in step.split(" + ") {
            self.act(part)?;
        }
        self.settle();
        Ok(())
    }

    fn act(&mut self, step: &str) -> Result<(), String> {
        let (verb, text) = step.split_once(' ').unwrap_or((step, ""));
        match (verb, text) {
            ("open" | "toggle", "") => {
                self.events
                    .send_or_drop(daemon::from_socket(Request::Toggle));
            }
            ("history", "") => {
                self.events
                    .send_or_drop(daemon::from_socket(Request::Activate {
                        key: Key::Command(Command::ClipboardHistory),
                        via: Via::Chord(Chord::Enter),
                        confirmed: false,
                        query: String::new(),
                    }));
            }
            ("capture", line) => {
                let line = line.replace("$TREE", &self.tree.to_string_lossy());
                let (kind, value) = line.split_once(' ').ok_or("capture <kind> <value>")?;
                let entry = captured(kind, value)?;
                self.events.send_or_drop(Event::Seen(Seen::Entry(entry)));
            }
            ("type", text) => self.visual()?.simulate_input(text),
            ("press", keys) => self.visual()?.simulate_keystrokes(keys),
            ("dispatch", keys) => {
                let window = self.window().ok_or("no window is open")?;
                for key in keys.split(' ') {
                    self.cx.dispatch_keystroke(window.into(), keystroke(key)?);
                }
            }
            ("decode", "") => {
                let scheduler = self.cx.dispatcher.scheduler();
                while scheduler.pending_task_counts().1 == 0 && scheduler.tick() {}
                while scheduler.tick_background_only() {}
            }
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
        mkfifo(&self.record)?;
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
        let icons = self.host.read_with(&self.cx, |host, _| host.shared().icons);
        icons.read_with(&self.cx, |icons, _| read(icons))
    }

    fn preview<R>(&self, read: impl FnOnce(&Preview) -> R) -> R {
        let preview = self
            .host
            .read_with(&self.cx, |host, _| host.shared().preview);
        preview.read_with(&self.cx, |preview, _| read(preview))
    }

    fn in_atlas(&mut self, image: &RenderImage) -> bool {
        let window = self.window().expect("an open window");
        window
            .update(&mut self.cx, |_, window, _| {
                window.has_image_atlas_entry(image)
            })
            .expect("the window takes an update")
    }

    fn view<R>(&self, read: impl FnOnce(&PaletteView, &App) -> R) -> Option<R> {
        let window = self.window()?;
        window.read_with(&self.cx, read).ok()
    }

    fn dump(&self) -> String {
        let dump = self.view(|view, cx| view.dump(cx));
        let dump = dump.unwrap_or_else(|| match &*self.exit.outcome.borrow() {
            None => "closed\n".to_owned(),
            Some(Ok(())) => "stopped\n".to_owned(),
            Some(Err(error)) => format!("stopped: {error}\n"),
        });
        dump.replace(&*self.tree.to_string_lossy(), "$TREE")
    }

    fn recorded(&self) -> Vec<u8> {
        match &self.stalled {
            Some(kept) => kept.clone(),
            None => fs::read(&self.record).unwrap_or_default(),
        }
    }
}

fn seed(clips: &mut Clips, line: &str) -> Result<(), String> {
    let (line, source) = match line.rsplit_once(" from=") {
        Some((line, app)) => (line, Some(app.to_owned())),
        None => (line, None),
    };
    let (rest, time) = line.rsplit_once(' ').ok_or("<kind> <value> <time>")?;
    let at = wall_time(time)
        .ok_or_else(|| format!("not a time {time}"))?
        .timestamp(WALL_EAST);
    match rest.split_once(' ').ok_or("<kind> <value>")? {
        ("pin", id) => clips.pin(ClipId(id.parse().map_err(|_| "pin <id>")?), at),
        (kind, value) => {
            clips.record(captured(kind, value)?.with_source(source), at);
            Ok(())
        }
    }
}

/// A `YYYY-MM-DDTHH:MM` wall time.
fn wall_time(text: &str) -> Option<Civil> {
    let (date, time) = text.split_once('T')?;
    let mut date = date.splitn(3, '-').map(str::parse::<u32>);
    let (hour, minute) = time.split_once(':')?;
    Some(Civil {
        year: i64::from(date.next()?.ok()?),
        month: date.next()?.ok()?,
        day: date.next()?.ok()?,
        hour: hour.parse().ok()?,
        minute: minute.parse().ok()?,
    })
}

fn captured(kind: &str, value: &str) -> Result<clip::Entry, String> {
    let value = value.replace("\\n", "\n");
    Ok(match kind {
        "text" => clip::Entry::captured_text(&value),
        "files" => clip::Entry::captured_files(&value).ok_or("no local file")?,
        "image" => {
            let size = value.split_once('x').and_then(|(width, height)| {
                Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
            });
            let (width, height) = size.ok_or("image <w>x<h>")?;
            let png = if u64::from(width) * u64::from(height) > super::preview::MAX_PIXELS {
                png_header(width, height)
            } else {
                icons::tests::png(width, height)
            };
            clip::Entry::captured_image(png)
        }
        _ => return Err(format!("no clip kind {kind}")),
    })
}

fn png_header(width: u32, height: u32) -> Vec<u8> {
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let body = [kind.as_slice(), data].concat();
        let length = u32::try_from(data.len()).expect("a short chunk");
        [
            &length.to_be_bytes()[..],
            &body,
            &crc32fast::hash(&body).to_be_bytes(),
        ]
        .concat()
    };
    let header = [
        &width.to_be_bytes()[..],
        &height.to_be_bytes(),
        &[8, 6, 0, 0, 0],
    ]
    .concat();
    [
        &b"\x89PNG\r\n\x1a\n"[..],
        &chunk(b"IHDR", &header),
        &chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]),
        &chunk(b"IEND", &[]),
    ]
    .concat()
}

impl Drop for Palette {
    fn drop(&mut self) {
        self.cx.quit();
        let _ = fs::remove_file(&self.record);
        let _ = fs::remove_file(self.record.with_extension("lock"));
        let _ = fs::remove_dir_all(&self.tree);
    }
}

const STABLE_TREE_PATH_BYTES: usize = 240;

fn tree(name: &str) -> PathBuf {
    let start = scratch().join(name);
    let start = start.to_str().expect("a UTF-8 scratch path");
    let pad = STABLE_TREE_PATH_BYTES
        .checked_sub(start.len())
        .unwrap_or_else(|| panic!("{start} is over {STABLE_TREE_PATH_BYTES} bytes"));
    PathBuf::from(format!("{start}{}", "x".repeat(pad)))
}

pub(super) fn mkfifo(path: &Path) -> Result<(), String> {
    let path = CString::new(path.as_os_str().as_bytes()).expect("a path with no NUL");
    // SAFETY: `path` is a NUL-terminated string that lives through the call.
    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
        return Err(format!("mkfifo: {}", io::Error::last_os_error()));
    }
    Ok(())
}
