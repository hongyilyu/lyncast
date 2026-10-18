//! lyncast binds no GPUI keys: GPUI runs a binding's action before any key listener, and an
//! action never learns whether its key is a repeat. The view reads each key here in the capture
//! phase, before the input handler gets the key's text.

use gpui::KeyDownEvent;

use super::model::Press;

/// What a key press means. `None` leaves the key to the input handler, which types its text.
pub fn press(event: &KeyDownEvent) -> Option<Press> {
    let mods = event.keystroke.modifiers;
    let bare = !(mods.control || mods.alt || mods.platform);
    let ctrl = mods.control && !mods.alt && !mods.platform;
    Some(match event.keystroke.key.as_str() {
        "up" if bare => Press::Up,
        "p" if ctrl => Press::Up,
        "down" if bare => Press::Down,
        "n" if ctrl => Press::Down,
        "left" if bare => Press::Left,
        "right" if bare => Press::Right,
        "home" if bare => Press::Home,
        "end" if bare => Press::End,
        "delete" if bare => Press::Delete,
        "backspace" if bare => Press::Backspace,
        "escape" if bare => Press::Escape,
        _ => return None,
    })
}
