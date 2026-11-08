//! App chords are checked before `WKWebView` editing; typing and its editing shortcuts stay in `WebKit`.
use crate::terminal::input::{self, KeyStroke, NativeKey, Source};
use gpui_kit::Keystroke;
pub fn app_key(_: u32) -> Option<Keystroke> {
    let Source::Native(native) = input::current_source(&Keystroke::default()) else {
        return None;
    };
    app_key_from_native(native)
}

fn app_key_from_native(native: NativeKey) -> Option<Keystroke> {
    let keystroke = Keystroke {
        key: key_name(&native)?,
        key_char: None,
        modifiers: native.modifiers,
    };
    let stroke = KeyStroke::new(&keystroke, &Source::Native(native), false);
    crate::keymap_dispatch::takes(&stroke).then_some(keystroke)
}

/// GPUI's name of the key: named by its position, else what it types, else its Latin position (Esc, which no chord
/// names and which types only a control character, is still the keymap's to cancel a leader with).
fn key_name(native: &NativeKey) -> Option<String> {
    let physical = input::physical_key(native.keycode);
    if physical.name() == "escape" {
        return Some("escape".into());
    }
    let key = input::named_key(physical)
        .map(str::to_owned)
        .or_else(|| native.unshifted.filter(|ch| !ch.is_control()).map(|ch| ch.to_string()))
        .or_else(|| input::latin_key(native.keycode).map(str::to_owned))?;
    Some(match key.as_str() {
        "return" => "enter".into(),
        "delete" => "backspace".into(),
        _ => key,
    })
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
