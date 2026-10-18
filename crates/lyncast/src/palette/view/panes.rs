//! The clipboard screen's preview pane and information panel.

use std::sync::Arc;

use gpui::{
    AnyElement, App, IntoElement, ObjectFit, ParentElement, Styled, StyledImage, div, img, px, rgb,
};

use super::PaletteView;
use crate::palette::clips::ClipLine;
use crate::palette::preview::Shown;
use crate::palette::theme;

impl PaletteView {
    pub(super) fn preview_pane(&self, clip: &ClipLine, cx: &App) -> AnyElement {
        let muted = |text: String| {
            div()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(text)
                .into_any_element()
        };
        let text = || self.clip_preview(clip.id, cx).unwrap_or_default();
        let preview = self.preview(cx);
        let shown = preview
            .as_ref()
            .and_then(|preview| preview.read(cx).shown());
        match shown {
            Some(Shown::Text) => div().text_sm().child(text()).into_any_element(),
            Some(Shown::Files) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(file_badge(clip.path.as_deref().unwrap_or_default()))
                .child(div().text_sm().child(text()))
                .into_any_element(),
            Some(Shown::Image {
                image,
                width,
                height,
            }) => div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    img(Arc::clone(image))
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .object_fit(ObjectFit::ScaleDown),
                )
                .child(muted(format!("{width}×{height}")))
                .into_any_element(),
            Some(Shown::TooLarge { width, height }) => {
                muted(format!("Image too large to preview: {width}×{height}"))
            }
            Some(Shown::Unreadable(reason)) => muted(format!("No preview: {reason}")),
            Some(Shown::Loading) | None => muted("Loading…".to_owned()),
        }
    }
}

pub(super) fn info_panel(clip: &ClipLine) -> AnyElement {
    let lines = clip.info().into_iter().map(|(label, value)| {
        div()
            .flex()
            .justify_between()
            .gap_3()
            .child(div().text_color(rgb(theme::MUTED)).child(label))
            .child(div().min_w_0().truncate().child(value))
    });
    div()
        .flex_none()
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .border_t_1()
        .border_color(rgb(theme::BORDER))
        .text_sm()
        .children(lines)
        .into_any_element()
}

fn file_badge(path: &str) -> AnyElement {
    let extension = std::path::Path::new(path)
        .extension()
        .map_or("FILE".to_owned(), |extension| {
            extension.to_string_lossy().to_uppercase()
        });
    div()
        .size(px(theme::FILE_BADGE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(rgb(theme::BADGE))
        .text_sm()
        .text_color(rgb(theme::MUTED))
        .child(extension)
        .into_any_element()
}
