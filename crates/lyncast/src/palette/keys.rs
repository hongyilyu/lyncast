//! lyncast binds no GPUI keys: GPUI runs a binding's action before any key listener, and an
//! action never learns whether its key is a repeat. The view reads each key here in the capture
//! phase, before the input handler gets the key's text.

use gpui::KeyDownEvent;
use lyncast_core::Chord;

use super::model::Press;

/// What a key press means. `None` leaves the key to the input handler, which types its text.
/// A held key repeats as presses with `is_held`. Only Enter's first press activates, only
/// Escape's first press takes a step, only Ctrl+K's first press opens or closes the menu, and
/// only the first press of Tab or Ctrl+T changes the screen or its filter.
pub fn press(event: &KeyDownEvent) -> Option<Press> {
    let mods = event.keystroke.modifiers;
    let bare = !(mods.control || mods.alt || mods.platform);
    let ctrl = mods.control && !mods.alt && !mods.platform;
    Some(match event.keystroke.key.as_str() {
        "enter" if event.is_held => Press::Ignore,
        "enter" if bare => Press::Activate(Chord::Enter),
        "enter" if ctrl => Press::Activate(Chord::CtrlEnter),
        "up" if bare => Press::Up,
        "p" if ctrl => Press::Up,
        "down" if bare => Press::Down,
        "n" if ctrl => Press::Down,
        "left" if bare => Press::Left,
        "right" if bare => Press::Right,
        "home" if bare => Press::Home,
        "end" if bare => Press::End,
        "delete" if bare => Press::Delete,
        "backspace" if bare => Press::Backspace {
            held: event.is_held,
        },
        "escape" if event.is_held => Press::Ignore,
        "escape" if bare => Press::Escape,
        "k" if ctrl && event.is_held => Press::Ignore,
        "k" if ctrl => Press::Menu,
        "tab" if event.is_held => Press::Ignore,
        "tab" if bare => Press::Tab,
        "t" if ctrl && event.is_held => Press::Ignore,
        "t" if ctrl => Press::CycleKinds,
        _ => return None,
    })
}
