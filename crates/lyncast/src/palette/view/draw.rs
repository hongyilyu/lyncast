//! Elements drawn from their arguments alone: the field's text, a key's cap, a header, a row, a
//! clip, an icon or a letter square, and the card.

use std::sync::Arc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, IntoElement, ParentElement, Pixels, RenderImage, ShapedLine, SharedString, Styled,
    TextRun, UnderlineStyle, Window, div, font, img, px, rgb, rgba,
};
use lyncast_core::Row;
use lyncast_core::calc;

use crate::palette::clips::ClipLine;
use crate::palette::field::Field;
use crate::palette::{model, theme};

pub(super) fn key_cap(name: &'static str) -> impl IntoElement {
    div()
        .px_1()
        .rounded(px(4.0))
        .bg(rgb(theme::BADGE))
        .text_color(rgb(theme::MUTED))
        .child(name)
}

pub(super) fn field_runs(field: &Field, placeholder: &'static str) -> (SharedString, Vec<TextRun>) {
    let run = |len, color, underline| TextRun {
        len,
        font: font(theme::FONT),
        color: rgb(color).into(),
        background_color: None,
        underline,
        strikethrough: None,
    };
    if field.shows_placeholder() {
        let runs = vec![run(placeholder.len(), theme::MUTED, None)];
        return (placeholder.into(), runs);
    }
    let text = field.text();
    let marked = field.marked().unwrap_or(text.len()..text.len());
    let underline = UnderlineStyle {
        thickness: px(1.0),
        color: None,
        wavy: false,
    };
    let runs = [
        run(marked.start, theme::TEXT, None),
        run(marked.len(), theme::TEXT, Some(underline)),
        run(text.len() - marked.end, theme::TEXT, None),
    ]
    .into_iter()
    .filter(|run| run.len > 0)
    .collect();
    (SharedString::from(text.to_owned()), runs)
}

pub(super) fn shape(text: SharedString, runs: &[TextRun], window: &mut Window) -> ShapedLine {
    window
        .text_system()
        .shape_line(text, px(theme::FIELD_TEXT), runs, None)
}

/// How far left of the field the line starts, so the caret at `cursor` stays inside a field
/// `width` wide.
pub(super) fn scroll(line: &ShapedLine, cursor: usize, width: Pixels) -> Pixels {
    (line.x_for_index(cursor) + px(theme::CARET_WIDTH) - width).max(px(0.0))
}

pub(super) fn header_line(name: SharedString) -> AnyElement {
    div()
        .w_full()
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

pub(super) fn row_line(row: &Row, icon: Option<Arc<RenderImage>>, selected: bool) -> AnyElement {
    if let Some(card) = &row.card {
        return card_line(card, selected);
    }
    let kind = model::kind_label(row.subject.key().kind());
    let line = div()
        .w_full()
        .h(px(theme::ROW))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px_2()
        .rounded(px(6.0))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(icon_slot(icon, &row.title))
                .child(row.title.clone()),
        )
        .children(kind.map(|kind| div().text_sm().text_color(rgb(theme::MUTED)).child(kind)));
    line.when(selected, |line| line.bg(rgba(theme::SELECTED)))
        .into_any_element()
}

pub(super) fn clip_line(clip: &ClipLine, selected: bool) -> AnyElement {
    let line = div()
        .w_full()
        .h(px(theme::ROW))
        .flex_none()
        .flex()
        .items_center()
        .gap_3()
        .px_2()
        .rounded(px(6.0))
        .child(letter_square(clip.letter()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .child(clip.title.clone()),
        );
    line.when(selected, |line| line.bg(rgba(theme::SELECTED)))
        .into_any_element()
}

pub(super) fn letter_square(letter: impl Into<SharedString>) -> AnyElement {
    div()
        .size(px(theme::ICON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .bg(rgb(theme::BADGE))
        .text_xs()
        .text_color(rgb(theme::MUTED))
        .child(letter.into())
        .into_any_element()
}

pub(super) fn icon_slot(icon: Option<Arc<RenderImage>>, title: &str) -> AnyElement {
    if let Some(icon) = icon {
        return img(icon)
            .size(px(theme::ICON))
            .flex_none()
            .into_any_element();
    }
    letter_square(
        title
            .chars()
            .take(1)
            .flat_map(char::to_uppercase)
            .collect::<String>(),
    )
}

pub(super) fn card_line(card: &calc::Answer, selected: bool) -> AnyElement {
    let half = |(text, badge): (&str, Option<&str>)| {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .gap_1()
            .child(div().text_xl().child(text.to_owned()))
            .children(badge.map(|badge| {
                div()
                    .px_2()
                    .rounded(px(4.0))
                    .bg(rgb(theme::BADGE))
                    .text_xs()
                    .text_color(rgb(theme::MUTED))
                    .child(badge.to_owned())
            }))
    };
    let [first, second] = model::card_halves(card).map(half);
    let border = if selected {
        theme::ACCENT
    } else {
        theme::BORDER
    };
    div()
        .w_full()
        .h(px(theme::CARD))
        .flex_none()
        .flex()
        .items_center()
        .px_4()
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(border))
        .child(first)
        .child(div().text_color(rgb(theme::MUTED)).child("→"))
        .child(second)
        .into_any_element()
}
