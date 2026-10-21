//! Nothing inside a `Host` update opens, closes, or updates a window. [`WindowFace`] defers every
//! such change to the end of the effect cycle, when no entity is leased.

use std::mem;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, Context, Entity, WeakEntity, WindowHandle, WindowId};
use lyncast_core::rank::Timestamp;

use super::icons::Icons;
use super::preview::Preview;
use super::trace::Trace;
use super::view::PaletteView;
use super::zone::Zone;
use crate::daemon::{Answer, Daemon, Face, IsOpen, Owner, Screen, Wait};
use crate::niri;
use crate::warning::warn;
use crate::wire::{Request, Summon};

enum WindowState {
    Closed,
    Opening { seq: u32, screen: Screen },
    Open(WindowHandle<PaletteView>),
}

/// What the owner task does after a drain.
pub enum Next {
    /// Wait for the bell.
    Serve,
    /// The stop has begun. Wait for the bell, or this long at most.
    Wait(Duration),
    /// The stop has finished.
    Done(Result<(), String>),
}

pub struct Host {
    owner: Option<Owner>,
    summons: Summons,
    windows: Vec<(WindowId, u32)>,
    shared: Shared,
    zone: Zone,
}

/// The entities that the host holds for every window, which each window gets as it opens.
#[derive(Clone)]
pub struct Shared {
    pub icons: Entity<Icons>,
    pub preview: Entity<Preview>,
}

struct Summons {
    window: WindowState,
    newest: u32,
    trace: Trace,
    /// The niri that each summon asks for its output's height.
    niri: Option<PathBuf>,
}

impl Host {
    pub fn new(owner: Owner, shared: Shared, zone: Zone, trace: Trace) -> Host {
        let niri = owner.daemon().vars().niri_socket.clone();
        Host {
            owner: Some(owner),
            summons: Summons {
                window: WindowState::Closed,
                newest: 0,
                trace,
                niri,
            },
            windows: Vec::new(),
            shared,
            zone,
        }
    }

    /// The daemon, for reads that change nothing, such as the clipboard history. `None` once
    /// the stop has finished.
    pub fn daemon(&self) -> Option<&Daemon> {
        self.owner.as_ref().map(Owner::daemon)
    }

    /// How many events that are not reads the daemon has taken. The host notifies when it
    /// moves.
    pub fn revision(&self) -> u64 {
        self.owner.as_ref().map_or(0, Owner::changes)
    }

    /// The zone of the wall clock and the daemon's time now, by the clock the owner reads.
    /// `None` once the stop has finished.
    pub fn wall(&self) -> Option<(Zone, Timestamp)> {
        Some((self.zone, self.owner.as_ref()?.now()))
    }

    pub fn shared(&self) -> Shared {
        self.shared.clone()
    }

    /// How the palette sends the daemon a request, which may change it. The answer is in the
    /// receiver at once unless the request starts a job, and then it comes when the job ends.
    /// `None` once the stop has finished. The root list's query comes here too, as a `query`
    /// request, so [`Daemon::handle`] gives the palette the rows it gives a socket client. Any
    /// other read that changes nothing goes through [`Host::daemon`] with no request.
    pub fn ask(
        &mut self,
        request: Request,
        cx: &mut Context<Host>,
    ) -> Option<oneshot::Receiver<Answer>> {
        self.with_owner(cx, |owner, face| owner.ask(face, request))
    }

    /// Hands the daemon every queued event, and finishes the stop once its wait is over.
    pub fn drain(&mut self, cx: &mut Context<Host>) -> Next {
        let Some(wait) = self.with_owner(cx, |owner, face| owner.take_queued(face)) else {
            return Next::Done(Err("the palette's host has no daemon".to_owned()));
        };
        match wait {
            Wait::Forever => Next::Serve,
            Wait::For(wait) => Next::Wait(wait),
            Wait::Over(over) => {
                let owner = self.owner.take().expect("the host holds the owner");
                let mut face = WindowFace {
                    summons: &mut self.summons,
                    cx,
                };
                Next::Done(owner.finish_blocking(over, &mut face))
            }
        }
    }

    fn with_owner<R>(
        &mut self,
        cx: &mut Context<Host>,
        hand: impl FnOnce(&mut Owner, &mut dyn Face) -> R,
    ) -> Option<R> {
        let owner = self.owner.as_mut()?;
        let revision = owner.changes();
        let handed = hand(
            owner,
            &mut WindowFace {
                summons: &mut self.summons,
                cx,
            },
        );
        if owner.changes() != revision {
            cx.notify();
        }
        Some(handed)
    }

