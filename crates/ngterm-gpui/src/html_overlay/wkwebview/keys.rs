//! App chords are checked before `WKWebView` editing; typing and its editing shortcuts stay in `WebKit`.
use crate::terminal::input::{self, KeyStroke, Source};
use gpui_kit::{Keystroke, Modifiers};
pub fn app_key(_: u32) -> Option<Keystroke> {
    let mut keystroke = Keystroke {
        key: String::new(),
        key_char: None,
        modifiers: Modifiers::default(),
    };
    let Source::Native(native) = input::current_source(&keystroke) else {
        return None;
    };
    keystroke.modifiers = native.modifiers;
    keystroke.key = input::named_key(input::physical_key(native.keycode))
        .map(str::to_owned)
        .or_else(|| native.unshifted.filter(|ch| !ch.is_control()).map(|ch| ch.to_string()))
        .or_else(|| input::latin_key(native.keycode).map(str::to_owned))?;
    keystroke.key = match keystroke.key.as_str() {
        "return" => "enter".into(),
        "delete" => "backspace".into(),
        _ => keystroke.key,
    };
    let stroke = KeyStroke::new(&keystroke, &Source::Native(native), false);
    crate::keymap_dispatch::takes(&stroke).then_some(keystroke)
}
