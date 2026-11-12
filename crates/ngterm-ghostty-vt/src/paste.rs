//! Pasting into a terminal (`paste.h`): the terminal frames the text for its current modes (bracketed paste 2004,
//! line endings as carriage returns otherwise, unsafe control bytes replaced) and refuses text that could run a
//! command unless the paste allows it; or, with Kitty paste events on (mode 5522), it sends the program an event
//! instead. The bytes go out through the installed [`Effects::write_pty`](crate::Effects::write_pty).

use std::ffi::c_void;

use crate::effects::{ClipboardContent, ClipboardLocation};
use crate::{Error, Terminal, ffi};

/// Why a paste happens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PasteSource {
    /// The user pasted from a clipboard (a key binding, a menu, a middle click).
    #[default]
    Clipboard,
    /// Text inserted another way (an IME commit, a drop, scripted input): always written as text.
    Text,
}

/// What a paste did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PasteOutcome {
    /// Bytes went to the program (the framed text, or a paste event).
    Written,
    /// There was nothing to paste (empty text).
    Empty,
    /// The text could run a command (a line ending outside bracketed paste, or the bracketed-paste terminator) and
    /// the paste did not allow it; nothing was written. Confirm with the user, then paste again allowing it.
    Unsafe,
}

/// Supplies the requested representation, borrowing all bytes only for this call.
unsafe extern "C" fn read_content(userdata: *mut c_void, mime: ffi::GhosttyString, writer: ffi::GhosttyWriter) -> bool {
    // SAFETY: userdata borrows the slice passed to `paste_contents`, alive across the C call.
    let contents = unsafe { &*userdata.cast::<&[ClipboardContent]>() };
    // SAFETY: the library borrows the MIME string for the callback.
    let mime = unsafe { crate::string_bytes(mime) };
    let Some(content) = contents.iter().find(|content| content.mime.as_bytes() == mime) else {
        return false;
    };
    let Some(write) = writer.write else {
        return false;
    };
    // SAFETY: writer and the borrowed content bytes are valid throughout the callback.
    content.data.is_empty() || unsafe { write(writer.userdata, content.data.as_ptr(), content.data.len()) }
}

impl Terminal {
    /// Paste `text` per the terminal's state; the bytes stream through [`Effects::write_pty`](crate::Effects).
    ///
    /// # Errors
    ///
    /// [`Error::InvalidValue`] when no effects are installed (nothing to write to), or the library's result when the
    /// paste fails (nothing is written then).
    pub fn paste(
        &mut self,
        text: &str,
        source: PasteSource,
        location: ClipboardLocation,
        allow_unsafe: bool,
    ) -> Result<PasteOutcome, Error> {
        self.paste_contents(
            &[ClipboardContent {
                mime: "text/plain".to_owned(),
                data: text.as_bytes().to_vec(),
            }],
            source,
            location,
            allow_unsafe,
        )
    }

    /// Paste a clipboard's real representations. Mode 5522 advertises the MIME list and grants
    /// its follow-up read; a text-source paste still inserts text through the library's framing.
    ///
    /// # Errors
    ///
    /// The same errors as [`Self::paste`]. No representation is retained after this call.
    pub fn paste_contents(
        &mut self,
        contents: &[ClipboardContent],
        source: PasteSource,
        location: ClipboardLocation,
        allow_unsafe: bool,
    ) -> Result<PasteOutcome, Error> {
        let mimes: Vec<_> = contents
            .iter()
            .take(16)
            .map(|content| ffi::GhosttyString {
                ptr: content.mime.as_ptr(),
                len: content.mime.len(),
            })
            .collect();
        let mut contents = contents;
        let paste = ffi::GhosttyPaste {
            size: std::mem::size_of::<ffi::GhosttyPaste>(),
            location: location.raw(),
            source: match source {
                PasteSource::Clipboard => ffi::GHOSTTY_PASTE_SOURCE_CLIPBOARD,
                PasteSource::Text => ffi::GHOSTTY_PASTE_SOURCE_TEXT,
            },
            mimes: mimes.as_ptr(),
            mimes_len: mimes.len(),
            reader: ffi::GhosttyMimeReader {
                read: Some(read_content),
                userdata: (&raw mut contents).cast(),
            },
            allow_unsafe,
        };
        let mut written = false;
        // SAFETY: the terminal is live and `&mut self` serializes the call; the paste, its MIME list and the text the
        // reader's userdata points at all outlive the call, which borrows them only for its duration.
        let result = unsafe { ffi::ghostty_terminal_paste(self.as_raw(), &raw const paste, &raw mut written) };
        match Error::check(result) {
            Ok(()) if written => Ok(PasteOutcome::Written),
            Ok(()) => Ok(PasteOutcome::Empty),
            Err(Error::Rejected) => Ok(PasteOutcome::Unsafe),
            Err(err) => Err(err),
        }
    }
}

