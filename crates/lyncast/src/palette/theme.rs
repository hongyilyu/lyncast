/// The window's size in logical pixels. It never resizes.
pub const WIDTH: f32 = 750.0;
pub const HEIGHT: f32 = 480.0;

/// The top margin that puts the window a third of the way down a display `height` tall: a third
/// of the space the window leaves, in whole pixels. A display shorter than the window leaves
/// none.
pub fn top_margin(height: f32) -> f32 {
    ((height - HEIGHT) / 3.0).floor().max(0.0)
}

/// The window's background, as `0xRRGGBBAA`. It lets a tenth of what lies behind the window
/// through, so the blur that niri's layer rule draws there shows.
pub const BACKGROUND: u32 = 0x1e1e20e6;
/// The window's corner radius in logical pixels.
pub const RADIUS: f32 = 12.0;

/// The dark theme, as `0xRRGGBB`.
pub const BORDER: u32 = 0x3a3a3e;
pub const TEXT: u32 = 0xe8e8ea;
/// Section headers, kind labels, and the placeholder.
pub const MUTED: u32 = 0x8e8e93;
/// The selected row's background, as `0xRRGGBBAA`. White at 7% is lighter than whatever the
/// translucent background shows.
pub const SELECTED: u32 = 0xffffff12;
/// The selected card's border.
pub const ACCENT: u32 = 0x5e8bff;
pub const CARET: u32 = 0x5e8bff;
/// The card's badges and the footer's keys.
pub const BADGE: u32 = 0x2a2a2e;
/// Why an activation failed, in the footer.
pub const FAILED: u32 = 0xff8a80;

pub const FONT: &str = "DejaVu Sans";

/// The field's bar, its line, its text size, and its caret's width.
pub const FIELD: f32 = 56.0;
pub const FIELD_LINE: f32 = 24.0;
pub const FIELD_TEXT: f32 = 18.0;
pub const CARET_WIDTH: f32 = 2.0;

/// A section header's height, a row's, and the calculator card's, and the list's padding above
/// and below them.
pub const HEADER: f32 = 28.0;
pub const ROW: f32 = 40.0;
pub const CARD: f32 = 84.0;
pub const LIST_PAD: f32 = 4.0;

/// The side of a row's icon, and of the letter square that stands in for one.
pub const ICON: f32 = 24.0;

/// The clipboard screen's list width, beside the preview pane, and the side of a file's badge
/// in the pane.
pub const CLIP_LIST: f32 = 300.0;
pub const FILE_BADGE: f32 = 48.0;

/// How far past the list's edges it lays out rows, so the row a key selects next has its
/// height before the list scrolls to it.
pub const OVERDRAW: f32 = 200.0;

pub const FOOTER: f32 = 36.0;

/// The Actions menu: its background, width, item height, and its inset from the window's
/// right edge and from the footer.
pub const MENU: u32 = 0x262629;
pub const MENU_WIDTH: f32 = 300.0;
pub const MENU_ROW: f32 = 32.0;
pub const MENU_INSET: f32 = 8.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc;

    #[test]
    fn the_geometry_is_as_the_doc_says() {
        let blocks = doc::PALETTE.blocks("geometry");
        let [block] = blocks.as_slice() else {
            panic!("palette.md holds one geometry block, not {}", blocks.len());
        };
        let (window, margins) = block.lines.split_first().expect("a window line");
        assert_eq!(
            window.as_str(),
            format!("window {WIDTH}x{HEIGHT}"),
            "line {}",
            block.line
        );
        for line in margins {
            let (height, want) = line
                .split_once(" => ")
                .unwrap_or_else(|| panic!("line {}: not <height> => <top>: {line}", block.line));
            let height: f32 = height.parse().expect("a display height");
            assert_eq!(top_margin(height).to_string(), want, "{line}");
        }
    }
}
