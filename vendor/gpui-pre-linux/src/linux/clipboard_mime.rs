//! Per-MIME clipboard access (an ngterm patch): GPUI's `ClipboardItem` carries text, images and paths, while the
//! Kitty clipboard protocol (OSC 5522) reads and writes any MIME type (Ghostty's GTK `Surface.set` offers one content
//! provider per representation; its Kitty reads fetch each requested type and list the advertised ones).
//!
//! - [`write_clipboard_representations`] owns the clipboard (or the primary selection) with exactly the given
//!   representations: `text/plain` is offered as `text/plain;charset=utf-8` first, then the bare and X11 names
//!   (Ghostty: some clients take the first match and garble UTF-8 offered as bare `text/plain`); any other type under
//!   its own name. No representation clears the selection.
//! - [`read_clipboard_mimes`] reads each requested type the owner offers, in request order, skipping the rest; a text
//!   type is read from whichever text target the owner offers (Ghostty reads text through GDK's text conversions).
//! - [`clipboard_mime_types`] lists the advertised MIME types without reading any data.
//!
//! They reach the platform on this thread (the main thread) through the one `linux_platform` registered, and fail
//! with [`ClipboardMimeError::Unsupported`] while headless or without that selection.

use std::cell::RefCell;
use std::rc::Weak;

/// One representation of a clipboard value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardRepresentation {
    pub mime: String,
    pub data: Vec<u8>,
}

/// Which selection a per-MIME access targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    /// The clipboard (`CLIPBOARD`, `wl_data_device`).
    Clipboard,
    /// The primary selection (`PRIMARY`, `zwp_primary_selection`).
    Primary,
}

/// Why a per-MIME access did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardMimeError {
    /// No display connection, or the display server has no such selection.
    Unsupported,
    /// The display server refused or the transfer failed.
    Io(String),
}

/// The text types Ghostty serves through text conversion (`terminal.clipboard.isTextMime`).
#[must_use]
pub fn is_text_mime(mime: &str) -> bool {
    matches!(
        mime,
        "text/plain" | "text/plain;charset=utf-8" | "UTF8_STRING" | "TEXT" | "STRING"
    )
}

/// The names a `text/plain` representation is offered under, UTF-8 first.
pub(crate) const TEXT_OFFERS: [&str; 4] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "text/plain;charset=UTF-8",
];

/// The types `representations` are offered under, in order and without repeats.
#[must_use]
pub(crate) fn offered_mimes(representations: &[ClipboardRepresentation]) -> Vec<String> {
    let mut offered: Vec<String> = Vec::new();
    for representation in representations {
        let names: Vec<&str> = if representation.mime == "text/plain" {
            TEXT_OFFERS.to_vec()
        } else {
            vec![representation.mime.as_str()]
        };
        for name in names {
            if !offered.iter().any(|known| known == name) {
                offered.push(name.to_owned());
            }
        }
    }
    offered
}

/// The bytes served for a request of `mime`: its own representation, else, for a text name, the `text/plain` one.
#[must_use]
pub(crate) fn served<'a>(representations: &'a [ClipboardRepresentation], mime: &str) -> Option<&'a [u8]> {
    representations
        .iter()
        .find(|representation| representation.mime == mime)
        .or_else(|| {
            (is_text_mime(mime) || TEXT_OFFERS.contains(&mime))
                .then(|| {
                    representations
                        .iter()
                        .find(|representation| representation.mime == "text/plain")
                })
                .flatten()
        })
        .map(|representation| representation.data.as_slice())
}

/// The owner's type to read a request of `mime` from among `offered`: the type itself, or for a text type the first
/// text target offered (UTF-8 ones first).
#[must_use]
pub(crate) fn source_type<'a>(offered: &'a [String], mime: &str) -> Option<&'a str> {
    if let Some(exact) = offered.iter().find(|offer| *offer == mime) {
        return Some(exact);
    }
    if !is_text_mime(mime) {
        return None;
    }
    [
        "text/plain;charset=utf-8",
        "text/plain;charset=UTF-8",
        "UTF8_STRING",
        "text/plain",
        "STRING",
        "TEXT",
    ]
    .into_iter()
    .find_map(|text| offered.iter().find(|offer| *offer == text))
    .map(String::as_str)
}

/// The bytes read from the owner's `source` type as served for a request: X11's `STRING` target holds ISO Latin-1,
/// converted to UTF-8 as GPUI's own text read does (Ghostty reads every text type through GDK's UTF-8 conversion).
#[must_use]
pub(crate) fn source_data(source: &str, data: Vec<u8>) -> Vec<u8> {
    if source == "STRING" {
        data.into_iter().map(char::from).collect::<String>().into_bytes()
    } else {
        data
    }
}

/// The advertised MIME types worth listing: MIME-shaped names (no X11 target names, no GPUI's own `pid/` marker),
/// with one UTF-8 text type standing in for X11's text-only targets and the charset's spellings.
#[must_use]
pub(crate) fn listed_mimes(offered: &[String], self_mime: Option<&str>) -> Vec<String> {
    let mut listed: Vec<String> = Vec::new();
    for offer in offered {
        if Some(offer.as_str()) == self_mime || offer.starts_with("pid/") {
            continue;
        }
        let name = if offer.eq_ignore_ascii_case("text/plain;charset=utf-8") {
            "text/plain;charset=utf-8"
        } else if offer.contains('/') {
            offer.as_str()
        } else if is_text_mime(offer) {
            "text/plain;charset=utf-8"
        } else {
            continue;
        };
        if !listed.iter().any(|known| known == name) {
            listed.push(name.to_owned());
        }
    }
    listed
}

