//! The palette's window. [`PaletteView`] holds the model and runs each effect the model asks
//! for against the host and the window. It draws the field, the footer, and the Actions menu,
//! and its modules draw the list and the clipboard screen's panes.

mod draw;
mod input;
#[cfg(test)]
pub(super) mod inspect;
mod list;
mod panes;

use futures::channel::oneshot;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, ElementInputHandler, FocusHandle,
    InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, ListAlignment, ListState,
    ModifiersChangedEvent, ParentElement, Render, Styled, Subscription, Task, TextAlign,
    WeakEntity, Window, canvas, div, fill, point, px, rgb, rgba, size,
};
use lyncast_core::ClipId;
use lyncast_core::clip::Delivery;

use super::clips::{self, ClipLine};
use super::field::Field;
use super::host::{Host, Shared};
use super::model::{self, Cap, Effect, Footer, Input, Landed, Landing, Menu, Model};
use super::preview;
use super::trace::Trace;
use super::{keys, theme};
use crate::clip::Content;
use crate::daemon::{Answer, Daemon, Screen};
use crate::wire::Request;
use draw::{field_runs, key_cap, scroll, shape};

/// The debug selector of the field that has the caret, by which the Tier 3 tests find its
/// bounds.
pub const FIELD: &str = "field";

enum Asked {
    Now(Answer),
    Later(oneshot::Receiver<Answer>),
}

pub struct PaletteView {
    model: Model,
    host: WeakEntity<Host>,
    shared: Shared,
    focus: FocusHandle,
    list: ListState,
    answer: Option<Task<()>>,
    revision: u64,
    trace: Trace,
    seq: u32,
    traced_first: bool,
    _activation: Subscription,
    _revision_watch: Option<Subscription>,
}

