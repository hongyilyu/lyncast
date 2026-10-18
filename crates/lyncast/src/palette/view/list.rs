//! The list, which draws a line as GPUI's `list()` asks for it.

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, list, rgb};

use super::PaletteView;
use super::draw::{header_line, row_line};
use crate::palette::model::{self, Line};
use crate::palette::theme;

impl PaletteView {
    pub(super) fn rows(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.model.lines().is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::MUTED))
                .child(model::NO_RESULTS)
                .into_any_element();
        }
        list(
            self.list.clone(),
            cx.processor(|view: &mut PaletteView, index, _, _| view.line(index)),
        )
        .flex_1()
        .px_2()
        .py_1()
        .into_any_element()
    }

    fn line(&self, index: usize) -> AnyElement {
        match self.model.lines()[index] {
            Line::Header(name) => header_line(name),
            Line::Row(row) => row_line(self.model.row(row), self.model.is_selected(row)),
        }
    }
}
