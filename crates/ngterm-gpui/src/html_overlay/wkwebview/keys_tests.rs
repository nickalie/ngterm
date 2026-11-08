//! Native page keys keep their GPUI spelling while the application keymap decides ownership.

use gpui_kit::{Keystroke, Modifiers};
use ngterm_core::keymap::{linux_defaults, parse_keymap};

use super::{NativeKey, app_key, app_key_from_native};
use crate::keymap_dispatch;

fn load(chord: &str) {
    let (keymap, diagnostics) = parse_keymap(&format!("command \"probe\" {chord} true\n"), &linux_defaults());
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    keymap_dispatch::load_keymap(&keymap);
    keymap_dispatch::keyboard_left();
}

fn native(keycode: u32, unshifted: Option<char>, modifiers: Modifiers) -> NativeKey {
    NativeKey {
        keycode,
        unshifted,
        modifiers,
        ascii_layout: true,
        ..NativeKey::default()
    }
}

#[test]
fn named_keys_use_gpuis_names_before_the_layout_character() {
    let modifiers = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    for (code, chord, name) in [
        (36, "return", "enter"),
        (104, "return", "enter"),
        (22, "delete", "backspace"),
        (65, "space", "space"),
        (113, "left", "left"),
        (114, "right", "right"),
        (111, "up", "up"),
        (116, "down", "down"),
        (67, "f1", "f1"),
        (96, "f12", "f12"),
    ] {
        load(&format!("ctrl+{chord}"));
        assert_eq!(
            app_key_from_native(native(code, Some('x'), modifiers)),
            Some(Keystroke {
                key: name.into(),
                key_char: None,
                modifiers,
            }),
            "{chord}"
        );
    }
}

#[test]
fn layout_characters_and_latin_fallbacks_keep_every_modifier() {
    let modifiers = Modifiers {
        control: true,
        shift: true,
        alt: true,
        platform: true,
        function: true,
    };
    for (code, plain, ascii_layout, chord, name) in [
        (38, Some('é'), true, "ctrl+shift+alt+cmd+é", "é"),
        (38, None, false, "ctrl+shift+alt+cmd+a", "a"),
        (38, Some('\u{1}'), false, "ctrl+shift+alt+cmd+a", "a"),
        (54, Some('с'), false, "ctrl+shift+alt+cmd+c", "с"),
    ] {
        load(chord);
        let mut key = native(code, plain, modifiers);
        key.ascii_layout = ascii_layout;
        assert_eq!(
            app_key_from_native(key),
            Some(Keystroke {
                key: name.into(),
                key_char: None,
                modifiers,
            })
        );
    }
}

#[test]
fn a_command_chord_matches_the_layouts_command_table() {
    load("cmd+d");
    let mut key = native(
        40,
        Some('e'),
        Modifiers {
            platform: true,
            ..Modifiers::default()
        },
    );
    key.command = Some('d');
    assert_eq!(app_key_from_native(key.clone()).unwrap().key, "e");
    key.command = Some('e');
    assert_eq!(app_key_from_native(key), None);
}

#[test]
fn typing_editing_unbound_keys_and_unowned_repeats_stay_in_the_page() {
    load("ctrl+x");
    for key in [
        native(38, Some('a'), Modifiers::default()),
        native(
            54,
            Some('c'),
            Modifiers {
                platform: true,
                ..Modifiers::default()
            },
        ),
        native(68, None, Modifiers::default()),
        native(9999, None, Modifiers::default()),
        native(9999, Some('\n'), Modifiers::default()),
        native(50, None, Modifiers::default()),
        NativeKey {
            repeat: true,
            ..native(
                53,
                Some('x'),
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
            )
        },
    ] {
        assert_eq!(app_key_from_native(key), None);
    }
    assert_eq!(app_key(0), None, "there is no AppKit event outside native dispatch");
}

#[test]
fn control_tab_is_the_window_switcher_even_without_a_binding() {
    load("ctrl+x");
    for shift in [false, true] {
        let modifiers = Modifiers {
            control: true,
            shift,
            ..Modifiers::default()
        };
        assert_eq!(
            app_key_from_native(native(23, None, modifiers)),
            Some(Keystroke {
                key: "tab".into(),
                key_char: None,
                modifiers,
            })
        );
    }
}
