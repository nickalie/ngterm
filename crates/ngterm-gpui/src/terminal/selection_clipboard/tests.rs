//! The clipboard locations' stores (upstream's `GhosttyCallbacksTests.swift`, #792), on GPUI's test platform: its
//! clipboard and primary selection are in memory, and on macOS the selection pasteboard is one of the test thread's
//! own, so nothing reaches the desktop's clipboard.

use gpui_kit::{App, ClipboardItem, TestAppContext};
use ngterm_term::ClipboardLocation;

use super::{Store, read, store, write};

/// Where Primary (OSC 52 `p`) lands: the system clipboard on macOS, the selection clipboard elsewhere.
const PRIMARY_STORE: Store = if cfg!(target_os = "macos") {
    Store::System
} else {
    Store::Selection
};

fn text(item: Option<ClipboardItem>) -> Option<String> {
    item.and_then(|item| item.text())
}

fn put(location: ClipboardLocation, value: &str, cx: &mut App) {
    write(location, ClipboardItem::new_string(value.to_owned()), cx);
}

/// The system clipboard, read past the mapping.
fn system(cx: &App) -> Option<String> {
    text(cx.read_from_clipboard())
}

fn seed_system(value: &str, cx: &App) {
    cx.write_to_clipboard(ClipboardItem::new_string(value.to_owned()));
}

/// The selection clipboard, read past the mapping.
#[cfg(target_os = "linux")]
fn selection(cx: &App) -> Option<String> {
    text(cx.read_from_primary())
}

#[cfg(target_os = "linux")]
fn seed_selection(value: &str, cx: &mut App) {
    cx.write_to_primary(ClipboardItem::new_string(value.to_owned()));
}

/// The test thread's own selection pasteboard, released when the thread ends.
#[cfg(target_os = "macos")]
struct OwnPasteboard(String);

#[cfg(target_os = "macos")]
impl Drop for OwnPasteboard {
    fn drop(&mut self) {
        let pasteboard =
            objc2_app_kit::NSPasteboard::pasteboardWithName(&objc2_foundation::NSString::from_str(&self.0));
        // SAFETY: a live `NSPasteboard`, and this message's signature; no one else knows this name.
        let () = unsafe { objc2::msg_send![&*pasteboard, releaseGlobally] };
    }
}

#[cfg(target_os = "macos")]
thread_local! {
    static OWN_PASTEBOARD: OwnPasteboard =
        OwnPasteboard(format!("com.nickalie.ngterm.test.{}", uuid::Uuid::new_v4()));
}

/// The name the selection store reads and writes under test.
#[cfg(target_os = "macos")]
pub(super) fn store_pasteboard() -> String {
    OWN_PASTEBOARD.with(|pasteboard| pasteboard.0.clone())
}

#[cfg(target_os = "macos")]
fn selection(cx: &App) -> Option<String> {
    text(super::read_selection(cx))
}

#[cfg(target_os = "macos")]
fn seed_selection(value: &str, cx: &mut App) {
    super::write_selection(&ClipboardItem::new_string(value.to_owned()), cx);
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn selection(cx: &App) -> Option<String> {
    cx.try_global::<super::SelectionBuffer>()
        .and_then(|buffer| text(buffer.item.clone()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn seed_selection(value: &str, cx: &mut App) {
    let buffer = cx.default_global::<super::SelectionBuffer>();
    buffer.item = Some(ClipboardItem::new_string(value.to_owned()));
    buffer.contents = None;
}

#[test]
fn each_location_maps_to_its_store() {
    assert_eq!(store(ClipboardLocation::Standard), Store::System);
    assert_eq!(store(ClipboardLocation::Selection), Store::Selection);
    assert_eq!(store(ClipboardLocation::Primary), PRIMARY_STORE);
}

#[gpui_kit::test]
fn a_selection_write_lands_on_the_selection_clipboard_and_leaves_the_system_one_alone(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_system("keep me", cx);
        put(ClipboardLocation::Selection, "selected text", cx);
        assert_eq!(system(cx).as_deref(), Some("keep me"));
        assert_eq!(selection(cx).as_deref(), Some("selected text"));
    });
}

#[gpui_kit::test]
fn a_standard_write_lands_on_the_system_clipboard_and_leaves_the_selection_alone(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_selection("old selection", cx);
        put(ClipboardLocation::Standard, "copied text", cx);
        assert_eq!(system(cx).as_deref(), Some("copied text"));
        assert_eq!(selection(cx).as_deref(), Some("old selection"));
    });
}

