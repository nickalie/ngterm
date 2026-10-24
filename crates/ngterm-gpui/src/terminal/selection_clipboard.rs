//! Where each of ghostty's clipboard locations lives on this OS (upstream's `GhosttyCallbacks.pasteboard(for:)`,
//! #792): every read and write of a terminal (`copy-on-select`, a middle-click, `paste_from_selection`, a program's
//! OSC 52) goes through [`read`] and [`write`].
//!
//! - Standard is the system clipboard everywhere.
//! - Selection (`copy-on-select`, OSC 52 `s`) is the selection clipboard: the primary selection on Linux (Wayland's
//!   primary selection, X11's PRIMARY); on macOS a private pasteboard named for the app id (`<app id>.selection`,
//!   as in Ghostty.app and upstream), so selecting text never replaces the system clipboard; on Windows, which has no
//!   selection clipboard, an app-wide in-process buffer, so a middle-click (`primary-paste`) pastes the last
//!   selection made inside ngterm.
//! - Primary (OSC 52 `p`) is the primary selection on Linux and the system clipboard on macOS (upstream: an OSC 52
//!   `p` write reaches the embedder, and macOS has no primary of its own); on Windows it shares the selection buffer,
//!   as it shares PRIMARY with Selection on Linux.
//!
//! The macOS and Windows forms are not built on macOS/Windows by CI until ngterm-gpui joins `cross-platform.yml`.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;

use gpui_kit::{App, ClipboardItem};
use ngterm_term::ClipboardLocation;

/// The store a clipboard location reads and writes on this OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Store {
    /// The system clipboard (GPUI's).
    System,
    /// The selection clipboard: the primary selection on Linux, the `<app id>.selection` pasteboard on macOS, the
    /// in-process buffer on Windows.
    Selection,
}

/// The store `location` reads and writes on this OS.
#[must_use]
pub(super) const fn store(location: ClipboardLocation) -> Store {
    match location {
        ClipboardLocation::Standard => Store::System,
        ClipboardLocation::Primary if cfg!(target_os = "macos") => Store::System,
        ClipboardLocation::Selection | ClipboardLocation::Primary => Store::Selection,
    }
}

/// What `location` holds.
#[must_use]
pub(super) fn read(location: ClipboardLocation, cx: &App) -> Option<ClipboardItem> {
    match store(location) {
        Store::System => cx.read_from_clipboard(),
        Store::Selection => read_selection(cx),
    }
}

/// Puts `item` on `location`.
pub(super) fn write(location: ClipboardLocation, item: ClipboardItem, cx: &mut App) {
    match store(location) {
        Store::System => cx.write_to_clipboard(item),
        Store::Selection => write_selection(&item, cx),
    }
}

#[cfg(target_os = "linux")]
fn read_selection(cx: &App) -> Option<ClipboardItem> {
    cx.read_from_primary()
}

#[cfg(target_os = "linux")]
fn write_selection(item: &ClipboardItem, cx: &mut App) {
    cx.write_to_primary(item.clone());
}

/// The private selection pasteboard's name: the app id's, as upstream names it for its bundle id.
#[cfg(target_os = "macos")]
#[must_use]
pub(super) fn selection_pasteboard_name() -> String {
    use std::sync::OnceLock;
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| macos::pasteboard_name(&crate::instance::app_id(&ngterm_core::paths::ProcessEnv)))
        .clone()
}

/// The pasteboard the selection store uses: [`selection_pasteboard_name`]'s, and in tests one of the test thread's
/// own, so no test touches the app's selection pasteboard or another test's.
#[cfg(target_os = "macos")]
fn store_pasteboard() -> String {
    let name = selection_pasteboard_name();
    if cfg!(test) {
        format!("{name}.test-{}-{:?}", std::process::id(), std::thread::current().id())
    } else {
        name
    }
}

#[cfg(target_os = "macos")]
fn read_selection(_: &App) -> Option<ClipboardItem> {
    macos::read(&store_pasteboard()).map(ClipboardItem::new_string)
}

#[cfg(target_os = "macos")]
fn write_selection(item: &ClipboardItem, _: &mut App) {
    // text only, as the terminal's other copies (GPUI's clipboard item carries the text a selection formats to)
    if let Some(text) = item.text() {
        macos::write(&store_pasteboard(), &text);
    }
}

/// The selection buffer of an OS with no selection clipboard (Windows): one per app, shared by every terminal.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[derive(Default)]
struct SelectionBuffer(Option<ClipboardItem>);

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl gpui_kit::Global for SelectionBuffer {}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_selection(cx: &App) -> Option<ClipboardItem> {
    cx.try_global::<SelectionBuffer>().and_then(|buffer| buffer.0.clone())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn write_selection(item: &ClipboardItem, cx: &mut App) {
    cx.default_global::<SelectionBuffer>().0 = Some(item.clone());
}
