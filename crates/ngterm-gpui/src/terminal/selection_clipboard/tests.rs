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
        .and_then(|buffer| text(buffer.0.clone()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn seed_selection(value: &str, cx: &mut App) {
    cx.default_global::<super::SelectionBuffer>().0 = Some(ClipboardItem::new_string(value.to_owned()));
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