    /// `on_window_closed`: a window that leaves `windows` traces `window_closed`, and the open
    /// summon's window closes the summon too, as a close by the compositor does.
    pub fn closed(&mut self, id: WindowId) {
        if let Some(at) = self.windows.iter().position(|&(window, _)| window == id) {
            let (_, seq) = self.windows.remove(at);
            self.summons.trace.event(seq, "window_closed");
        }
        if matches!(&self.summons.window, WindowState::Open(handle) if handle.window_id() == id) {
            self.summons.window = WindowState::Closed;
        }
    }

    /// The open summon's window, once its deferred open has run.
    #[cfg(test)]
    pub fn window(&self) -> Option<WindowHandle<PaletteView>> {
        match self.summons.window {
            WindowState::Open(handle) => Some(handle),
            WindowState::Closed | WindowState::Opening { .. } => None,
        }
    }

    /// No job is out, so every answer the palette waits for is in its receiver.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.daemon().is_none_or(|daemon| daemon.jobs() == 0)
    }

    /// The quit hook's: the owner, for a stop that runs once GPUI's loop has ended.
    pub fn take_owner(&mut self) -> Option<Owner> {
        self.owner.take()
    }
}

struct WindowFace<'a, 'c> {
    summons: &'a mut Summons,
    cx: &'a mut Context<'c, Host>,
}

impl Face for WindowFace<'_, '_> {
    fn summon(&self) -> Summon {
        match self.summons.window {
            WindowState::Closed => Summon::Closed,
            WindowState::Opening { .. } | WindowState::Open(_) => Summon::Open,
        }
    }

    fn open(&mut self, screen: Screen) {
        self.summons.newest += 1;
        let (seq, trace, host) = (
            self.summons.newest,
            self.summons.trace,
            self.cx.weak_entity(),
        );
        trace.event(seq, "toggle_handled_open");
        self.summons.window = WindowState::Opening { seq, screen };
        let Some(socket) = self.summons.niri.clone() else {
            self.cx
                .defer(move |cx| open_summon(cx, &host, trace, seq, None));
            return;
        };
        // niri answers on its event loop, so the question waits on a thread of its own, never on
        // GPUI's main thread. A thread that does not start drops the sender, and the window opens
        // with no height.
        let (answer_to, answer) = oneshot::channel();
        let _ = thread::Builder::new().spawn(move || {
            let _ = answer_to.send(niri::focused_height(Some(&socket)));
        });
        self.cx
            .spawn(async move |_, cx| {
                let height = answer.await.ok().flatten();
                cx.update(|cx| cx.defer(move |cx| open_summon(cx, &host, trace, seq, height)));
            })
            .detach();
    }

    fn close(&mut self, _: IsOpen) {
        if let WindowState::Open(handle) =
            mem::replace(&mut self.summons.window, WindowState::Closed)
        {
            self.cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    fn show_clipboard(&mut self) {
        match &mut self.summons.window {
            WindowState::Opening { screen, .. } => *screen = Screen::Clipboard,
            WindowState::Open(handle) => {
                let handle = *handle;
                self.cx.defer(move |cx| {
                    let _ = handle.update(cx, |view, window, cx| view.show_clipboard(window, cx));
                });
            }
            WindowState::Closed => {}
        }
    }
}

/// Opens summon `seq`'s window, unless another summon or a close has come since, `height` the
/// logical height of the output that niri says has the focus.
fn open_summon(cx: &mut App, host: &WeakEntity<Host>, trace: Trace, seq: u32, height: Option<u32>) {
    let opening = host.read_with(cx, |host, _| match host.summons.window {
        WindowState::Opening {
            seq: opening,
            screen,
        } if opening == seq => Some(screen),
        _ => None,
    });
    let Ok(Some(screen)) = opening else {
        return;
    };
    match super::open_window(cx, host, screen, trace, seq, height) {
        Ok(handle) => {
            trace.event(seq, "window_opened");
            let _ = host.update(cx, |host, _| {
                host.summons.window = WindowState::Open(handle);
                host.windows.push((handle.window_id(), seq));
            });
        }
        Err(error) => {
            warn(format_args!("the palette did not open: {error}"));
            let _ = host.update(cx, |host, _| host.summons.window = WindowState::Closed);
        }
    }
}
