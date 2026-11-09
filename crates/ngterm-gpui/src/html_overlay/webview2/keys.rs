//! The app's keys while a page's web view holds the keyboard. `WebView2` gets every key first there, where the WPE host
//! sees the keymap's before the page's (`keymap_dispatch`): so every key `WebView2` reports (`AcceleratorKeyPressed`:
//! one with Control or Alt held, or one that types no character, Esc and the arrows among them) is asked about, as
//! upstream's monitor sees every press, and one the keymap or the window would take (a custom command, a menu chord,
//! Ctrl+Tab's switcher, the Esc that abandons a half-typed leader) is kept from the page and dispatched to the window
//! instead. Every other key, typing and the page's own shortcuts (copy, paste, select all), stays the page's. A
//! typed character is not among the keys `WebView2` reports: the page script's report of it (`bridge`) reaches the
//! monitor too late to take the key, so it can only end a half-typed leader or a repeat window
//! (`keymap_dispatch::page_typed`), and a leader's plain-letter tail, which upstream's monitor fires, cannot fire here.
//! That script reports only the top frame: typing inside an iframe leaves a leader or repeat window live until
//! another reported key ends it or its timeout lapses.

use gpui_kit::{Keystroke, Modifiers};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};

use crate::terminal::input;

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

/// The keystroke a press of `virtual_key` with `modifiers` held is to the app, `None` for a key no chord names (so a
/// bare modifier is never asked about and a half-typed leader keeps waiting).
#[must_use]
pub fn chord_keystroke(virtual_key: u32, modifiers: Modifiers) -> Option<Keystroke> {
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

/// The physical identity of a Windows accelerator, including the extended bit that distinguishes the navigation
/// block and keypad. Synthesized events with no scan code use the same layout fallback as native window events.
#[must_use]
pub fn keycode(virtual_key: u32, scan_code: u32, extended: bool) -> u32 {
    let scan_code = scan_code | if extended { 0xe000 } else { 0 };
    input::xkb_keycode_of_scan_code(scan_code)
        .or_else(|| {
            let virtual_key = u16::try_from(virtual_key).ok()?;
            input::xkb_keycode_of_scan_code(input::virtual_key_scan_code(virtual_key, scan_code))
        })
        .unwrap_or(0)
}

/// The keystroke the app takes from a press of `virtual_key` in a page (module docs), `None` for the page's own.
/// `repeat` is the event's own word that the key was already down.
#[must_use]
pub fn app_key(virtual_key: u32, keycode: u32, repeat: bool) -> Option<Keystroke> {
    let keystroke = chord_keystroke(virtual_key, held())?;
    let stroke = crate::keymap_dispatch::page_stroke(&keystroke, keycode, repeat);
    crate::keymap_dispatch::takes(&stroke).then_some(keystroke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_and_native_events_share_physical_identities() {
        for (virtual_key, scan, extended, expected) in [
            (0x54, 0x14, false, "KeyT"),
            (0x53, 0x1f, false, "KeyS"),
            (0x74, 0x3f, false, "F5"),
            (0x0d, 0x1c, false, "Enter"),
            (0x0d, 0x1c, true, "NumpadEnter"),
            (0x0d, 0, true, "NumpadEnter"),
            (0x26, 0, false, "ArrowUp"),
            (0x26, 0x48, true, "ArrowUp"),
            (0x68, 0x48, false, "Numpad8"),
        ] {
            assert_eq!(
                keycode(virtual_key, scan, extended),
                input::xkb_keycode(expected).unwrap()
            );
        }
        assert_eq!(keycode(0x54, 0, false), input::xkb_keycode("KeyT").unwrap());
        assert_eq!(keycode(u32::MAX, 0xffff, false), 0);
    }

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
    fn every_named_key_is_asked_about_and_a_bare_modifier_is_not() {
        let ctrl = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(
            chord_keystroke(0x54, ctrl).map(|keystroke| keystroke.key).as_deref(),
            Some("t")
        );
        for (key, modifiers, name) in [
            (0x54, Modifiers::default(), "t"),
            (0x1B, Modifiers::default(), "escape"),
            (
                0x54,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
                "t",
            ),
            (0x71, Modifiers::default(), "f2"),
        ] {
            assert_eq!(
                chord_keystroke(key, modifiers)
                    .map(|keystroke| keystroke.key)
                    .as_deref(),
                Some(name)
            );
        }
        assert!(chord_keystroke(0x11, ctrl).is_none(), "a modifier alone");
    }
    #[test]
    fn every_named_virtual_key_has_the_keymaps_spelling() {
        let keys = [
            (0x08, "backspace"),
            (0x09, "tab"),
            (0x0D, "enter"),
            (0x1B, "escape"),
            (0x20, "space"),
            (0x21, "pageup"),
            (0x22, "pagedown"),
            (0x23, "end"),
            (0x24, "home"),
            (0x25, "left"),
            (0x26, "up"),
            (0x27, "right"),
            (0x28, "down"),
            (0x2D, "insert"),
            (0x2E, "delete"),
            (0xBA, ";"),
            (0xBB, "="),
            (0xBC, ","),
            (0xBD, "-"),
            (0xBE, "."),
            (0xBF, "/"),
            (0xC0, "`"),
            (0xDB, "["),
            (0xDC, "\\"),
            (0xDD, "]"),
            (0xDE, "'"),
        ];
        for (key, name) in keys {
            assert_eq!(key_name(key).as_deref(), Some(name), "{key:#x}");
        }
        for key in [0, 0x10, 0x11, 0x12, 0x5B, 0x5C, 0x88, 0xFF, u32::MAX] {
            assert_eq!(key_name(key), None, "{key:#x}");
            assert!(
                chord_keystroke(
                    key,
                    Modifiers {
                        control: true,
                        ..Modifiers::default()
                    }
                )
                .is_none()
            );
        }
    }

    #[test]
    fn accelerator_modifiers_and_function_keys_preserve_the_whole_keystroke() {
        for modifiers in [
            Modifiers {
                control: true,
                shift: true,
                ..Modifiers::default()
            },
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
            Modifiers {
                platform: true,
                ..Modifiers::default()
            },
        ] {
            let key = chord_keystroke(0x41, modifiers).unwrap();
            assert_eq!(key.key, "a");
            assert_eq!(key.modifiers, modifiers);
            assert_eq!(key.key_char, None);
        }
        for key in 0x70..=0x87 {
            assert_eq!(
                chord_keystroke(key, Modifiers::default()).unwrap().key,
                format!("f{}", key - 0x6F)
            );
        }
        for (key, name) in (0x41..=0x5A).zip('a'..='z').chain((0x30..=0x39).zip('0'..='9')) {
            assert_eq!(key_name(key), Some(name.to_string()));
        }
    }

    #[test]
    fn the_threads_keyboard_state_keeps_page_typing_and_routes_control_tab() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState};
        struct Restore([u8; 256]);
        impl Drop for Restore {
            fn drop(&mut self) {
                // SAFETY: restoring only this test thread's keyboard state.
                unsafe { SetKeyboardState(self.0.as_ptr()) };
            }
        }
        let mut saved = Restore([0; 256]);
        // SAFETY: the full 256-byte buffer required by the keyboard API.
        assert_ne!(unsafe { GetKeyboardState(saved.0.as_mut_ptr()) }, 0);
        let mut state = [0; 256];
        for keys in [
            vec![],
            vec![VK_CONTROL],
            vec![VK_SHIFT],
            vec![VK_CONTROL, VK_SHIFT],
            vec![VK_MENU],
            vec![VK_LWIN],
            vec![VK_RWIN],
        ] {
            state.fill(0);
            for key in &keys {
                state[usize::from(*key)] = 0x80;
            }
            // SAFETY: this updates only the calling thread, never injects system input.
            assert_ne!(unsafe { SetKeyboardState(state.as_ptr()) }, 0);
            let modifiers = held();
            assert_eq!(modifiers.control, keys.contains(&VK_CONTROL));
            assert_eq!(modifiers.alt, keys.contains(&VK_MENU));
            assert_eq!(modifiers.shift, keys.contains(&VK_SHIFT));
            assert!(!modifiers.function, "Windows reports no Fn modifier");
            assert_eq!(modifiers.platform, keys.contains(&VK_LWIN) || keys.contains(&VK_RWIN));
            if modifiers.control {
                assert_eq!(app_key(0x09, keycode(0x09, 0, false), false).unwrap().key, "tab");
            } else if !modifiers.alt && !modifiers.platform {
                assert!(
                    app_key(0x41, keycode(0x41, 0, false), false).is_none(),
                    "plain typing stays in the page"
                );
                assert!(
                    app_key(0x87, keycode(0x87, 0, false), false).is_none(),
                    "an unbound function key stays in the page"
                );
            }
        }
    }

    /// A half-typed leader `ctrl+a>g`, armed through the monitor, with no modifier held on this thread.
    fn armed_leader() -> PlainKeyboard {
        let keyboard = PlainKeyboard::new();
        let (keymap, _) =
            ngterm_core::keymap::parse_keymap("command \"g\" ctrl+a>g true\n", &ngterm_core::keymap::linux_defaults());
        crate::keymap_dispatch::load_keymap(&keymap);
        assert_eq!(crate::keymap_dispatch::press_dispatched("ctrl-a"), Some(false));
        assert!(crate::keymap_dispatch::sequence_is_live());
        keyboard
    }

    /// No key held on this thread until dropped, whatever the desktop's keyboard holds.
    struct PlainKeyboard([u8; 256]);

    impl PlainKeyboard {
        fn new() -> Self {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState};
            let mut saved = [0; 256];
            // SAFETY: the full 256-byte buffer required by the keyboard API.
            assert_ne!(unsafe { GetKeyboardState(saved.as_mut_ptr()) }, 0);
            // SAFETY: this updates only the calling thread, never injects system input.
            assert_ne!(unsafe { SetKeyboardState([0u8; 256].as_ptr()) }, 0);
            Self(saved)
        }
    }

    impl Drop for PlainKeyboard {
        fn drop(&mut self) {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState;
            // SAFETY: restoring only this test thread's keyboard state.
            unsafe { SetKeyboardState(self.0.as_ptr()) };
        }
    }

    /// Upstream's Esc in a page: it abandons a half-typed leader and is the keymap's only then.
    #[test]
    fn a_plain_escape_in_a_page_is_handed_over_while_a_leader_waits() {
        let _keyboard = armed_leader();
        assert_eq!(
            app_key(0x1B, keycode(0x1B, 0, false), false)
                .map(|keystroke| keystroke.key)
                .as_deref(),
            Some("escape")
        );
        assert_eq!(
            crate::keymap_dispatch::press_page_report(
                &chord_keystroke(0x1B, held()).unwrap(),
                keycode(0x1B, 0, false),
                false
            ),
            Some(false)
        );
        assert!(!crate::keymap_dispatch::sequence_is_live());
        assert!(
            app_key(0x1B, keycode(0x1B, 0, false), false).is_none(),
            "with nothing to cancel Esc is the page's"
        );
    }

    /// Upstream's unmatched press: a plain key the page keeps ends the half-typed leader.
    #[test]
    fn a_plain_key_the_page_keeps_ends_a_half_typed_leader() {
        for virtual_key in [0x25, 0x0D, 0x58] {
            let _keyboard = armed_leader();
            assert!(
                app_key(virtual_key, keycode(virtual_key, 0, false), false).is_none(),
                "{virtual_key:#x} stays in the page"
            );
            assert!(!crate::keymap_dispatch::sequence_is_live(), "{virtual_key:#x}");
        }
    }

    /// An owned autorepeat is handed over, and its report carries the repeat flag to the dispatch, which does not fire
    /// the held key again.
    #[test]
    fn a_held_keys_autorepeat_is_handed_over_without_firing_again() {
        let _keyboard = PlainKeyboard::new();
        let (keymap, _) =
            ngterm_core::keymap::parse_keymap("command \"x\" f5 true\n", &ngterm_core::keymap::linux_defaults());
        crate::keymap_dispatch::load_keymap(&keymap);
        assert!(app_key(0x74, keycode(0x74, 0, false), false).is_some());
        let keystroke = chord_keystroke(0x74, held()).unwrap();
        let press = |repeat| crate::keymap_dispatch::press_page_report(&keystroke, keycode(0x74, 0, false), repeat);
        assert_eq!(press(false), Some(true));
        assert!(
            app_key(0x74, keycode(0x74, 0, false), true).is_some(),
            "the keymap owns the held key"
        );
        assert_eq!(press(true), Some(false));
        assert!(app_key(0x74, keycode(0x74, 0, false), false).is_some());
        assert_eq!(press(false), Some(true));
    }

    /// A keyboard with exactly `keys` held on this thread until dropped.
    fn holding(keys: &[u16]) -> PlainKeyboard {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetKeyboardState;
        let keyboard = PlainKeyboard::new();
        let mut state = [0u8; 256];
        for key in keys {
            state[usize::from(*key)] = 0x80;
        }
        // SAFETY: this updates only the calling thread, never injects system input.
        assert_ne!(unsafe { SetKeyboardState(state.as_ptr()) }, 0);
        keyboard
    }

    /// Upstream's `testKeyRepeatDoesNotToggleTwice` in a page, from the keys the event reports to the window's
    /// dispatch of what they hand over: a held chord fires once, however often it repeats, and a new press fires again.
    #[test]
    fn a_chord_held_in_a_page_fires_once() {
        let _keyboard = holding(&[VK_CONTROL, VK_SHIFT]);
        let (keymap, _) = ngterm_core::keymap::parse_keymap("", &ngterm_core::keymap::linux_defaults());
        crate::keymap_dispatch::load_keymap(&keymap);
        let press = |repeat: bool| {
            let keystroke = app_key(0x54, keycode(0x54, 0, false), repeat).expect("the keymap owns the chord");
            crate::keymap_dispatch::press_page_report(&keystroke, keycode(0x54, 0, false), repeat)
        };
        assert_eq!(press(false), Some(true), "the first press fires");
        for _ in 0..3 {
            assert_eq!(press(true), Some(false), "an autorepeat fires nothing");
        }
        assert_eq!(press(false), Some(true), "a new press fires again");
    }

    /// A plain key the leader is waiting for is the keymap's, and a bare modifier leaves the leader waiting.
    #[test]
    fn a_leaders_next_plain_key_is_handed_over_and_a_modifier_is_not_asked_about() {
        let _keyboard = armed_leader();
        assert!(app_key(0x10, keycode(0x10, 0, false), false).is_none(), "Shift alone");
        assert!(crate::keymap_dispatch::sequence_is_live(), "still waiting");
        assert_eq!(
            app_key(0x47, keycode(0x47, 0, false), false)
                .map(|keystroke| keystroke.key)
                .as_deref(),
            Some("g")
        );
    }
}
