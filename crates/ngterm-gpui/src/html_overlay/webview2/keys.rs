//! The app's keys while a page's web view holds the keyboard. `WebView2` gets every key first there, where the WPE host
//! sees the keymap's before the page's (`keymap_dispatch`): so a chord with Control, Alt or the Windows key held, or a
//! function key, is asked about as `WebView2` reports it (`AcceleratorKeyPressed`), and one the keymap or the window
//! would take (a custom command, a menu chord, Ctrl+Tab's switcher) is kept from the page and dispatched to the window
//! instead. Every other key, typing and the page's own shortcuts (copy, paste, select all), stays the page's.

use gpui_kit::{Keystroke, Modifiers};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};

use crate::terminal::input::{KeyStroke, Source};

/// GPUI's name of the key at virtual key `virtual_key`, by its US position; `None` for a key no chord names (a
/// modifier, a media key).
#[must_use]
pub fn key_name(virtual_key: u32) -> Option<String> {
    let char_of = |offset: u32, first: char| char::from_u32(u32::from(first) + offset).map(String::from);
    match virtual_key {
        0x41..=0x5A => char_of(virtual_key - 0x41, 'a'),
        0x30..=0x39 => char_of(virtual_key - 0x30, '0'),
        0x70..=0x87 => Some(format!("f{}", virtual_key - 0x6F)),
        _ => named(virtual_key).map(str::to_owned),
    }
}

fn named(virtual_key: u32) -> Option<&'static str> {
    Some(match virtual_key {
        0x08 => "backspace",
        0x09 => "tab",
        0x0D => "enter",
        0x1B => "escape",
        0x20 => "space",
        0x21 => "pageup",
        0x22 => "pagedown",
        0x23 => "end",
        0x24 => "home",
        0x25 => "left",
        0x26 => "up",
        0x27 => "right",
        0x28 => "down",
        0x2D => "insert",
        0x2E => "delete",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => return None,
    })
}

/// The keystroke a press of `virtual_key` with `modifiers` held is to the app, when it may be the app's at all: with
/// Control, Alt or the Windows key held, or a function key.
#[must_use]
pub fn chord_keystroke(virtual_key: u32, modifiers: Modifiers) -> Option<Keystroke> {
    let function_key = (0x70..=0x87).contains(&virtual_key);
    if !(modifiers.control || modifiers.alt || modifiers.platform || function_key) {
        return None;
    }
    Some(Keystroke {
        modifiers,
        key: key_name(virtual_key)?,
        key_char: None,
    })
}

/// The modifiers held now, as the thread's keyboard state has them.
fn held() -> Modifiers {
    // SAFETY: reads of the calling thread's keyboard state.
    let down = |key: u16| unsafe { GetKeyState(i32::from(key)) } < 0;
    Modifiers {
        control: down(VK_CONTROL),
        alt: down(VK_MENU),
        shift: down(VK_SHIFT),
        platform: down(VK_LWIN) || down(VK_RWIN),
        function: false,
    }
}

/// The keystroke the app takes from a press of `virtual_key` in a page (module docs), `None` for the page's own.
#[must_use]
pub fn app_key(virtual_key: u32) -> Option<Keystroke> {
    let keystroke = chord_keystroke(virtual_key, held())?;
    let stroke = KeyStroke::new(&keystroke, &Source::Keystroke, false);
    crate::keymap_dispatch::takes(&stroke).then_some(keystroke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_as_gpui_names_them() {
        assert_eq!(key_name(0x54).as_deref(), Some("t"));
        assert_eq!(key_name(0x31).as_deref(), Some("1"));
        assert_eq!(key_name(0x70).as_deref(), Some("f1"));
        assert_eq!(key_name(0x87).as_deref(), Some("f24"));
        assert_eq!(key_name(0x09).as_deref(), Some("tab"));
        assert_eq!(key_name(0xBC).as_deref(), Some(","));
        assert_eq!(key_name(0x11), None, "a modifier");
    }

    #[test]
    fn only_a_chord_or_a_function_key_may_be_the_apps() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(
            chord_keystroke(0x54, ctrl).map(|keystroke| keystroke.key).as_deref(),
            Some("t")
        );
        assert!(
            chord_keystroke(0x54, Modifiers::default()).is_none(),
            "typing is the page's"
        );
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert!(chord_keystroke(0x54, shift).is_none());
        assert!(chord_keystroke(0x71, Modifiers::default()).is_some(), "a function key");
        assert!(chord_keystroke(0x11, ctrl).is_none(), "a modifier alone");
    }
}
