mod host;
#[cfg(test)]
mod tests;
mod theme;
mod trace;
mod view;

use std::cell::RefCell;
use std::env;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::{
    App, AppContext, AsyncApp, Bounds, QuitMode, WeakEntity, WindowBounds, WindowHandle,
    WindowKind, WindowOptions, WindowingModes, point, px, size,
};

use crate::daemon::{self, Bell, Owner, Rung, Wake};
use host::{Host, Next};
use trace::Trace;
use view::PaletteView;

const TRACE_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// Runs the daemon with the palette until a Quit or until the compositor goes. Without
/// `WAYLAND_DISPLAY` it fails before it takes the lock, because GPUI's Wayland-only platform
/// panics without a display server. The socket is bound before GPUI runs, and requests wait in
/// the inbox until the owner task takes them. With `trace`, the PROBE lines go to stdout.
pub fn run(record: Option<PathBuf>, trace: bool) -> Result<(), String> {
    if env::var_os("WAYLAND_DISPLAY").is_none_or(|display| display.is_empty()) {
        return Err(
            "not started: the palette needs a Wayland compositor; run lyncast daemon --headless without one"
                .to_owned(),
        );
    }
    let trace = if trace {
        Trace::start().map_err(|error| format!("not started: {error}"))?
    } else {
        Trace::OFF
    };
    let done = serve(record, trace);
    trace.flush(Instant::now() + TRACE_BEFORE_EXIT);
    done
}

fn serve(record: Option<PathBuf>, trace: Trace) -> Result<(), String> {
    let (bell, rung) = Bell::new();
    let owner = daemon::start(record, Some(bell))?;
    let outcome: Rc<RefCell<Option<Result<(), String>>>> = Rc::default();
    let leftover: Rc<RefCell<Option<Owner>>> = Rc::default();
    let (ended, kept) = (Rc::clone(&outcome), Rc::clone(&leftover));
    gpui_platform::linux(WindowingModes::WAYLAND)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            let host = cx.new(|_| Host::new(owner, trace));
            let weak = host.downgrade();
            let closed = weak.clone();
            cx.on_window_closed(move |cx, id| {
                let _ = closed.update(cx, |host, _| host.closed(id));
            })
            .detach();
            // GPUI runs this hook when its loop ends, also when the compositor goes. It holds
            // the host's one strong handle, so the host lives until then.
            cx.on_app_quit(move |cx| {
                *kept.borrow_mut() = host.update(cx, |host, _| host.take_owner());
                async {}
            })
            .detach();
            cx.spawn(async move |cx| {
                trace.event(0, "app_ready");
                let done = own(weak, rung, cx).await;
                *ended.borrow_mut() = Some(done);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    if let Some(done) = outcome.take() {
        return done;
    }
    // GPUI's loop ends before the stop has finished only when the compositor goes, so that is
    // the exit, even when the stop that runs here answers a Quit.
    if let Some(owner) = leftover.take() {
        owner.stop_blocking();
    }
    Err(daemon::WAYLAND_CLOSED.to_owned())
}

async fn own(host: WeakEntity<Host>, mut rung: Rung, cx: &mut AsyncApp) -> Result<(), String> {
    let gone = |_| "the palette's host is gone".to_owned();
    loop {
        match host.update(cx, |host, cx| host.drain(cx)).map_err(gone)? {
            Next::Serve => {
                if let Wake::BellsGone = rung.next().await {
                    return Err("the daemon's channel closed".to_owned());
                }
            }
            Next::Wait(wait) => {
                let timer = cx.background_executor().timer(wait);
                futures::future::select(Box::pin(rung.next()), timer).await;
            }
            Next::Done(done) => return done,
        }
    }
}

fn open_window(
    cx: &mut App,
    host: &WeakEntity<Host>,
    trace: Trace,
    seq: u32,
) -> gpui::Result<WindowHandle<PaletteView>> {
    cx.open_window(options(cx), |window, cx| {
        cx.new(|cx| PaletteView::new(window, cx, host, trace, seq))
    })
}

/// A layer-shell surface on the Overlay layer with the keyboard to itself, anchored to the top
/// edge, so the compositor centers it, and pushed down a third of the space it leaves on the
/// display GPUI lists first.
fn options(cx: &App) -> WindowOptions {
    let height = cx
        .displays()
        .first()
        .map(|display| f32::from(display.bounds().size.height));
    let top = height.map_or(0.0, theme::top_margin);
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.0), px(0.0)),
            size: size(px(theme::WIDTH), px(theme::HEIGHT)),
        })),
        focus: true,
        show: true,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "lyncast".to_owned(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP,
            margin: Some((px(top), px(0.0), px(0.0), px(0.0))),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        app_id: Some("lyncast".to_owned()),
        ..Default::default()
    }
}
