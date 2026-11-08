//! The native adapter's packed key output as the terminal and application keymap read it.

use gpui_kit::{Keystroke, Modifiers};
use ngterm_core::ghostty_config::binds::Mods;
use ngterm_term::ModSides;

use super::{NativeKey, Source, current_source, decode_macos_key};

fn native(flags: u32) -> Option<NativeKey> {
    let Source::Native(key) = decode_macos_key(0, flags, u32::from('a'), u32::from('q'), c"a", c"a") else {
        return None;
    };
    Some(key)
}

#[test]
fn native_keycodes_preserve_the_hardware_position_and_unknown_codes_fall_back() {
    for (code, xkb) in [(0, 38), (8, 54), (10, 94), (36, 36), (76, 104), (123, 113), (126, 111)] {
        let Source::Native(key) = decode_macos_key(code, 0, 0, 0, c"", c"") else {
            panic!("missing native key {code}");
        };
        assert_eq!(key.keycode, xkb, "{code}");
    }
    for code in [i32::MIN, -1, 52, 127, i32::MAX] {
        assert_eq!(decode_macos_key(code, 0, 0, 0, c"", c""), Source::Keystroke, "{code}");
    }
}

#[test]
fn held_modifiers_and_layout_consumption_are_decoded_independently() {
    let cases = [
        (0, Modifiers::default()),
        (
            1 << 17,
            Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        ),
        (
            1 << 18,
            Modifiers {
                control: true,
                ..Modifiers::default()
            },
        ),
        (
            1 << 19,
            Modifiers {
                alt: true,
                ..Modifiers::default()
            },
        ),
        (
            1 << 20,
            Modifiers {
                platform: true,
                ..Modifiers::default()
            },
        ),
        (
            1 << 23,
            Modifiers {
                function: true,
                ..Modifiers::default()
            },
        ),
        (
            0x9e_0000,
            Modifiers {
                shift: true,
                control: true,
                alt: true,
                platform: true,
                function: true,
            },
        ),
    ];
    for (flags, modifiers) in cases {
        let key = native(flags).unwrap();
        assert_eq!(key.modifiers, modifiers, "{flags:#x}");
        assert_eq!(
            key.consumed,
            Modifiers {
                shift: modifiers.shift,
                alt: modifiers.alt,
                ..Modifiers::default()
            }
        );
        assert_eq!(key.command, modifiers.platform.then_some('q'));
        assert_eq!(key.unshifted, Some('a'));
        assert_eq!(key.text, "a");
        assert_eq!(key.without_option.as_deref(), Some("a"));
    }
}

#[test]
fn packed_state_and_right_hand_modifiers_do_not_alias_appkit_flags() {
    let cases = [
        (0, false, false, false, Mods::empty(), ModSides::empty()),
        (1, true, false, false, Mods::empty(), ModSides::empty()),
        (2, false, true, false, Mods::empty(), ModSides::empty()),
        (4, false, false, true, Mods::empty(), ModSides::empty()),
        (8, false, false, false, Mods::empty(), ModSides::ALT),
        (16, false, false, false, Mods::empty(), ModSides::SHIFT),
        (32, false, false, false, Mods::empty(), ModSides::CTRL),
        (64, false, false, false, Mods::empty(), ModSides::SUPER),
        (1 << 16, false, false, false, Mods::CAPS_LOCK, ModSides::empty()),
        (
            0x1_007f,
            true,
            true,
            true,
            Mods::CAPS_LOCK,
            ModSides::ALT | ModSides::SHIFT | ModSides::CTRL | ModSides::SUPER,
        ),
        (1 << 31, false, false, false, Mods::empty(), ModSides::empty()),
    ];
    for (flags, repeat, ascii_layout, composing, locks, sides) in cases {
        assert_eq!(
            native(flags).unwrap(),
            NativeKey {
                keycode: 38,
                repeat,
                ascii_layout,
                composing,
                locks,
                sides,
                unshifted: Some('a'),
                text: "a".into(),
                without_option: Some("a".into()),
                ..NativeKey::default()
            },
            "{flags:#x}"
        );
    }
}

#[test]
fn layout_characters_and_command_table_keep_unicode_and_reject_invalid_scalars() {
    for (plain, expected) in [
        (0, None),
        (0xd800, None),
        (0x11_0000, None),
        (0x10400, Some('𐐀')),
        (0x0105, Some('ą')),
    ] {
        for (flags, command, expected_command) in [
            (0, u32::from('1'), None),
            (1 << 20, u32::from('1'), Some('1')),
            (1 << 20, 0xdfff, None),
            (1 << 20, 0x11_0000, None),
        ] {
            let Source::Native(key) = decode_macos_key(18, flags, plain, command, c"Ą", c"ą") else {
                panic!("the number row key has a native record");
            };
            assert_eq!(key.unshifted, expected);
            assert_eq!(key.command, expected_command);
            assert_eq!(key.text, "Ą");
            assert_eq!(key.without_option.as_deref(), Some("ą"));
        }
    }
}

#[test]
fn typed_and_option_free_text_are_lossy_utf8_and_stop_at_nul() {
    let invalid = c"\xff";
    for (text, without_option, expected_text, expected_without) in [
        (c"", c"", "", ""),
        (c"´", c"e", "´", "e"),
        (c"你好", c"é", "你好", "é"),
        (invalid, c"a", "�", "a"),
        (c"a", invalid, "a", "�"),
        (
            std::ffi::CStr::from_bytes_until_nul(b"a\0ignored").unwrap(),
            c"a",
            "a",
            "a",
        ),
    ] {
        let Source::Native(key) = decode_macos_key(0, 0, 0, 0, text, without_option) else {
            panic!("the A key has a native record");
        };
        assert_eq!(key.text, expected_text);
        assert_eq!(key.without_option.as_deref(), Some(expected_without));
    }
}

#[test]
fn outside_appkit_dispatch_a_keystroke_has_no_native_event() {
    assert_eq!(current_source(&Keystroke::default()), Source::Keystroke);
}
