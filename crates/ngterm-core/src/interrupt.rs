//! How typed input clears a session's agent status (upstream's `InterruptKeystroke.swift`): a keystroke or injected
//! text classified as an interrupt (Escape, a bare Ctrl-C), a submit (Return, or text carrying a newline) or plain
//! typing, for [`AgentIndicator::cleared_by`](crate::model::AgentIndicator::cleared_by).
//!
//! Upstream matches macOS virtual key codes; here a frontend names its key by Ghostty's physical key name and its
//! held [`Mods`], and [`classify_named`] maps them onto [`Key`] and [`KeyModifiers`], so the truth table stays
//! host-free and one for every frontend: `ngterm-gpui`'s `terminal::input::status_keystroke`, reported per terminal key
//! press and applied by [`Core::clear_status_on_keystroke`], and `ngterm-mobile`'s phone keys, which tell an origin.
//!
//! [`Core::clear_status_on_keystroke`]: crate::Core::clear_status_on_keystroke

use std::ops::BitOr;

use crate::ghostty_config::binds::Mods;
use crate::model::StatusKeystroke;

/// The keyboard modifiers the classification reads (upstream's `KeyModifiers`): a small set of flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct KeyModifiers(u8);

impl KeyModifiers {
    pub const NONE: Self = Self(0);
    pub const CONTROL: Self = Self(1);
    /// Command on macOS; the Super key here.
    pub const COMMAND: Self = Self(1 << 1);
    /// Option on macOS; Alt here.
    pub const OPTION: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);

    /// Whether every flag of `other` is set.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether no flag is set.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for KeyModifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Upstream's `KeyModifiers` of Ghostty's (Super standing for ⌘); the lock keys are dropped.
impl From<Mods> for KeyModifiers {
    fn from(mods: Mods) -> Self {
        [
            (Mods::CTRL, Self::CONTROL),
            (Mods::SUPER, Self::COMMAND),
            (Mods::ALT, Self::OPTION),
            (Mods::SHIFT, Self::SHIFT),
        ]
        .into_iter()
        .filter(|(held, _)| mods.contains(*held))
        .fold(Self::NONE, |modifiers, (_, bit)| modifiers | bit)
    }
}

/// The physical key of a keystroke, layout-independent (upstream's macOS key codes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Return,
    KeypadEnter,
    /// The key in the QWERTY C position, whatever letter the layout puts there.
    C,
    /// Any other key.
    Other,
}

impl Key {
    /// The key Ghostty's physical key `name` is (`enter`, `key_c`, ...).
    #[must_use]
    pub fn named(name: &str) -> Self {
        match name {
            "escape" => Self::Escape,
            "enter" => Self::Return,
            "numpad_enter" => Self::KeypadEnter,
            "key_c" => Self::C,
            _ => Self::Other,
        }
    }
}

/// [`classify`] of a keystroke as a frontend reports it: Ghostty's physical key `name`, the text the key types before
/// Ctrl transformations (empty for none) and the held modifiers.
#[must_use]
pub fn classify_named(name: &str, character: Option<&str>, mods: Mods) -> StatusKeystroke {
    classify(Key::named(name), character.filter(|text| !text.is_empty()), mods.into())
}

/// What a keystroke means to a status glyph: an interrupt first, then a submit, else plain typing.
#[must_use]
pub fn classify(key: Key, character: Option<&str>, modifiers: KeyModifiers) -> StatusKeystroke {
    if is_interrupt(key, character, modifiers) {
        StatusKeystroke::Interrupt
    } else if is_submit(key, modifiers) {
        StatusKeystroke::Submit
    } else {
        StatusKeystroke::Other
    }
}

/// What injected text (`session.type`) means to a status glyph: a line feed or carriage return anywhere submits,
/// since the injector types Return for each; anything else, Unicode line separators included, is plain typing. Never
/// an interrupt.
#[must_use]
pub fn classify_text(text: &str) -> StatusKeystroke {
    if text.contains(['\n', '\r']) {
        StatusKeystroke::Submit
    } else {
        StatusKeystroke::Other
    }
}

