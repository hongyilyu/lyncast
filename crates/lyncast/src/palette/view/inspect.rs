//! What the Tier 3 tests read of a window.

use gpui::{Background, Bounds, Context, EntityInputHandler, Pixels, ScaledPixels, Window, rgb};

use super::PaletteView;
use crate::palette::theme;

/// Where in its field the window draws the caret.
#[derive(Debug, PartialEq)]
pub enum Caret {
    AtStart,
    Inside,
    AtEnd,
}

impl PaletteView {
    /// palette.md's `palette` block text for this window.
    pub fn dump(&self) -> String {
        self.model.dump()
    }

    /// The bounds of the selected line and of the line before it, if the list drew them, and
    /// the list's own.
    pub fn selected_bounds(
        &self,
    ) -> (
        Option<Bounds<Pixels>>,
        Option<Bounds<Pixels>>,
        Bounds<Pixels>,
    ) {
        let line = self.model.selected_line().expect("a selected row");
        let above = line.checked_sub(1);
        (
            self.list.bounds_for_item(line),
            above.and_then(|above| self.list.bounds_for_item(above)),
            self.list.viewport_bounds(),
        )
    }

    /// Where the caret is in `field`, the field that has it, once the window draws the caret
    /// once and wholly inside the field, the input method's place for the caret is where the
    /// window draws it, and no composing underline shows outside the field.
    pub fn caret(
        &mut self,
        field: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Caret, String> {
        let scale = window.scale_factor();
        let near = |at: f32, edge: Pixels| (at - f32::from(edge)).abs() <= 1.0 / scale;
        let shown = |bounds: Bounds<ScaledPixels>, mask: Bounds<ScaledPixels>| {
            let shown = bounds.intersect(&mask);
            (shown.left().0 / scale, shown.right().0 / scale)
        };
        let inside = |(left, right): (f32, f32)| {
            (left >= f32::from(field.left()) || near(left, field.left()))
                && (right <= f32::from(field.right()) || near(right, field.right()))
        };
        let fill = Background::from(rgb(theme::CARET));
        let carets: Vec<_> = window
            .painted_quads()
            .into_iter()
            .filter(|quad| quad.background == fill)
            .collect();
        let [caret] = carets.as_slice() else {
            return Err(format!("the window draws {} carets", carets.len()));
        };
        let drawn = shown(caret.bounds, caret.bounds);
        let caret_shown = shown(caret.bounds, caret.content_mask.bounds);
        if caret_shown != drawn || !inside(drawn) {
            return Err(format!(
                "the caret at {drawn:?} shows at {caret_shown:?}, and the field is at {field:?}"
            ));
        }
        let ime = self
            .selected_text_range(false, window, cx)
            .and_then(|cursor| self.bounds_for_range(cursor.range, field, window, cx))
            .ok_or("no bounds for the input method's caret")?;
        if !near(drawn.0, ime.left()) {
            return Err(format!(
                "the input method's caret is at {ime:?}, and the window's at {drawn:?}"
            ));
        }
        for underline in window.painted_underlines() {
            let underline = shown(underline.bounds, underline.content_mask.bounds);
            if !inside(underline) {
                return Err(format!(
                    "an underline shows at {underline:?}, outside the field at {field:?}"
                ));
            }
        }
        Ok(if near(drawn.0, field.left()) {
            Caret::AtStart
        } else if near(drawn.1, field.right()) {
            Caret::AtEnd
        } else {
            Caret::Inside
        })
    }
}
