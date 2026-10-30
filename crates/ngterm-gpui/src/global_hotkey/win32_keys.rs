//! The global hotkey's chord in `RegisterHotKey`'s terms, host-free so every OS tests it: the key's position and the
//! modifier flags.

use ngterm_core::keymap::{Modifiers, is_function_key};

/// `RegisterHotKey`'s modifier flags (`MOD_*`).
pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
/// A held key's autorepeat is no new press.
pub const MOD_NOREPEAT: u32 = 0x4000;

/// Where the chord's key is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPosition {
    /// A key whose virtual key no layout moves (Enter, Tab, Space, Backspace, the arrows, the function keys).
    Fixed(u16),
    /// A key of the main block by its scan code (set 1), the key the chord names on the US layout: its virtual key
    /// is the one the active layout puts there.
    Scan(u32),
}

/// The position of chord key `key`; `None` for a key with none (upstream's `keyCode(forChordKey:)`, as
/// [`super::keycode`]).
#[must_use]
pub fn position(key: &str) -> Option<KeyPosition> {
    let fixed = match key {
        "return" => Some(0x0D),
        "tab" => Some(0x09),
        "space" => Some(0x20),
        "delete" => Some(0x08),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        function if is_function_key(function) => function[1..].parse::<u16>().ok().map(|number| 0x6F + number),
        _ => None,
    };
    if let Some(vk) = fixed {
        return Some(KeyPosition::Fixed(vk));
    }
    // an XKB keycode of the main block is its scan code plus 8
    super::keycode(key).map(|keycode| KeyPosition::Scan(keycode - 8))
}

/// `mods` as `RegisterHotKey`'s flags (Command is the Windows key), autorepeat off.
#[must_use]
pub fn modifiers(mods: Modifiers) -> u32 {
    [
        (Modifiers::CONTROL, MOD_CONTROL),
        (Modifiers::OPTION, MOD_ALT),
        (Modifiers::SHIFT, MOD_SHIFT),
        (Modifiers::COMMAND, MOD_WIN),
    ]
    .into_iter()
    .filter(|(modifier, _)| mods.contains(*modifier))
    .fold(MOD_NOREPEAT, |flags, (_, flag)| flags | flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_key_names_a_scan_code_or_a_fixed_virtual_key() {
        assert_eq!(position("`"), Some(KeyPosition::Scan(0x29)), "the key left of 1");
        assert_eq!(position("q"), Some(KeyPosition::Scan(0x10)));
        assert_eq!(position("1"), Some(KeyPosition::Scan(0x02)));
        assert_eq!(position("\\"), Some(KeyPosition::Scan(0x2B)));
        assert_eq!(position("/"), Some(KeyPosition::Scan(0x35)));
        assert_eq!(position("return"), Some(KeyPosition::Fixed(0x0D)));
        assert_eq!(position("delete"), Some(KeyPosition::Fixed(0x08)), "Backspace");
        assert_eq!(position("space"), Some(KeyPosition::Fixed(0x20)));
        assert_eq!(position("f1"), Some(KeyPosition::Fixed(0x70)));
        assert_eq!(position("f20"), Some(KeyPosition::Fixed(0x83)));
        assert_eq!(position("up"), Some(KeyPosition::Fixed(0x26)));
        assert_eq!(position("é"), None);
    }

    #[test]
    fn the_modifiers_read_as_register_hot_key_flags_without_autorepeat() {
        assert_eq!(modifiers(Modifiers::empty()), MOD_NOREPEAT);
        assert_eq!(
            modifiers(Modifiers::CONTROL | Modifiers::OPTION | Modifiers::SHIFT | Modifiers::COMMAND),
            MOD_NOREPEAT | MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_WIN
        );
    }
}
