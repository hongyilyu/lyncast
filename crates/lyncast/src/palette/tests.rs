use std::cell::Cell;
use std::rc::Rc;

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer};
use gpui::{
    Context, IntoElement, Render, TestAppContext, Window, WindowBounds, WindowKind, div,
    prelude::*, px, size,
};

use crate::doc;

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

#[gpui::test]
fn a_window_opens_and_closes(cx: &mut TestAppContext) {
    let closed = Rc::new(Cell::new(0));
    let seen = Rc::clone(&closed);
    cx.update(|cx| {
        cx.on_window_closed(move |_, _| seen.set(seen.get() + 1))
            .detach();
    });
    let window = cx
        .update(|cx| cx.open_window(super::options(cx), |_, cx| cx.new(|_| Blank)))
        .expect("the test platform opens a window");
    assert_eq!(cx.windows().len(), 1, "the window is open");
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window takes an update");
    cx.run_until_parked();
    assert_eq!(closed.get(), 1, "the close observer saw the window go");
    assert!(cx.windows().is_empty(), "no window is left");
}

#[gpui::test]
fn the_window_is_placed_as_the_geometry_block_says(cx: &mut TestAppContext) {
    let blocks = doc::PALETTE.blocks("geometry");
    let block = blocks.first().expect("palette.md holds a geometry block");
    let (width, height) = block.lines[0]
        .strip_prefix("window ")
        .and_then(|sizes| sizes.split_once('x'))
        .expect("window <w>x<h>");
    let (display, options) = cx.update(|cx| {
        let display = cx.displays()[0].bounds().size.height;
        (display, super::options(cx))
    });
    let top = block.lines[1..]
        .iter()
        .find_map(|line| line.strip_prefix(&format!("{} => ", f32::from(display))))
        .unwrap_or_else(|| panic!("the geometry block has no line for a display {display:?} tall"));
    let Some(WindowBounds::Windowed(bounds)) = options.window_bounds else {
        panic!("the window has windowed bounds");
    };
    let pixels = |text: &str| px(text.parse::<f32>().expect("pixels"));
    assert_eq!(bounds.size, size(pixels(width), pixels(height)));
    let WindowKind::LayerShell(shell) = options.kind else {
        panic!("the window is a layer-shell surface");
    };
    assert_eq!(shell.margin, Some((pixels(top), px(0.0), px(0.0), px(0.0))));
    assert_eq!(shell.layer, Layer::Overlay);
    assert_eq!(shell.anchor, Anchor::TOP);
    assert_eq!(
        shell.keyboard_interactivity,
        KeyboardInteractivity::Exclusive
    );
    assert_eq!(shell.namespace, "lyncast");
}
