use gpui::{
    AnyElement, AppContext, Context, IntoElement, ParentElement, Render, Styled, WeakEntity,
    Window, div, px, rgb,
};
use lyncast_core::{Kind, Row, Section};

use super::host::Host;
use super::theme;
use super::trace::Trace;
use crate::daemon::Answer;
use crate::wire::Request;

pub struct PaletteView {
    rows: Vec<Row>,
    trace: Trace,
    seq: u32,
    traced_first: bool,
}

impl PaletteView {
    /// Asks the host for the root list of an empty query. A daemon that is quitting answers
    /// with a failure, and then the list is empty.
    pub fn new(
        _: &mut Window,
        cx: &mut Context<PaletteView>,
        host: &WeakEntity<Host>,
        trace: Trace,
        seq: u32,
    ) -> PaletteView {
        let request = Request::Query {
            text: String::new(),
        };
        let asked = host.update(cx, |host, cx| host.ask(request, cx));
        let rows = match asked.ok().flatten().map(|mut answer| answer.try_recv()) {
            Some(Ok(Some(Answer::Rows(rows)))) => rows,
            _ => Vec::new(),
        };
        PaletteView {
            rows,
            trace,
            seq,
            traced_first: false,
        }
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
}

impl Render for PaletteView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.trace_first_frame(window);
        let mut lines: Vec<AnyElement> = Vec::new();
        let mut section = None;
        for row in &self.rows {
            if section != Some(row.section) {
                section = Some(row.section);
                lines.extend(header(row.section).map(header_line));
            }
            lines.push(row_line(row));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .px_2()
            .py_1()
            .bg(rgb(theme::BACKGROUND))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT)
            .children(lines)
    }
}

/// A section's header. The Answer section, which holds only the calculator card, has none.
fn header(section: Section) -> Option<&'static str> {
    match section {
        Section::Answer => None,
        Section::Results => Some("Results"),
        Section::Suggestions => Some("Suggestions"),
        Section::Commands => Some("Commands"),
    }
}

/// The kind a row shows on its right. The calculator card and clip rows show none.
fn kind_label(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::App => Some("Application"),
        Kind::Command => Some("Command"),
        Kind::Clip | Kind::Calculator => None,
    }
}

fn header_line(name: &'static str) -> AnyElement {
    div()
        .h(px(theme::HEADER))
        .flex_none()
        .flex()
        .items_end()
        .px_2()
        .pb_1()
        .text_xs()
        .text_color(rgb(theme::MUTED))
        .child(name)
        .into_any_element()
}

fn row_line(row: &Row) -> AnyElement {
    let kind = kind_label(row.subject.key().kind());
    div()
        .h(px(theme::ROW))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px_2()
        .child(row.title.clone())
        .children(kind.map(|kind| div().text_sm().text_color(rgb(theme::MUTED)).child(kind)))
        .into_any_element()
}
