//! Nothing inside a `Host` update opens, closes, or updates a window. [`WindowFace`] defers every
//! such change to the end of the effect cycle, when no entity is leased.

use std::mem;
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, Context, Entity, WeakEntity, WindowHandle, WindowId};
use lyncast_core::AppId;

use super::icons::Icons;
use super::trace::Trace;
use super::view::PaletteView;
use crate::daemon::{Answer, Face, IsOpen, Owner, Wait};
use crate::warning::warn;
use crate::wire::{Request, Summon};

enum Shown {
    Closed,
    /// The deferred open of this summon is on its way.
    Opening(u32),
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
    icons: Entity<Icons>,
}

struct Summons {
    shown: Shown,
    newest: u32,
    trace: Trace,
}

impl Host {
    pub fn new(owner: Owner, icons: Entity<Icons>, trace: Trace) -> Host {
        Host {
            owner: Some(owner),
            summons: Summons {
                shown: Shown::Closed,
                newest: 0,
                trace,
            },
            windows: Vec::new(),
            icons,
        }
    }

    pub fn icons(&self) -> Entity<Icons> {
        self.icons.clone()
    }

    /// The `Icon=` value of the app with this ID. `None` too once the stop has finished.
    pub fn icon(&self, id: &AppId) -> Option<&str> {
        self.owner.as_ref()?.daemon().icon(id)
    }

    /// How the palette sends the daemon a request, which may change it. The answer is in the
    /// receiver at once unless the request starts a job, and then it comes when the job ends.
    /// `None` once the stop has finished. The root list's query comes here too, as a `query`
    /// request, so [`Daemon::handle`](crate::daemon::Daemon::handle) gives the palette the rows
    /// it gives a socket client. Any other read that changes nothing, such as [`Host::icon`],
    /// goes to the daemon with no request.
    pub fn ask(
        &mut self,
        request: Request,
        cx: &mut Context<Host>,
    ) -> Option<oneshot::Receiver<Answer>> {
        let owner = self.owner.as_mut()?;
        let mut face = WindowFace {
            summons: &mut self.summons,
            cx,
        };
        Some(owner.ask(&mut face, request))
    }

    /// Hands the daemon every queued event, and finishes the stop once its wait is over.
    pub fn drain(&mut self, cx: &mut Context<Host>) -> Next {
        let Some(owner) = self.owner.as_mut() else {
            return Next::Done(Err("the palette's host has no daemon".to_owned()));
        };
        let mut face = WindowFace {
            summons: &mut self.summons,
            cx,
        };
        match owner.take_queued(&mut face) {
            Wait::Forever => Next::Serve,
            Wait::For(wait) => Next::Wait(wait),
            Wait::Over(over) => {
                let owner = self.owner.take().expect("the host holds the owner");
                Next::Done(owner.finish_blocking(over, &mut face))
            }
        }
    }

    /// `on_window_closed`: a window that leaves `windows` traces `window_closed`, and the open
    /// summon's window closes the summon too, as a close by the compositor does.
    pub fn closed(&mut self, id: WindowId) {
        if let Some(at) = self.windows.iter().position(|&(window, _)| window == id) {
            let (_, seq) = self.windows.remove(at);
            self.summons.trace.event(seq, "window_closed");
        }
        if matches!(&self.summons.shown, Shown::Open(handle) if handle.window_id() == id) {
            self.summons.shown = Shown::Closed;
        }
    }

    /// The open summon's window, once its deferred open has run.
    #[cfg(test)]
    pub fn window(&self) -> Option<WindowHandle<PaletteView>> {
        match self.summons.shown {
            Shown::Open(handle) => Some(handle),
            Shown::Closed | Shown::Opening(_) => None,
        }
    }

    /// No job is out, so every answer the palette waits for is in its receiver.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        self.owner
            .as_ref()
            .is_none_or(|owner| owner.daemon().jobs() == 0)
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
        match self.summons.shown {
            Shown::Closed => Summon::Closed,
            Shown::Opening(_) | Shown::Open(_) => Summon::Open,
        }
    }

    fn open(&mut self) {
        self.summons.newest += 1;
        let (seq, trace, host) = (
            self.summons.newest,
            self.summons.trace,
            self.cx.weak_entity(),
        );
        trace.event(seq, "toggle_handled_open");
        self.summons.shown = Shown::Opening(seq);
        self.cx.defer(move |cx| open_summon(cx, &host, trace, seq));
    }

    fn close(&mut self, _: IsOpen) {
        if let Shown::Open(handle) = mem::replace(&mut self.summons.shown, Shown::Closed) {
            self.cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }
}

fn open_summon(cx: &mut App, host: &WeakEntity<Host>, trace: Trace, seq: u32) {
    if !host
        .read_with(
            cx,
            |host, _| matches!(host.summons.shown, Shown::Opening(opening) if opening == seq),
        )
        .unwrap_or(false)
    {
        return;
    }
    match super::open_window(cx, host, trace, seq) {
        Ok(handle) => {
            trace.event(seq, "window_opened");
            let _ = host.update(cx, |host, _| {
                host.summons.shown = Shown::Open(handle);
                host.windows.push((handle.window_id(), seq));
            });
        }
        Err(error) => {
            warn(format_args!("the palette did not open: {error}"));
            let _ = host.update(cx, |host, _| host.summons.shown = Shown::Closed);
        }
    }
}
