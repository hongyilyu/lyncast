/// The window's size in logical pixels. It never resizes.
pub const WIDTH: f32 = 750.0;
pub const HEIGHT: f32 = 480.0;

/// The top margin that puts the window a third of the way down a display `height` tall: a third
/// of the space the window leaves, in whole pixels. A display shorter than the window leaves
/// none.
pub fn top_margin(height: f32) -> f32 {
    ((height - HEIGHT) / 3.0).floor().max(0.0)
}

/// The dark theme, as `0xRRGGBB`.
pub const BACKGROUND: u32 = 0x1e1e20;
pub const BORDER: u32 = 0x3a3a3e;
pub const TEXT: u32 = 0xe8e8ea;
/// Section headers and kind labels.
pub const MUTED: u32 = 0x8e8e93;

pub const FONT: &str = "DejaVu Sans";

/// A section header's height and a row's.
pub const HEADER: f32 = 28.0;
pub const ROW: f32 = 40.0;

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
