use std::cell::Cell;
use std::rc::Rc;

use gpui::{Context, IntoElement, Render, TestAppContext, Window, WindowOptions, div, prelude::*};

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
        .update(|cx| cx.open_window(WindowOptions::default(), |_, cx| cx.new(|_| Blank)))
        .expect("the test platform opens a window");
    assert_eq!(cx.windows().len(), 1, "the window is open");
    window
        .update(cx, |_, window, _| window.remove_window())
        .expect("the window takes an update");
    cx.run_until_parked();
    assert_eq!(closed.get(), 1, "the close observer saw the window go");
    assert!(cx.windows().is_empty(), "no window is left");
}
