mod clips;
mod field;
mod host;
mod icons;
mod keys;
mod model;
mod pixels;
mod preview;
#[cfg(test)]
mod tests;
mod theme;
mod trace;
mod view;
mod zone;

use std::cell::RefCell;
use std::env;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::{
    App, AppContext, AsyncApp, Bounds, Entity, QuitMode, WeakEntity, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, WindowingModes, point, px, size,
};

use crate::daemon::{self, Bell, Owner, Rung, Screen, Wake};
use host::{Host, Next, Shared};
use icons::{IconRoots, Icons};
use preview::Preview;
use trace::Trace;
use view::PaletteView;
use zone::Zone;

const TRACE_BEFORE_EXIT: Duration = Duration::from_secs(1);

/// The window's layer-shell namespace, which the layer rule of `dist/niri/lyncast.kdl` matches.
pub const NAMESPACE: &str = "lyncast";

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
    let roots = IconRoots::from_vars(owner.daemon().vars());
    let zone = Zone::load();
    let exit = Exit::default();
    let mounted = Mount {
        owner,
        roots,
        zone,
        rung,
        trace,
        exit: exit.clone(),
    };
    gpui_platform::linux(WindowingModes::WAYLAND)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            mount(cx, mounted);
        });
    if let Some(done) = exit.outcome.take() {
        return done;
    }
    // GPUI's loop ends before the stop has finished only when the compositor goes, so that is
    // the exit, even when the stop that runs here answers a Quit.
    if let Some(owner) = exit.leftover.take() {
        owner.stop_blocking();
    }
    Err(daemon::WAYLAND_CLOSED.to_owned())
}

/// What the palette host starts with.
struct Mount {
    owner: Owner,
    roots: IconRoots,
    zone: Zone,
    rung: Rung,
    trace: Trace,
    exit: Exit,
}

/// Where the palette host leaves how it ended: the owner task's outcome once the stop has
/// finished, or the owner that GPUI's quit hook took before it had.
#[derive(Clone, Default)]
struct Exit {
    outcome: Rc<RefCell<Option<Result<(), String>>>>,
    leftover: Rc<RefCell<Option<Owner>>>,
}

/// Starts the palette host in `cx`: the host with the icons from its roots and the preview, the
/// observer of closed windows, the quit hook, and the owner task.
fn mount(cx: &mut App, mounted: Mount) -> Entity<Host> {
    let Mount {
        owner,
        roots,
        zone,
        rung,
        trace,
        exit,
    } = mounted;
    let shared = Shared {
        icons: cx.new(|_| Icons::new(roots)),
        preview: cx.new(|_| Preview::default()),
    };
    let host = cx.new(|_| Host::new(owner, shared, zone, trace));
    let weak = host.downgrade();
    let closed = weak.clone();
    cx.on_window_closed(move |cx, id| {
        let _ = closed.update(cx, |host, _| host.closed(id));
    })
    .detach();
    // GPUI runs this hook when its loop ends, also when the compositor goes. It holds a strong
    // handle, so the host lives until then.
    let held = host.clone();
    let leftover = exit.leftover;
    cx.on_app_quit(move |cx| {
        *leftover.borrow_mut() = held.update(cx, |host, _| host.take_owner());
        async {}
    })
    .detach();
    let outcome = exit.outcome;
    cx.spawn(async move |cx| {
        trace.event(0, "app_ready");
        let done = own(weak, rung, cx).await;
        *outcome.borrow_mut() = Some(done);
        cx.update(|cx| cx.quit());
    })
    .detach();
    host
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
    screen: Screen,
    trace: Trace,
    seq: u32,
    height: Option<u32>,
) -> gpui::Result<WindowHandle<PaletteView>> {
    let shared = host.read_with(cx, |host, _| host.shared())?;
    cx.open_window(options(cx, height), |window, cx| {
        cx.new(|cx| PaletteView::new(window, cx, host, shared, screen, trace, seq))
    })
}

/// A layer-shell surface on the Overlay layer with the keyboard to itself, anchored to the top
/// edge, so the compositor centers it on the output it picks, and pushed down a third of the
/// space it leaves on an output `height` tall, or else on the display GPUI lists first.
fn options(cx: &App, height: Option<u32>) -> WindowOptions {
    let height = height.map(|height| height as f32).or_else(|| {
        cx.displays()
            .first()
            .map(|display| f32::from(display.bounds().size.height))
    });
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
            namespace: NAMESPACE.to_owned(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP,
            margin: Some((px(top), px(0.0), px(0.0), px(0.0))),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        app_id: Some("lyncast".to_owned()),
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}