impl PaletteView {
    /// Takes the keyboard focus, so the field gets the window's text, and asks the host for
    /// the list of an empty field on `screen`. It tells the model whether the window is active
    /// now and at each change, and has the clipboard screen read its list again whenever the
    /// daemon takes an event that is not a read.
    pub fn new(
        window: &mut Window,
        cx: &mut Context<PaletteView>,
        host: &WeakEntity<Host>,
        shared: Shared,
        screen: Screen,
        trace: Trace,
        seq: u32,
    ) -> PaletteView {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let (mut model, list) = Model::new(screen);
        let opened = model.active(window.is_window_active());
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            let effect = view.model.active(window.is_window_active());
            view.run(effect, window, cx);
        });
        let upgraded = host.upgrade();
        let revision = upgraded.as_ref().map_or(0, |host| host.read(cx).revision());
        let revision_watch = upgraded.map(|host| {
            cx.observe_in(&host, window, |view, host, window, cx| {
                let revision = host.read(cx).revision();
                if revision != view.revision {
                    view.revision = revision;
                    if let Some(effect) = view.model.refresh() {
                        view.run(Some(effect), window, cx);
                    }
                }
            })
        });
        let mut view = PaletteView {
            model,
            host: host.clone(),
            shared,
            focus,
            list: ListState::new(0, ListAlignment::Top, px(theme::OVERDRAW)),
            answer: None,
            revision,
            trace,
            seq,
            traced_first: false,
            _activation: activation,
            _revision_watch: revision_watch,
        };
        view.run(Some(list), window, cx);
        view.run(opened, window, cx);
        view
    }

    /// Clipboard History from a client while the window is open.
    pub fn show_clipboard(&mut self, window: &mut Window, cx: &mut Context<PaletteView>) {
        let effect = self.model.land(Landed::Show, window.modifiers().modified());
        self.run(effect, window, cx);
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = keys::press(event) else {
            return;
        };
        cx.stop_propagation();
        let effect = self.model.input(Input::Press(press));
        self.run(effect, window, cx);
    }

    fn key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        let mods = window.modifiers().modified();
        let effect = self.model.released(&event.keystroke.key, mods);
        self.run(effect, window, cx);
    }

    fn modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let effect = self.model.modifiers(event.modifiers.modified());
        self.run(effect, window, cx);
    }

    fn run(&mut self, effect: Option<Effect>, window: &mut Window, cx: &mut Context<Self>) {
        let mods = window.modifiers().modified();
        let mut next = effect;
        while let Some(effect) = next {
            next = match effect {
                Effect::Query(text) => match self.ask(Request::Query { text }, cx) {
                    Some(Asked::Now(Answer::Rows(rows))) => {
                        let next = self.model.land(Landed::Rows(rows), mods);
                        self.reset_list(Landing::Fresh);
                        next
                    }
                    Some(Asked::Now(Answer::Failed(_))) | None => Some(Effect::Close),
                    Some(Asked::Now(_) | Asked::Later(_)) => {
                        unreachable!("the daemon answers a query with its rows at once")
                    }
                },
                Effect::Clips {
                    filter,
                    kinds,
                    landing,
                } => match self.clip_lines(&filter, kinds, cx) {
                    Some(lines) => {
                        let next = self.model.land(Landed::Clips(lines, landing), mods);
                        self.reset_list(landing);
                        next
                    }
                    None => Some(Effect::Close),
                },
                Effect::Paste => {
                    let text = self.paste_text(cx);
                    self.model.land(Landed::Paste(text), mods)
                }
                Effect::Ask(request) => match self.ask(request, cx) {
                    Some(Asked::Now(answer)) => self.model.land(Landed::Answer(answer), mods),
                    Some(Asked::Later(answer)) => {
                        self.wait(answer, window, cx);
                        None
                    }
                    None => Some(Effect::Close),
                },
                Effect::Close => {
                    window.remove_window();
                    None
                }
            };
        }
        self.sync_preview(window, cx);
        self.reveal(window, cx);
        cx.notify();
    }

    fn reset_list(&self, landing: Landing) {
        let top = self.list.logical_scroll_top();
        let count = self.model.lines().len();
        self.list.reset_with_uniform_height(count, px(theme::ROW));
        if landing == Landing::Refresh {
            self.list.scroll_to(top);
        }
    }

    fn clip_lines(&self, filter: &str, kinds: clips::Kinds, cx: &App) -> Option<Vec<ClipLine>> {
        let host = self.host.upgrade()?;
        let host = host.read(cx);
        let (zone, now) = host.wall()?;
        let rows = host.daemon()?.clips().rows();
        Some(clips::clip_lines(
            &rows,
            filter,
            kinds,
            |at| zone.date(at),
            zone.date(now),
        ))
    }

    fn daemon<'a>(&self, cx: &'a App) -> Option<&'a Daemon> {
        self.host.upgrade()?.read(cx).daemon()
    }

    fn paste_text(&self, cx: &App) -> Option<String> {
        let current = self.daemon(cx)?.clips().current()?;
        match current.content {
            Content::Text(_) | Content::Files => current.preview.clone(),
            Content::Image => None,
        }
    }

    fn sync_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.model.selected_clip().map(|clip| clip.id);
        let preview = self.shared.preview.clone();
        if preview.read(cx).of() == selected {
            return;
        }
        let what = selected.and_then(|id| {
            let clips = self.daemon(cx)?.clips();
            let row = clips.get(id)?;
            Some(preview::what(row, || clips.payload(&Delivery::Clip(id))))
        });
        preview.update(cx, |preview, cx| preview.select(selected, what, window, cx));
    }

    fn clip_preview(&self, id: ClipId, cx: &App) -> Option<String> {
        self.daemon(cx)?.clips().get(id)?.preview.clone()
    }

    fn ask(&mut self, request: Request, cx: &mut Context<Self>) -> Option<Asked> {
        let asked = self.host.update(cx, |host, cx| host.ask(request, cx));
        let mut answer = asked.ok().flatten()?;
        match answer.try_recv() {
            Ok(Some(now)) => Some(Asked::Now(now)),
            Ok(None) => Some(Asked::Later(answer)),
            Err(oneshot::Canceled) => None,
        }
    }

    fn wait(&mut self, answer: oneshot::Receiver<Answer>, window: &Window, cx: &mut Context<Self>) {
        self.answer = Some(cx.spawn_in(window, async move |view, cx| {
            let answer = answer
                .await
                .unwrap_or_else(|_| Answer::Failed("the daemon gave no answer".to_owned()));
            let _ = view.update_in(cx, |view, window, cx| {
                let mods = window.modifiers().modified();
                let effect = view.model.land(Landed::Answer(answer), mods);
                view.run(effect, window, cx);
            });
        }));
    }

    /// `open_window` lays out and paints before it returns, but presents later: the next frame
    /// callback runs just before GPUI draws and presents, and a task spawned there runs once the
    /// present has returned.
    fn trace_first_frame(&mut self, window: &mut Window) {
        if self.traced_first {
            return;
        }
        self.traced_first = true;
        let (trace, seq) = (self.trace, self.seq);
        trace.event(seq, "first_render");
        window.on_next_frame(move |window, cx| {
            trace.event(seq, "frame1_pre_present");
            let handle = window.window_handle();
            cx.spawn(async move |cx| {
                trace.event(seq, "frame1_presented");
                let _ = cx.update_window(handle, move |_, window, _| {
                    window.on_next_frame(move |_, _| trace.event(seq, "frame1_done"));
                });
            })
            .detach();
        });
    }

    fn field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.model.menu().is_none();
        let kinds = self.model.kinds().map(|kinds| {
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .child(kinds.label())
                .child(key_cap("Ctrl T"))
        });
        div()
            .h(px(theme::FIELD))
            .flex_none()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(div().flex_1().min_w_0().child(self.field_line(
                self.model.field(),
                self.model.placeholder(),
                focused,
                cx,
            )))
            .children(kinds)
    }

    fn field_line(
        &self,
        field: &Field,
        placeholder: &'static str,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (view, focus) = (cx.entity(), self.focus.clone());
        let (text, runs) = field_runs(field, placeholder);
        let cursor = field.cursor();
        let line = canvas(
            move |_, window, _| shape(text, &runs, window),
            move |bounds, line, window, cx| {
                if focused {
                    window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx);
                }
                let height = bounds.size.height;
                let left = bounds.left() - scroll(&line, cursor, bounds.size.width);
                let origin = point(left, bounds.top());
                let _ = line.paint(origin, height, TextAlign::Left, None, window, cx);
                if focused {
                    let caret = point(left + line.x_for_index(cursor), bounds.top());
                    let caret = Bounds::new(caret, size(px(theme::CARET_WIDTH), height));
                    window.paint_quad(fill(caret, rgb(theme::CARET)));
                }
            },
        );
        div()
            .w_full()
            .h(px(theme::FIELD_LINE))
            .overflow_hidden()
            .when(focused, |clip| clip.debug_selector(|| FIELD.to_owned()))
            .child(line.size_full())
    }

    fn menu(&self, menu: &Menu, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<AnyElement> = menu
            .shown()
            .enumerate()
            .map(|(at, (chord, item))| {
                let line = div()
                    .h(px(theme::MENU_ROW))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .rounded(px(6.0))
                    .child(item.label().to_owned())
                    .children(chord.map(|chord| key_cap(Cap::of(chord).label())));
                line.when(menu.is_selected(at), |line| line.bg(rgba(theme::SELECTED)))
                    .into_any_element()
            })
            .collect();
        let empty = items.is_empty().then(|| {
            div()
                .h(px(theme::MENU_ROW))
                .flex()
                .items_center()
                .px_2()
                .text_color(rgb(theme::MUTED))
                .child(model::NO_ACTIONS)
        });
        div()
            .absolute()
            .right(px(theme::MENU_INSET))
            .bottom(px(theme::FOOTER + theme::MENU_INSET))
            .w(px(theme::MENU_WIDTH))
            .flex()
            .flex_col()
            .p_1()
            .rounded(px(8.0))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::MENU))
            .children(items)
            .children(empty)
            .child(
                div()
                    .mt_1()
                    .pt_2()
                    .px_2()
                    .border_t_1()
                    .border_color(rgb(theme::BORDER))
                    .child(self.field_line(menu.field(), model::MENU_PLACEHOLDER, true, cx)),
            )
    }

    fn footer(&self) -> impl IntoElement {
        let bar = div()
            .h(px(theme::FOOTER))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .px_4()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_sm();
        match self.model.footer() {
            Some(Footer::Failed(reason)) => bar.child(
                div()
                    .flex_1()
                    .text_color(rgb(theme::FAILED))
                    .child(reason.to_owned()),
            ),
            Some(footer) => {
                let bar = match &footer {
                    Footer::Confirm(title) => {
                        bar.child(div().flex_1().child(model::question(title)))
                    }
                    _ => bar,
                };
                let hints = footer.hints().into_iter().enumerate();
                bar.children(hints.flat_map(|(at, (label, cap))| {
                    let label = div().child(label.to_owned());
                    let label = if at == 0 {
                        label
                    } else {
                        label.text_color(rgb(theme::MUTED))
                    };
                    [
                        label.into_any_element(),
                        key_cap(cap.label()).into_any_element(),
                    ]
                }))
            }
            None => bar,
        }
    }
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.trace_first_frame(window);
        div()
            .key_context("Palette")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(Self::modifiers_changed))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgba(theme::BACKGROUND))
            .rounded(px(theme::RADIUS))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT)
            .relative()
            .child(self.field(cx))
            .child(match self.model.kinds() {
                Some(_) => self.clipboard(cx),
                None => self.rows(cx),
            })
            .child(self.footer())
            .children(self.model.menu().map(|menu| self.menu(menu, cx)))
    }
}
