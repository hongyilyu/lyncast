//! The palette's window. [`PaletteView`] holds the model, runs each effect the model asks for
//! against the host and the window, and draws the field, the list, and the footer.

mod draw;
mod input;
#[cfg(test)]
pub(super) mod inspect;
mod list;

use gpui::{
    AppContext, Bounds, Context, ElementInputHandler, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, ParentElement, Render, Styled, TextAlign, WeakEntity,
    Window, canvas, div, fill, point, px, rgb, size,
};

use super::host::Host;
use super::model::{Effect, Input, Landed, Line, Model};
use super::trace::Trace;
use super::{keys, theme};
use crate::daemon::Answer;
use crate::wire::Request;
use draw::{field_runs, scroll, shape};

/// The debug selector of the field that has the caret, by which the Tier 3 tests find its
/// bounds.
pub const FIELD: &str = "field";

pub struct PaletteView {
    model: Model,
    host: WeakEntity<Host>,
    focus: FocusHandle,
    list: ListState,
    trace: Trace,
    seq: u32,
    traced_first: bool,
}

impl PaletteView {
    /// Takes the keyboard focus, so the field gets the window's text, and asks the host for
    /// the root list of an empty field.
    pub fn new(
        window: &mut Window,
        cx: &mut Context<PaletteView>,
        host: &WeakEntity<Host>,
        trace: Trace,
        seq: u32,
    ) -> PaletteView {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let (model, effect) = Model::new();
        let mut view = PaletteView {
            model,
            host: host.clone(),
            focus,
            list: ListState::new(0, ListAlignment::Top, px(theme::OVERDRAW)),
            trace,
            seq,
            traced_first: false,
        };
        view.run(Some(effect), window, cx);
        view
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = keys::press(event) else {
            return;
        };
        cx.stop_propagation();
        let effect = self.model.input(Input::Press(press));
        self.run(effect, window, cx);
    }

    fn run(&mut self, effect: Option<Effect>, window: &mut Window, cx: &mut Context<Self>) {
        let mut next = effect;
        while let Some(effect) = next {
            next = match effect {
                Effect::Query(text) => match self.ask(Request::Query { text }, cx) {
                    Some(Answer::Rows(rows)) => {
                        let next = self.model.land(Landed::Rows(rows));
                        let count = self.model.lines().len();
                        self.list.reset_with_uniform_height(count, px(theme::ROW));
                        next
                    }
                    Some(Answer::Failed(_)) | None => Some(Effect::Close),
                    Some(_) => unreachable!("the daemon answers a query with its rows"),
                },
                Effect::Close => {
                    window.remove_window();
                    None
                }
            };
        }
        self.reveal();
        cx.notify();
    }

    fn ask(&mut self, request: Request, cx: &mut Context<Self>) -> Option<Answer> {
        let asked = self.host.update(cx, |host, cx| host.ask(request, cx));
        asked.ok().flatten()?.try_recv().ok().flatten()
    }

    /// A list that has not been laid out has no height to scroll in, and it shows its top, where
    /// the first list's selection is.
    fn reveal(&self) {
        let Some(line) = self.model.selected_line() else {
            return;
        };
        if self.list.viewport_bounds().size.height <= px(0.0) {
            return;
        }
        if line > 0 && matches!(self.model.lines()[line - 1], Line::Header(_)) {
            self.list.scroll_to_reveal_item(line - 1);
        }
        self.list.scroll_to_reveal_item(line);
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
        let (view, focus) = (cx.entity(), self.focus.clone());
        let (text, runs) = field_runs(&self.model);
        let cursor = self.model.field().cursor();
        let line = canvas(
            move |_, window, _| shape(text, &runs, window),
            move |bounds, line, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx);
                let height = bounds.size.height;
                let left = bounds.left() - scroll(&line, cursor, bounds.size.width);
                let origin = point(left, bounds.top());
                let _ = line.paint(origin, height, TextAlign::Left, None, window, cx);
                let caret = point(left + line.x_for_index(cursor), bounds.top());
                window.paint_quad(fill(
                    Bounds::new(caret, size(px(theme::CARET_WIDTH), height)),
                    rgb(theme::CARET),
                ));
            },
        );
        div()
            .h(px(theme::FIELD))
            .flex_none()
            .flex()
            .items_center()
            .px_4()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(
                div()
                    .w_full()
                    .h(px(theme::FIELD_LINE))
                    .overflow_hidden()
                    .debug_selector(|| FIELD.to_owned())
                    .child(line.size_full()),
            )
    }

    fn footer(&self) -> impl IntoElement {
        div()
            .h(px(theme::FOOTER))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .px_4()
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .children(self.model.footer())
    }
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.trace_first_frame(window);
        div()
            .key_context("Palette")
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::key_down))
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT)
            .child(self.field(cx))
            .child(self.rows(cx))
            .child(self.footer())
    }
}
