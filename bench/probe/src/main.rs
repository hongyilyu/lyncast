//! `summon-probe <socket>` starts with no window and reads lines on the Unix
//! socket: `open` opens the window, `close` closes it, `quit` exits. Each step
//! prints `PROBE seq=<n> ev=<name> mono_ns=<CLOCK_MONOTONIC ns>` on stdout,
//! which is the protocol bench/summon.py reads.
use futures::StreamExt;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, QuitMode, SharedString, Window, WindowBounds,
    WindowKind, WindowOptions, div, layer_shell::*, point, prelude::*, px, rgb, size, uniform_list,
};
use std::io::{BufRead, BufReader};
use std::rc::Rc;

fn mono_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

fn log(seq: u32, ev: &str) {
    println!("PROBE seq={seq} ev={ev} mono_ns={}", mono_ns());
}

struct Launcher {
    seq: u32,
    logged_first: bool,
    items: Rc<Vec<SharedString>>,
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if !self.logged_first {
            self.logged_first = true;
            let seq = self.seq;
            log(seq, "first_render");
            // open_window() lays out and paints synchronously but does not
            // present. This runs at the start of the first platform
            // request_frame, just before draw and present.
            window.on_next_frame(move |window, cx| {
                log(seq, "frame1_pre_present");
                let handle = window.window_handle();
                // A foreground task runs only after this request_frame (draw,
                // queue submit, surface present, wl_surface.commit) returns.
                cx.spawn(async move |cx| {
                    log(seq, "frame1_presented");
                    // The next frame GPUI draws. GPUI throttles an unfocused
                    // window to 30 fps, so under anvil this comes about 35 ms
                    // after the present (notes/05).
                    let _ = cx.update_window(handle, move |_, window, _| {
                        window.on_next_frame(move |_, _| log(seq, "frame1_done"));
                    });
                })
                .detach();
            });
        }
        let items = self.items.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .text_color(rgb(0xe0e0e0))
            .font_family("DejaVu Sans")
            .border_1()
            .border_color(rgb(0x5080ff))
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xl()
                    .border_b_1()
                    .border_color(rgb(0x444444))
                    .child(format!("summon-probe #{}: type to search...", self.seq)),
            )
            .child(
                uniform_list("results", items.len(), move |range, _window, _cx| {
                    range
                        .map(|ix| div().id(ix).px_3().h(px(24.)).child(items[ix].clone()))
                        .collect::<Vec<_>>()
                })
                .flex_1(),
            )
    }
}

fn open_launcher(cx: &mut App, seq: u32, items: Rc<Vec<SharedString>>) -> AnyWindowHandle {
    let opts = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(800.), px(500.)),
        })),
        app_id: Some("summon-probe".to_string()),
        focus: true,
        show: true,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "summon-probe".to_string(),
            layer: Layer::Overlay,
            // Anchored to the top only, so the compositor centers it
            // horizontally and the margin pushes it down.
            anchor: Anchor::TOP,
            margin: Some((px(120.), px(0.), px(0.), px(0.))),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        ..Default::default()
    };
    let h = cx
        .open_window(opts, |_, cx| {
            cx.new(|_| Launcher {
                seq,
                logged_first: false,
                items,
            })
        })
        .expect("open_window failed");
    log(seq, "window_opened");
    h.into()
}

fn make_items() -> Rc<Vec<SharedString>> {
    Rc::new(
        (0..1000)
            .map(|i| {
                SharedString::from(format!(
                    "Result {i:04}  Some App or Command Name  ~/path/to/thing-{i}"
                ))
            })
            .collect(),
    )
}

fn main() {
    env_logger::init();
    let sock = std::env::args()
        .nth(1)
        .expect("usage: summon-probe <socket>");
    let app = gpui_platform::application().with_quit_mode(QuitMode::Explicit);
    app.run(move |cx: &mut App| {
        let items = make_items();
        let listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind");
        log(0, "app_ready");
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<String>();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else { continue };
                for line in BufReader::new(conn).lines() {
                    let Ok(cmd) = line else { break };
                    let _ = tx.unbounded_send(cmd);
                }
            }
        });
        cx.spawn(async move |cx| {
            let mut current: Option<AnyWindowHandle> = None;
            let mut seq = 0u32;
            while let Some(cmd) = rx.next().await {
                match cmd.as_str() {
                    "open" if current.is_none() => {
                        seq += 1;
                        log(seq, "toggle_handled_open");
                        current = Some(cx.update(|cx| open_launcher(cx, seq, items.clone())));
                    }
                    "close" => {
                        if let Some(h) = current.take() {
                            cx.update_window(h, |_, window, _| window.remove_window())
                                .expect("the window is open");
                        }
                    }
                    "quit" => {
                        cx.update(|cx| cx.quit());
                        break;
                    }
                    _ => {}
                }
            }
        })
        .detach();
    });
}
