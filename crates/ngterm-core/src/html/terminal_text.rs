//! Text a terminal program wrote, read as the text it shows (upstream's `TerminalText.strippingEscapes`): a log page's
//! diff headers and a failed custom command's panel drop the escape sequences that `text.js` paints or ignores.

use std::borrow::Cow;

const ESC: char = '\u{1b}';
const BEL: char = '\u{07}';

/// `text` without its escape sequences: CSI (`ESC [` up to a final byte `@`-`~`), OSC (`ESC ]` up to BEL or `ESC \`),
/// DCS, SOS, PM and APC strings (up to `ESC \` only, over the doubled ESCs a tmux passthrough puts inside), the
/// intermediate forms (`ESC (` up to a final byte, as in the `ESC ( B` that `tput sgr0` writes) and the
/// two-character forms. A partial sequence at the end goes with it, since a cut tail can end mid-escape. This is the
/// grammar by which `text.js` reads a page's colors, so a colored diff's headers read the same on both sides.
#[must_use]
pub fn stripping_escapes(text: &str) -> Cow<'_, str> {
    if !text.contains(ESC) {
        return Cow::Borrowed(text);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while let Some(&c) = chars.get(index) {
        if c != ESC {
            result.push(c);
            index += 1;
            continue;
        }
        let Some(&kind) = chars.get(index + 1) else {
            break;
        };
        index = sequence_end(&chars, index + 2, kind);
    }
    Cow::Owned(result)
}

/// The index past a sequence of `kind` (the character after its ESC) whose body starts at `index`; it can be past the
/// end of `chars`.
fn sequence_end(chars: &[char], mut index: usize, kind: char) -> usize {
    let at = |index: usize| chars.get(index).copied();
    match kind {
        '[' => {
            while at(index).is_some_and(|c| !('\u{40}'..='\u{7e}').contains(&c)) {
                index += 1;
            }
            index + 1
        }
        ']' => {
            while at(index).is_some_and(|c| c != BEL && c != ESC) {
                index += 1;
            }
            if at(index) == Some(ESC) {
                index += 1;
            }
            index + 1
        }
        'P' | 'X' | '^' | '_' => {
            while let Some(c) = at(index) {
                let next = at(index + 1);
                if c == ESC && next == Some('\\') {
                    break;
                }
                index += if c == ESC && next == Some(ESC) { 2 } else { 1 };
            }
            index + 2
        }
        ' '..='/' => {
            while at(index).is_some_and(|c| (' '..='/').contains(&c)) {
                index += 1;
            }
            index + 1
        }
        _ => index,
    }
}

#[cfg(test)]
#[path = "terminal_text_tests.rs"]
mod tests;