#[gpui_kit::test]
fn a_primary_write_lands_on_its_store(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_system("old clipboard", cx);
        seed_selection("old selection", cx);
        put(ClipboardLocation::Primary, "osc 52 p", cx);
        let (written, untouched, kept) = match PRIMARY_STORE {
            Store::System => (system(cx), selection(cx), "old selection"),
            Store::Selection => (selection(cx), system(cx), "old clipboard"),
        };
        assert_eq!(written.as_deref(), Some("osc 52 p"));
        assert_eq!(untouched.as_deref(), Some(kept));
    });
}

#[gpui_kit::test]
fn a_read_takes_the_locations_store(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_system("system clipboard", cx);
        seed_selection("last selection", cx);
        assert_eq!(
            text(read(ClipboardLocation::Selection, cx)).as_deref(),
            Some("last selection")
        );
        assert_eq!(
            text(read(ClipboardLocation::Standard, cx)).as_deref(),
            Some("system clipboard")
        );
        let primary = match PRIMARY_STORE {
            Store::System => "system clipboard",
            Store::Selection => "last selection",
        };
        assert_eq!(text(read(ClipboardLocation::Primary, cx)).as_deref(), Some(primary));
    });
}

#[cfg(target_os = "macos")]
#[test]
fn the_selection_pasteboard_is_named_for_this_app() {
    use objc2_app_kit::NSPasteboard;
    let app_id = crate::instance::app_id(&ngterm_core::paths::ProcessEnv);
    assert_eq!(super::selection_pasteboard_name(), format!("{app_id}.selection"));
    assert_ne!(
        super::selection_pasteboard_name(),
        NSPasteboard::generalPasteboard().name().to_string()
    );
}

/// The named pasteboard is real on macOS: before any selection it holds nothing, so a middle-click finds nothing.
#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn the_selection_pasteboard_starts_empty(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_system("system clipboard", cx);
        assert_eq!(text(read(ClipboardLocation::Selection, cx)), None);
    });
}

/// The selection holds text alone, as the terminal's other copies do: an image leaves what it held.
#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn a_selection_write_without_text_keeps_the_selection(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_selection("kept", cx);
        write(
            ClipboardLocation::Selection,
            ClipboardItem::new_image(&gpui_kit::Image::empty()),
            cx,
        );
        assert_eq!(selection(cx).as_deref(), Some("kept"));
    });
}

/// Windows has no selection clipboard: before any selection, a middle-click finds nothing to paste.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[gpui_kit::test]
fn the_selection_buffer_starts_empty(cx: &mut TestAppContext) {
    cx.update(|cx| {
        seed_system("system clipboard", cx);
        assert_eq!(text(read(ClipboardLocation::Selection, cx)), None);
    });
}

#[gpui_kit::test]
fn the_selection_store_round_trips_every_representation_and_clears(cx: &mut TestAppContext) {
    use ngterm_term::ClipboardContent;
    let part = |mime: &str, data: &[u8]| ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    };
    let parts = vec![
        part("text/plain", b"selected"),
        part("application/x-ngterm-test", &[0, 255, 7]),
    ];
    cx.update(|cx| {
        super::write_mimes(ClipboardLocation::Selection, parts.clone(), cx).unwrap();
        let wanted = [
            "application/x-ngterm-test".to_owned(),
            "image/png".into(),
            "text/plain".into(),
        ];
        assert_eq!(
            super::read_mimes(ClipboardLocation::Selection, &wanted, cx).unwrap(),
            [parts[1].clone(), parts[0].clone()]
        );
        let types = super::mime_types(ClipboardLocation::Selection, cx).unwrap();
        assert!(types.contains(&"text/plain".to_owned()), "{types:?}");
        // the selection's text reads as the item a middle-click pastes, where the store keeps items
        #[cfg(not(target_os = "linux"))]
        assert_eq!(selection(cx).as_deref(), Some("selected"));
        super::write_mimes(ClipboardLocation::Selection, Vec::new(), cx).unwrap();
        assert!(super::mime_types(ClipboardLocation::Selection, cx).unwrap().is_empty());
        // an item write replaces what a per-MIME write left
        seed_selection("plain", cx);
        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            super::read_mimes(ClipboardLocation::Selection, &wanted, cx).unwrap(),
            [part("text/plain", b"plain")]
        );
    });
}

