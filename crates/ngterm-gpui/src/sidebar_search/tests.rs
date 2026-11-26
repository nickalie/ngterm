//! The search field's own keys; the field in a window (its chord, typing, Return, Escape, the clear button) is in
//! `window/tests.rs`.

use super::*;
use crate::terminal::input::Source;

fn key(source: &str) -> Option<FieldKey> {
    let keystroke = crate::parse_keystroke(source);
    field_key(&KeyStroke::new(&keystroke, &Source::Keystroke, false))
}

#[test]
fn return_opens_escape_dismisses_and_the_arrows_step() {
    assert_eq!(key("enter"), Some(FieldKey::Open));
    assert_eq!(key("shift-enter"), Some(FieldKey::Open));
    assert_eq!(key("escape"), Some(FieldKey::Dismiss));
    assert_eq!(key("down"), Some(FieldKey::Step(true)));
    assert_eq!(key("up"), Some(FieldKey::Step(false)));
}

#[test]
fn typing_and_chords_stay_the_fields_and_the_keymaps() {
    assert_eq!(key("a"), None);
    assert_eq!(key("left"), None);
    assert_eq!(key("ctrl-enter"), None);
    assert_eq!(key("alt-down"), None);
    assert_eq!(key("ctrl-alt-shift-f"), None);
}
