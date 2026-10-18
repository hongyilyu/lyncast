//! The window's input handler, in UTF-16 as GPUI gives it, over the field's UTF-8 offsets.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, UTF16Selection, Window, point};

use super::PaletteView;
use super::draw::{field_runs, scroll, shape};
use crate::palette::model::Input;

impl EntityInputHandler for PaletteView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let field = self.model.field();
        let (start, end) = (field.byte_offset(range.start), field.byte_offset(range.end));
        let (start, end) = (start.min(end), start.max(end));
        *adjusted = Some(field.utf16_offset(start)..field.utf16_offset(end));
        Some(field.text()[start..end].to_owned())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let field = self.model.field();
        let cursor = field.utf16_offset(field.cursor());
        Some(UTF16Selection {
            range: cursor..cursor,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let field = self.model.field();
        let marked = field.marked()?;
        Some(field.utf16_offset(marked.start)..field.utf16_offset(marked.end))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let effect = self.model.input(Input::Unmark);
        self.run(effect, window, cx);
    }

    fn replace_text_in_range(
        &mut self,
        utf16: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.to_owned();
        let effect = self.model.input(Input::Replace { utf16, text });
        self.run(effect, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        utf16: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.to_owned();
        let effect = self.model.input(Input::Mark {
            utf16,
            text,
            selected,
        });
        self.run(effect, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        element: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let field = self.model.field();
        let (start, end) = (field.byte_offset(range.start), field.byte_offset(range.end));
        let (text, runs) = field_runs(&self.model);
        let line = shape(text, &runs, window);
        let left = element.left() - scroll(&line, field.cursor(), element.size.width);
        Some(Bounds::from_corners(
            point(left + line.x_for_index(start), element.top()),
            point(left + line.x_for_index(end), element.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
