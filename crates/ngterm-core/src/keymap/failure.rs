//! A failed custom command's HUD text (upstream's `CommandFailure.swift`): the panel's first line from the command's
//! name and failure reason, and its detail from the tail of what the command wrote to stderr.
//!
//! The runner posts the panel without the control dispatcher's text validation, so every string here is already
//! stripped of escape sequences and control characters and capped at [`HudSpec::MAX_TEXT_LENGTH`].

use crate::hud::{HudSpec, text_length};
use crate::model::naming::sanitized;
use crate::terminal_text::stripping_escapes;

/// How much of stderr is retained, in bytes: only the last line is shown, and a command writing megabytes must not
/// grow the buffer with it.
pub const TAIL_LIMIT: usize = 16 * 1024;

/// The last usable line of a captured stderr tail, as HUD detail; `None` when nothing usable was written. Decoding
/// is lenient because a cut tail can split a scalar, and lines are cleaned before one is chosen, so a trailing bare
/// colour reset does not hide the diagnostic above it.
#[must_use]
pub fn detail(tail: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(tail);
    text.split(is_newline)
        .rev()
        .map(cleaned)
        .find(|line| !line.is_empty())
        .map(capped)
}

/// The panel's first line, `<name>: <reason>`. `reason` is the exit status of a command that ran or the launch
/// error of one that never started; a long name is truncated first, since the reason says what happened.
#[must_use]
pub fn message(name: &str, reason: &str) -> String {
    const SEPARATOR: &str = ": ";
    let reason = cleaned(reason);
    let used = text_length(&format!("{reason}{SEPARATOR}"));
    if used >= HudSpec::MAX_TEXT_LENGTH {
        return capped(reason);
    }
    let room = HudSpec::MAX_TEXT_LENGTH - used;
    let mut name = cleaned(name);
    while text_length(&name) > room && name.pop().is_some() {}
    if name.is_empty() {
        reason
    } else {
        format!("{name}{SEPARATOR}{reason}")
    }
}

fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Escape sequences dropped, then control characters, then surrounding blanks: an unstripped CSI would paint
/// outside the panel, and stripping C0 alone leaves the `[31m` an escape trails behind.
fn cleaned(text: &str) -> String {
    super::keybind::trim_blank(&sanitized(&stripping_escapes(text))).to_owned()
}

/// Truncates to what a HUD accepts, measured in [`text_length`]'s unit, one scalar at a time.
fn capped(mut text: String) -> String {
    while text_length(&text) > HudSpec::MAX_TEXT_LENGTH && text.pop().is_some() {}
    text
}