/// Whether `text` is safe to paste whatever the terminal's state (no line ending, no bracketed-paste terminator).
#[must_use]
pub fn paste_is_safe(text: &str) -> bool {
    // SAFETY: the pointer and length describe the borrowed string for the call.
    unsafe { ffi::ghostty_paste_is_safe(text.as_ptr().cast(), text.len()) }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::Effects;

    /// Collects what the terminal writes to the program.
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Effects for Sink {
        fn write_pty(&mut self, data: &[u8]) {
            self.0.lock().unwrap().extend_from_slice(data);
        }
    }

    fn terminal() -> (Terminal, Arc<Mutex<Vec<u8>>>) {
        let mut terminal = Terminal::new(80, 24).unwrap();
        let out = Arc::new(Mutex::new(Vec::new()));
        terminal.set_effects(Box::new(Sink(Arc::clone(&out)))).unwrap();
        (terminal, out)
    }

    fn take(out: &Arc<Mutex<Vec<u8>>>) -> Vec<u8> {
        std::mem::take(&mut *out.lock().unwrap())
    }

    #[test]
    fn a_plain_paste_turns_newlines_into_returns_only_when_allowed() {
        let (mut terminal, out) = terminal();
        let paste = |terminal: &mut Terminal, text, allow| {
            terminal
                .paste(text, PasteSource::Clipboard, ClipboardLocation::Standard, allow)
                .unwrap()
        };
        assert_eq!(paste(&mut terminal, "echo hi", false), PasteOutcome::Written);
        assert_eq!(take(&out), b"echo hi");
        assert_eq!(paste(&mut terminal, "ls\nrm", false), PasteOutcome::Unsafe);
        assert!(take(&out).is_empty());
        assert_eq!(paste(&mut terminal, "ls\nrm", true), PasteOutcome::Written);
        assert_eq!(take(&out), b"ls\rrm");
        assert_eq!(paste(&mut terminal, "", false), PasteOutcome::Empty);
    }

    #[test]
    fn a_bracketed_paste_is_wrapped_and_its_newlines_are_safe() {
        let (mut terminal, out) = terminal();
        terminal.vt_write(b"\x1b[?2004h");
        let outcome = terminal
            .paste("a\nb\x1bc", PasteSource::Clipboard, ClipboardLocation::Standard, false)
            .unwrap();
        assert_eq!(outcome, PasteOutcome::Written);
        assert_eq!(take(&out), b"\x1b[200~a\nb c\x1b[201~");
        let terminator = terminal
            .paste(
                "x\x1b[201~y",
                PasteSource::Clipboard,
                ClipboardLocation::Standard,
                false,
            )
            .unwrap();
        assert_eq!(terminator, PasteOutcome::Unsafe);
    }

    #[test]
    fn a_paste_with_nowhere_to_write_fails() {
        let mut terminal = Terminal::new(80, 24).unwrap();
        assert_eq!(
            terminal.paste("x", PasteSource::Text, ClipboardLocation::Standard, false),
            Err(Error::InvalidValue)
        );
    }

    #[test]
    fn safety_is_conservative() {
        assert!(paste_is_safe("plain"));
        assert!(!paste_is_safe("a\nb"));
        assert!(!paste_is_safe("a\rb"));
    }
}