thread_local! {
    static PLATFORM: RefCell<Weak<super::LinuxPlatform>> = const { RefCell::new(Weak::new()) };
}

/// Makes `platform` the one the per-MIME functions reach on this thread.
pub(crate) fn register(platform: &std::rc::Rc<super::LinuxPlatform>) {
    PLATFORM.set(std::rc::Rc::downgrade(platform));
}

fn with_platform<R>(
    access: impl FnOnce(&super::LinuxPlatform) -> Result<R, ClipboardMimeError>,
) -> Result<R, ClipboardMimeError> {
    let platform = PLATFORM
        .with_borrow(Weak::upgrade)
        .ok_or(ClipboardMimeError::Unsupported)?;
    access(&platform)
}

/// The MIME types `target` advertises, without reading their data.
///
/// # Errors
/// [`ClipboardMimeError::Unsupported`] without a display connection or that selection.
pub fn clipboard_mime_types(target: ClipboardTarget) -> Result<Vec<String>, ClipboardMimeError> {
    with_platform(|platform| platform.clipboard_mime_types(target))
}

/// Each of `mimes` that `target` offers, read in request order; types it does not offer are skipped.
///
/// # Errors
/// [`ClipboardMimeError::Unsupported`] without a display connection or that selection.
pub fn read_clipboard_mimes(
    target: ClipboardTarget,
    mimes: &[String],
) -> Result<Vec<ClipboardRepresentation>, ClipboardMimeError> {
    with_platform(|platform| platform.read_clipboard_mimes(target, mimes))
}

/// Owns `target` with exactly `representations`; none clears it.
///
/// # Errors
/// [`ClipboardMimeError::Unsupported`] without a display connection or that selection,
/// [`ClipboardMimeError::Io`] when the display server refused ownership.
pub fn write_clipboard_representations(
    target: ClipboardTarget,
    representations: Vec<ClipboardRepresentation>,
) -> Result<(), ClipboardMimeError> {
    with_platform(|platform| platform.write_clipboard_representations(target, representations))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(mime: &str, data: &[u8]) -> ClipboardRepresentation {
        ClipboardRepresentation {
            mime: mime.to_owned(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn text_is_offered_utf8_first_beside_the_other_types() {
        let reps = [
            rep("text/plain", b"hi"),
            rep("text/html", b"<b>"),
            rep("image/png", b"\x89PNG"),
        ];
        assert_eq!(
            offered_mimes(&reps),
            [
                "text/plain;charset=utf-8",
                "text/plain",
                "UTF8_STRING",
                "text/plain;charset=UTF-8",
                "text/html",
                "image/png"
            ]
        );
        assert_eq!(offered_mimes(&[rep("image/png", b"")]), ["image/png"]);
        assert!(offered_mimes(&[]).is_empty());
    }

    #[test]
    fn text_names_serve_the_text_representation_and_others_their_own() {
        let reps = [rep("text/plain", b"hi"), rep("application/x-bin", &[0, 255])];
        assert_eq!(served(&reps, "UTF8_STRING"), Some(&b"hi"[..]));
        assert_eq!(served(&reps, "text/plain;charset=utf-8"), Some(&b"hi"[..]));
        assert_eq!(served(&reps, "TEXT"), Some(&b"hi"[..]));
        assert_eq!(served(&reps, "STRING"), Some(&b"hi"[..]));
        assert_eq!(served(&reps, "text/plain;charset=UTF-8"), Some(&b"hi"[..]));
        assert_eq!(served(&reps, "application/x-bin"), Some(&[0, 255][..]));
        assert_eq!(served(&reps, "image/png"), None);
        assert_eq!(served(&[rep("image/png", b"x")], "text/plain"), None);
    }

    #[test]
    fn a_text_request_reads_the_best_text_target_and_others_their_exact_type() {
        let offered: Vec<String> = ["STRING", "UTF8_STRING", "image/png"].map(String::from).to_vec();
        assert_eq!(source_type(&offered, "text/plain"), Some("UTF8_STRING"));
        assert_eq!(source_type(&offered, "image/png"), Some("image/png"));
        assert_eq!(source_type(&offered, "text/html"), None);
        let bare: Vec<String> = vec!["text/plain".into()];
        assert_eq!(source_type(&bare, "text/plain;charset=utf-8"), Some("text/plain"));
        // a legacy owner's Latin-1 `STRING` reads as UTF-8; other sources are served as read
        let latin: Vec<String> = vec!["STRING".into()];
        let source = source_type(&latin, "text/plain").unwrap();
        assert_eq!(source_data(source, vec![b'c', b'a', b'f', 0xE9]), "café".as_bytes());
        assert_eq!(source_data("UTF8_STRING", vec![0xE9]), [0xE9]);
    }

    #[test]
    fn listings_keep_mime_names_and_name_text_targets_once() {
        let offered: Vec<String> = [
            "TARGETS",
            "UTF8_STRING",
            "text/plain",
            "pid/42",
            "image/png",
            "STRING",
            "text/plain;charset=UTF-8",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            listed_mimes(&offered, Some("pid/42")),
            ["text/plain;charset=utf-8", "text/plain", "image/png"]
        );
    }
}
