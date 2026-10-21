//! What the Tier 3 tests read of a window.

use std::sync::Arc;

use gpui::{
    App, Background, Bounds, Context, EntityInputHandler, Pixels, RenderImage, ScaledPixels,
    Window, rgb,
};

use super::PaletteView;
use crate::palette::model::{Item, Line};
use crate::palette::preview::Shown;
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
    pub fn dump(&self, cx: &App) -> String {
        let preview = self.preview_line(cx);
        self.model.dump(preview.as_deref())
    }

    fn preview_line(&self, cx: &App) -> Option<String> {
        let id = self.model.selected_clip()?.id;
        Some(match self.shared.preview.read(cx).shown() {
            None => "none".to_owned(),
            Some(Shown::Text) => format!("text {:?}", self.clip_preview(id, cx)?),
            Some(Shown::Files) => format!("files {:?}", self.clip_preview(id, cx)?),
            Some(Shown::Loading) => "loading".to_owned(),
            Some(Shown::Image {
                image,
                width,
                height,
            }) => {
                let drawn = image.size(0);
                format!(
                    "image {width}x{height} drawn {}x{}",
                    drawn.width.0, drawn.height.0
                )
            }
            Some(Shown::TooLarge { width, height }) => {
                format!("image too large {width}x{height}")
            }
            Some(Shown::Unreadable(reason)) => format!("no image: {reason}"),
        })
    }

    /// The image the preview pane shows, if it shows one.
    pub fn preview_image(&self, cx: &App) -> Option<Arc<RenderImage>> {
        match self.shared.preview.read(cx).shown()? {
            Shown::Image { image, .. } => Some(Arc::clone(image)),
            _ => None,
        }
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

    /// The titles of the rows the list has laid out since it last reset, but for those above
    /// the first one it shows.
    pub fn laid_out(&self) -> Vec<String> {
        let lines = self.model.lines().iter().enumerate();
        lines
            .filter(|(at, _)| self.list.bounds_for_item(*at).is_some())
            .filter_map(|(_, line)| match *line {
                Line::Row(at) => Some(match self.model.item(at) {
                    Item::Row(row) => row.title.clone(),
                    Item::Clip(clip) => clip.title.clone(),
                }),
                Line::Header(_) => None,
            })
            .collect()
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
