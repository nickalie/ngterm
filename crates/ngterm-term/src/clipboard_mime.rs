//! The clipboard's MIME contract every frontend shares for the Kitty clipboard protocol (OSC 5522): Ghostty's
//! `terminal.clipboard.isTextMime` names the clipboard's text under several MIME names, a read asks a store for at
//! most four distinct names (`kitty/clipboard_response.zig`), and a write hands a store its text as `text/plain`
//! alone. The desktop's stores (`ngterm-gpui`'s `selection_clipboard.rs`) and the mobile apps (`ngterm-mobile`'s
//! panes) both go through it, so a read under any text name finds what any write stored.

use crate::ClipboardContent;

/// The most distinct MIME types one read hands back (Ghostty's `clipboard_response.zig`).
pub const MAX_READ_TYPES: usize = 4;

/// The most MIME types a listing names (Ghostty's `clipboard_response.zig`).
pub const MAX_LISTED_TYPES: usize = 16;

/// The name a store keeps `mime` under: every Ghostty text name is the store's `text/plain`.
#[must_use]
pub fn text_source(mime: &str) -> &str {
    match mime {
        "text/plain" | "text/plain;charset=utf-8" | "UTF8_STRING" | "TEXT" | "STRING" => "text/plain",
        _ => mime,
    }
}

/// What a read of `mimes` asks: the first four distinct requested names (`wanted`, in request order), and the names to
/// ask the store for (`sources`: each text alias as `text/plain`, once).
#[must_use]
pub fn read_sources(mimes: &[String]) -> (Vec<String>, Vec<String>) {
    let mut wanted: Vec<String> = Vec::new();
    for mime in mimes {
        if wanted.len() < MAX_READ_TYPES && !wanted.contains(mime) {
            wanted.push(mime.clone());
        }
    }
    let mut sources: Vec<String> = Vec::new();
    for mime in &wanted {
        let source = text_source(mime);
        if !sources.iter().any(|known| known == source) {
            sources.push(source.to_owned());
        }
    }
    (wanted, sources)
}

/// The store's answer `contents` (named by [`read_sources`]' sources) relabelled with each `wanted` name, in request
/// order; a name the store did not answer is skipped.
#[must_use]
pub fn relabel(wanted: Vec<String>, contents: &[ClipboardContent]) -> Vec<ClipboardContent> {
    wanted
        .into_iter()
        .filter_map(|mime| {
            contents
                .iter()
                .find(|content| content.mime == text_source(&mime))
                .map(|content| ClipboardContent {
                    mime,
                    data: content.data.clone(),
                })
        })
        .collect()
}

/// Reads `mimes` through a store's own `read` of its source names, as [`read_sources`] and [`relabel`] say.
///
/// # Errors
/// Whatever `read` fails with.
pub fn read_through<E>(
    mimes: &[String],
    read: impl FnOnce(&[String]) -> Result<Vec<ClipboardContent>, E>,
) -> Result<Vec<ClipboardContent>, E> {
    let (wanted, sources) = read_sources(mimes);
    let contents = read(&sources)?;
    Ok(relabel(wanted, &contents))
}

/// `contents` with one text part, named `text/plain`, where it stood: an exact `text/plain` part wins, else the first
/// alias part ([`text_source`]) is relabelled; the other text parts are dropped.
#[must_use]
pub fn canonical_text(contents: Vec<ClipboardContent>) -> Vec<ClipboardContent> {
    let is_text = |content: &ClipboardContent| text_source(&content.mime) == "text/plain";
    let kept = contents
        .iter()
        .position(|content| content.mime == "text/plain")
        .or_else(|| contents.iter().position(is_text));
    contents
        .into_iter()
        .enumerate()
        .filter_map(|(index, mut content)| {
            if !is_text(&content) {
                Some(content)
            } else if Some(index) == kept {
                content.mime = "text/plain".into();
                Some(content)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(mime: &str, data: &[u8]) -> ClipboardContent {
        ClipboardContent {
            mime: mime.into(),
            data: data.to_vec(),
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_write_keeps_one_text_part_named_text_plain_where_it_stood() {
        assert_eq!(
            canonical_text(vec![
                part("image/png", b"png"),
                part("UTF8_STRING", b"a"),
                part("TEXT", b"b")
            ]),
            [part("image/png", b"png"), part("text/plain", b"a")],
            "the first alias is relabelled, the others dropped"
        );
        assert_eq!(
            canonical_text(vec![
                part("text/plain;charset=utf-8", b"alias"),
                part("text/html", b"<b>"),
                part("text/plain", b"exact"),
                part("STRING", b"latin"),
            ]),
            [part("text/html", b"<b>"), part("text/plain", b"exact")],
            "an exact text/plain part wins over any alias"
        );
        assert_eq!(
            canonical_text(vec![part("text/html", b"<b>"), part("image/png", b"png")]),
            [part("text/html", b"<b>"), part("image/png", b"png")]
        );
        assert!(canonical_text(Vec::new()).is_empty());
    }

    #[test]
    fn a_read_asks_four_distinct_names_and_each_text_alias_once() {
        let (wanted, sources) = read_sources(&names(&[
            "UTF8_STRING",
            "image/png",
            "UTF8_STRING",
            "text/plain",
            "text/html",
            "application/json",
        ]));
        assert_eq!(wanted, names(&["UTF8_STRING", "image/png", "text/plain", "text/html"]));
        assert_eq!(sources, names(&["text/plain", "image/png", "text/html"]));
    }

    #[test]
    fn answers_are_relabelled_in_request_order_and_missing_names_skipped() {
        let read = read_through(&names(&["TEXT", "image/png", "text/plain", "text/html"]), |sources| {
            assert_eq!(sources, names(&["text/plain", "image/png", "text/html"]));
            Ok::<_, ()>(vec![part("text/html", b"<b>"), part("text/plain", b"hi")])
        })
        .unwrap();
        assert_eq!(
            read,
            [
                part("TEXT", b"hi"),
                part("text/plain", b"hi"),
                part("text/html", b"<b>")
            ]
        );
        assert_eq!(
            read_through(&names(&["image/png"]), |_| Err::<Vec<_>, _>("io")),
            Err("io")
        );
    }
}
