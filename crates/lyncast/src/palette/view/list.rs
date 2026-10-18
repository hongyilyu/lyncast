//! The list: each line as GPUI's `list()` asks for it, and the scroll that keeps the selected
//! row in view.

use std::sync::Arc;

use gpui::{
    AnyElement, Context, IntoElement, ListOffset, ParentElement, Pixels, RenderImage, Styled,
    Window, div, list, px, rgb,
};
use lyncast_core::{Row, Subject};

use super::PaletteView;
use super::draw::{clip_line, header_line, row_line};
use crate::palette::model::{Item, Line};
use crate::palette::{icons, theme};

impl PaletteView {
    pub(super) fn rows(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.model.lines().is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::MUTED))
                .child(self.model.empty_text())
                .into_any_element();
        }
        list(
            self.list.clone(),
            cx.processor(|view: &mut PaletteView, index, _, cx| view.line(index, cx)),
        )
        .flex_1()
        .px_2()
        .py(px(theme::LIST_PAD))
        .into_any_element()
    }

    fn line(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.model.lines()[index] {
            Line::Header(heading) => header_line(heading.label().into()),
            Line::Row(at) => {
                let selected = self.model.is_selected(at);
                match self.model.item(at) {
                    Item::Row(row) => row_line(row, self.icon(row, cx), selected),
                    Item::Clip(clip) => clip_line(clip, selected),
                }
            }
        }
    }

    /// Scrolls the selected row into view, and the header above it when the row is the first
    /// under one. Each line is as tall as the theme says, so the scroll needs no line that the
    /// list has measured, only the list's height. A list that has not been laid out has none,
    /// and `open_window` lays the window out before it returns, so the reveal waits for the end
    /// of the effect cycle then.
    pub(super) fn reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let height = self.list.viewport_bounds().size.height - px(2.0 * theme::LIST_PAD);
        if height <= px(0.0) {
            cx.defer_in(window, |view, window, cx| {
                if view.list.viewport_bounds().size.height > px(2.0 * theme::LIST_PAD) {
                    view.reveal(window, cx);
                    cx.notify();
                }
            });
            return;
        }
        let Some(line) = self.model.selected_line() else {
            return;
        };
        let lines = self.model.lines();
        let first = match line.checked_sub(1) {
            Some(above) if matches!(lines[above], Line::Header(_)) => above,
            _ => line,
        };
        let mut tops = vec![px(0.0)];
        for &line in lines {
            tops.push(tops[tops.len() - 1] + self.height(line));
        }
        let scrolled = self.list.logical_scroll_top();
        let shown = tops[scrolled.item_ix.min(lines.len())] + scrolled.offset_in_item;
        let (above, below) = (tops[first], tops[line + 1]);
        let goal = if above < shown {
            above
        } else if below > shown + height {
            below - height
        } else {
            return;
        };
        let item_ix = tops.partition_point(|&top| top <= goal) - 1;
        self.list.scroll_to(ListOffset {
            item_ix,
            offset_in_item: goal - tops[item_ix],
        });
    }

    fn height(&self, line: Line) -> Pixels {
        px(match line {
            Line::Header(_) => theme::HEADER,
            Line::Row(at) => match self.model.item(at) {
                Item::Row(row) if row.card.is_some() => theme::CARD,
                Item::Row(_) | Item::Clip(_) => theme::ROW,
            },
        })
    }

    /// The window draws again when the icons change, because GPUI redraws a window when an
    /// entity it read while drawing notifies.
    fn icon(&self, row: &Row, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let Subject::App { id, .. } = &row.subject else {
            return None;
        };
        let key = icons::key(self.daemon(cx)?.icon(id)?);
        self.icons.update(cx, |icons, cx| icons.get(&key, cx))
    }
}
