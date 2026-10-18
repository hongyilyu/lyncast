//! The list, which draws a line as GPUI's `list()` asks for it.

use std::sync::Arc;

use gpui::{AnyElement, Context, IntoElement, ParentElement, RenderImage, Styled, div, list, rgb};
use lyncast_core::{Row, Subject};

use super::PaletteView;
use super::draw::{header_line, row_line};
use crate::palette::model::{self, Line};
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
                .child(model::NO_RESULTS)
                .into_any_element();
        }
        list(
            self.list.clone(),
            cx.processor(|view: &mut PaletteView, index, _, cx| view.line(index, cx)),
        )
        .flex_1()
        .px_2()
        .py_1()
        .into_any_element()
    }

    fn line(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        match self.model.lines()[index] {
            Line::Header(name) => header_line(name),
            Line::Row(at) => {
                let row = self.model.row(at);
                row_line(row, self.icon(row, cx), self.model.is_selected(at))
            }
        }
    }

    /// The window draws again when the icons change, because GPUI redraws a window when an
    /// entity it read while drawing notifies.
    fn icon(&self, row: &Row, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        let Subject::App { id, .. } = &row.subject else {
            return None;
        };
        let host = self.host.upgrade()?;
        let key = icons::key(host.read(cx).icon(id)?);
        self.icons.update(cx, |icons, cx| icons.get(&key, cx))
    }
}