/// Whether the keystroke submits the line: Return or keypad Enter with NO modifier, since Shift-Return and Alt-Return
/// insert a newline in Claude Code and Codex.
#[must_use]
pub fn is_submit(key: Key, modifiers: KeyModifiers) -> bool {
    matches!(key, Key::Return | Key::KeypadEnter) && modifiers.is_empty()
}

/// Whether the keystroke interrupts the agent. Escape does under any modifiers. Ctrl-C must be bare (control only),
/// so a copy chord like Ctrl-Shift-C cannot clear a glyph mid-work; it matches the layout's base letter `character`
/// (Latin layouts, Dvorak included), and the physical C key when that key produces no Latin letter (Cyrillic, Greek,
/// or no character at all), so a remapped Latin layout's letter there (Dvorak's `j`) is no false interrupt.
#[must_use]
pub fn is_interrupt(key: Key, character: Option<&str>, modifiers: KeyModifiers) -> bool {
    if key == Key::Escape {
        return true;
    }
    if modifiers != KeyModifiers::CONTROL {
        return false;
    }
    let base = character.map(str::to_lowercase);
    if base.as_deref() == Some("c") {
        return true;
    }
    key == Key::C && !is_latin_letter(base.as_deref())
}

/// Whether `base` is a single lower-case basic-Latin letter.
fn is_latin_letter(base: Option<&str>) -> bool {
    let mut chars = base.unwrap_or_default().chars();
    matches!((chars.next(), chars.next()), (Some('a'..='z'), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn a_named_keystroke_classifies_by_its_ghostty_key_and_mods() {
        assert_eq!(
            classify_named("enter", Some("\r"), Mods::empty()),
            StatusKeystroke::Submit
        );
        assert_eq!(
            classify_named("numpad_enter", None, Mods::CAPS_LOCK),
            StatusKeystroke::Submit
        );
        assert_eq!(classify_named("enter", None, Mods::SHIFT), StatusKeystroke::Other);
        assert_eq!(
            classify_named("key_c", Some(""), Mods::CTRL),
            StatusKeystroke::Interrupt
        );
        assert_eq!(
            classify_named("key_j", Some("c"), Mods::CTRL),
            StatusKeystroke::Interrupt
        );
        assert_eq!(classify_named("key_c", Some("j"), Mods::CTRL), StatusKeystroke::Other);
        assert_eq!(
            classify_named("key_c", Some("c"), Mods::CTRL | Mods::SHIFT),
            StatusKeystroke::Other
        );
        assert_eq!(classify_named("escape", None, Mods::SUPER), StatusKeystroke::Interrupt);
        assert_eq!(
            classify_named("key_a", Some("a"), Mods::empty()),
            StatusKeystroke::Other
        );
    }

    #[test]
    fn ghostty_mods_map_onto_upstreams_modifiers_without_the_locks() {
        let all = Mods::CTRL | Mods::SUPER | Mods::ALT | Mods::SHIFT | Mods::CAPS_LOCK | Mods::NUM_LOCK;
        assert_eq!(
            KeyModifiers::from(all),
            KeyModifiers::CONTROL | KeyModifiers::COMMAND | KeyModifiers::OPTION | KeyModifiers::SHIFT
        );
        assert!(KeyModifiers::from(Mods::CAPS_LOCK | Mods::NUM_LOCK).is_empty());
    }

    #[test]
    fn escape_interrupts_under_any_modifiers() {
        assert!(is_interrupt(Key::Escape, Some("\u{1b}"), KeyModifiers::NONE));
        assert!(is_interrupt(Key::Escape, Some("\u{1b}"), CTRL));
        assert!(is_interrupt(Key::Escape, Some("\u{1b}"), KeyModifiers::COMMAND));
    }

    #[test]
    fn bare_control_c_interrupts() {
        assert!(is_interrupt(Key::C, Some("c"), CTRL));
        // dvorak: the C letter sits on another physical key, caught by the character check
        assert!(is_interrupt(Key::Other, Some("c"), CTRL));
        // cyrillic: the physical C key produces "с" (U+0441), caught by the physical-key fallback
        assert!(is_interrupt(Key::C, Some("с"), CTRL));
        // no base character: trust the physical key
        assert!(is_interrupt(Key::C, None, CTRL));
    }

    #[test]
    fn dvorak_control_j_does_not_interrupt() {
        assert!(!is_interrupt(Key::C, Some("j"), CTRL));
    }

    #[test]
    fn return_without_modifiers_submits() {
        assert!(is_submit(Key::Return, KeyModifiers::NONE));
        assert!(is_submit(Key::KeypadEnter, KeyModifiers::NONE));
        assert!(!is_submit(Key::Return, KeyModifiers::SHIFT));
        assert!(!is_submit(Key::Return, KeyModifiers::OPTION));
        assert!(!is_submit(Key::Return, KeyModifiers::COMMAND));
        assert!(!is_submit(Key::Other, KeyModifiers::NONE));
    }

    #[test]
    fn classify_orders_interrupt_before_submit_before_other() {
        assert_eq!(
            classify(Key::Escape, Some("\u{1b}"), KeyModifiers::NONE),
            StatusKeystroke::Interrupt
        );
        assert_eq!(classify(Key::C, Some("c"), CTRL), StatusKeystroke::Interrupt);
        assert_eq!(
            classify(Key::Return, Some("\r"), KeyModifiers::NONE),
            StatusKeystroke::Submit
        );
        assert_eq!(
            classify(Key::Return, Some("\r"), KeyModifiers::SHIFT),
            StatusKeystroke::Other
        );
        assert_eq!(
            classify(Key::Other, Some("a"), KeyModifiers::NONE),
            StatusKeystroke::Other
        );
    }

    #[test]
    fn classify_text_submits_only_with_a_newline() {
        assert_eq!(classify_text("a"), StatusKeystroke::Other);
        assert_eq!(classify_text("yes\n"), StatusKeystroke::Submit);
        assert_eq!(classify_text("\r"), StatusKeystroke::Submit);
        assert_eq!(classify_text("yes\r\n"), StatusKeystroke::Submit);
        assert_eq!(classify_text("\r\n"), StatusKeystroke::Submit);
        assert_eq!(classify_text("a\u{2028}b"), StatusKeystroke::Other);
        assert_eq!(classify_text("\u{1b}"), StatusKeystroke::Other);
    }

    #[test]
    fn non_interrupt_keystrokes_do_not_clear() {
        assert!(!is_interrupt(Key::C, Some("c"), KeyModifiers::NONE));
        assert!(!is_interrupt(Key::Other, Some("a"), KeyModifiers::NONE));
        assert!(!is_interrupt(Key::Other, Some("d"), CTRL));
        assert!(!is_interrupt(Key::C, Some("c"), KeyModifiers::COMMAND));
        assert!(!is_interrupt(Key::C, Some("c"), KeyModifiers::OPTION));
        assert!(!is_interrupt(Key::C, Some("c"), CTRL | KeyModifiers::COMMAND));
        // ctrl-shift-c: the base reads "C"; shift is excluded so it must not clear
        assert!(!is_interrupt(Key::C, Some("C"), CTRL | KeyModifiers::SHIFT));
    }

    #[test]
    fn key_modifiers_set() {
        let mods = CTRL | KeyModifiers::SHIFT;
        assert!(mods.contains(KeyModifiers::CONTROL));
        assert!(mods.contains(KeyModifiers::SHIFT));
        assert!(!mods.contains(KeyModifiers::COMMAND));
        assert!(!mods.contains(KeyModifiers::OPTION));
        assert!(KeyModifiers::NONE.is_empty());
    }
}