#[gpui_kit::test]
fn text_aliases_read_each_store_as_text_and_keep_the_requested_names(cx: &mut TestAppContext) {
    use ngterm_term::ClipboardContent;
    let wanted = ["UTF8_STRING", "text/plain;charset=utf-8", "TEXT", "STRING"].map(String::from);
    let text = "héllo 🐈".as_bytes();
    cx.update(|cx| {
        for location in [
            ClipboardLocation::Standard,
            ClipboardLocation::Selection,
            ClipboardLocation::Primary,
        ] {
            super::write_mimes(
                location,
                vec![ClipboardContent {
                    mime: "text/plain".into(),
                    data: text.to_vec(),
                }],
                cx,
            )
            .unwrap();
            let expected: Vec<_> = wanted
                .iter()
                .map(|mime| ClipboardContent {
                    mime: mime.clone(),
                    data: text.to_vec(),
                })
                .collect();
            assert_eq!(super::read_mimes(location, &wanted, cx).unwrap(), expected);
            // The limit counts distinct requested names, before mapping aliases to the same source.
            let repeated: Vec<_> = wanted.iter().chain(wanted.iter()).cloned().collect();
            assert_eq!(super::read_mimes(location, &repeated, cx).unwrap(), expected);
            super::write_mimes(location, Vec::new(), cx).unwrap();
            assert!(super::read_mimes(location, &wanted, cx).unwrap().is_empty());
        }
    });
}

fn part(mime: &str, data: &[u8]) -> ngterm_term::ClipboardContent {
    ngterm_term::ClipboardContent {
        mime: mime.into(),
        data: data.to_vec(),
    }
}

#[test]
fn a_write_keeps_one_text_part_named_text_plain_where_it_stood() {
    let canonical = super::canonical_text;
    assert_eq!(
        canonical(vec![
            part("image/png", b"png"),
            part("UTF8_STRING", b"a"),
            part("TEXT", b"b")
        ]),
        [part("image/png", b"png"), part("text/plain", b"a")],
        "the first alias is relabelled, the others dropped"
    );
    assert_eq!(
        canonical(vec![
            part("text/plain;charset=utf-8", b"alias"),
            part("text/html", b"<b>"),
            part("text/plain", b"exact"),
            part("STRING", b"latin"),
        ]),
        [part("text/html", b"<b>"), part("text/plain", b"exact")],
        "an exact text/plain part wins over any alias"
    );
    assert_eq!(
        canonical(vec![part("text/html", b"<b>"), part("image/png", b"png")]),
        [part("text/html", b"<b>"), part("image/png", b"png")]
    );
    assert!(canonical(Vec::new()).is_empty());
}

#[gpui_kit::test]
fn an_alias_only_write_stores_text_that_every_text_name_and_a_paste_read(cx: &mut TestAppContext) {
    let text = "héllo 🐈".as_bytes();
    let alias = "text/plain;charset=utf-8".to_owned();
    cx.update(|cx| {
        for location in [
            ClipboardLocation::Standard,
            ClipboardLocation::Selection,
            ClipboardLocation::Primary,
        ] {
            super::write_mimes(location, vec![part(&alias, text)], cx).unwrap();
            assert_eq!(
                super::mime_types(location, cx).unwrap(),
                ["text/plain"],
                "{location:?}: the store holds real text"
            );
            assert_eq!(
                super::read_mimes(location, &["text/plain".into(), alias.clone(), "STRING".into()], cx).unwrap(),
                [part("text/plain", text), part(&alias, text), part("STRING", text)],
                "{location:?}"
            );
            // a plain paste (middle-click) of a store that keeps items reads the same text
            #[cfg(not(target_os = "linux"))]
            if location == ClipboardLocation::Selection {
                assert_eq!(selection(cx).as_deref(), Some("héllo 🐈"));
            }

            super::write_mimes(
                location,
                vec![part("UTF8_STRING", b"alias"), part("text/plain", b"exact")],
                cx,
            )
            .unwrap();
            assert_eq!(super::mime_types(location, cx).unwrap(), ["text/plain"], "{location:?}");
            assert_eq!(
                super::read_mimes(location, std::slice::from_ref(&alias), cx).unwrap(),
                [part(&alias, b"exact")],
                "{location:?}: the exact part wins"
            );
            #[cfg(not(target_os = "linux"))]
            if location == ClipboardLocation::Selection {
                assert_eq!(selection(cx).as_deref(), Some("exact"));
            }
            super::write_mimes(location, Vec::new(), cx).unwrap();
        }
    });
}
