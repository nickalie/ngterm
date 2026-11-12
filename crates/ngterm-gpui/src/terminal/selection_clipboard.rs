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
//! Both native forms are built and tested in `cross-platform.yml`.
//!
//! The Kitty clipboard protocol reads and writes those same stores per MIME type ([`mime_types`], [`read_mimes`],
//! [`write_mimes`]): GPUI's Wayland/X11 clipboard through our `gpui-pre-linux` patch, `NSPasteboard` types on macOS
//! (`selection_clipboard/macos.rs`), the Win32 clipboard's formats on Windows (`selection_clipboard/windows.rs`). A
//! test build keeps the system clipboard and Linux's primary selection in memory instead, per thread.

#[cfg(any(windows, test))]
mod cf_html;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

use gpui_kit::{App, ClipboardItem};
use ngterm_term::{ClipboardContent, ClipboardLocation, clipboard_mime};

/// Why a per-MIME access did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MimeFailure {
    /// This OS has no such clipboard (or no display connection).
    #[cfg_attr(
        not(target_os = "linux"),
        expect(
            dead_code,
            reason = "only Linux can lack a clipboard: headless, or without a primary selection"
        )
    )]
    Unsupported,
    /// The clipboard refused or the transfer failed.
    Io(String),
}

/// The MIME types the store of `location` advertises, at most 16, without reading their data.
///
/// # Errors
/// As [`MimeFailure`] says.
pub(super) fn mime_types(location: ClipboardLocation, cx: &App) -> Result<Vec<String>, MimeFailure> {
    let mut types = match store(location) {
        Store::System => system::mime_types(cx),
        Store::Selection => selection_mimes::mime_types(cx),
    }?;
    types.truncate(clipboard_mime::MAX_LISTED_TYPES);
    Ok(types)
}

/// Each of `mimes` (the first four) the store of `location` holds, read in request order; the rest are skipped.
/// Text aliases read the store's `text/plain` and retain the requested name (Ghostty's `embedded.zig`).
///
/// # Errors
/// As [`MimeFailure`] says.
pub(super) fn read_mimes(
    location: ClipboardLocation,
    mimes: &[String],
    cx: &App,
) -> Result<Vec<ClipboardContent>, MimeFailure> {
    read_through(mimes, |sources| match store(location) {
        Store::System => system::read_mimes(sources, cx),
        Store::Selection => selection_mimes::read_mimes(sources, cx),
    })
}

/// The shared read contract over a store's own `read` (`ngterm_term::clipboard_mime::read_through`): the first four
/// distinct names, each text alias asked of the store as `text/plain` (once), every answer relabelled with the name it
/// was asked under.
fn read_through(
    mimes: &[String],
    read: impl FnOnce(&[String]) -> Result<Vec<ClipboardContent>, MimeFailure>,
) -> Result<Vec<ClipboardContent>, MimeFailure> {
    clipboard_mime::read_through(mimes, read)
}

/// Replaces what the store of `location` holds with exactly `contents`; none clears it.
///
/// # Errors
/// As [`MimeFailure`] says.
pub(super) fn write_mimes(
    location: ClipboardLocation,
    contents: Vec<ClipboardContent>,
    cx: &mut App,
) -> Result<(), MimeFailure> {
    write_through(contents, |contents| match store(location) {
        Store::System => system::write_mimes(contents, cx),
        Store::Selection => selection_mimes::write_mimes(contents, cx),
    })
}

/// The shared write contract over a store's own `write`: the store receives its text as `text/plain` only, so it
/// keeps real text (Windows's `CF_UNICODETEXT`, GPUI's text item and UTF-8 offers, the selection buffer's item)
/// and a read under any text name finds it.
fn write_through(
    contents: Vec<ClipboardContent>,
    write: impl FnOnce(Vec<ClipboardContent>) -> Result<(), MimeFailure>,
) -> Result<(), MimeFailure> {
    write(clipboard_mime::canonical_text(contents))
}

/// The text a set of representations carries as a GPUI item (`text/plain`), for the stores that keep items.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn text_item(contents: &[ClipboardContent]) -> Option<ClipboardItem> {
    contents
        .iter()
        .find(|content| content.mime == "text/plain")
        .map(|content| ClipboardItem::new_string(String::from_utf8_lossy(&content.data).into_owned()))
}

/// The representations a GPUI item carries: its text as `text/plain`.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn item_contents(item: Option<&ClipboardItem>) -> Vec<ClipboardContent> {
    item.and_then(ClipboardItem::text)
        .map(|text| ClipboardContent {
            mime: "text/plain".into(),
            data: text.into_bytes(),
        })
        .into_iter()
        .collect()
}

/// `contents` limited to `wanted`, in `wanted`'s order.
#[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
fn pick(contents: &[ClipboardContent], wanted: &[String]) -> Vec<ClipboardContent> {
    wanted
        .iter()
        .filter_map(|mime| contents.iter().find(|content| &content.mime == mime).cloned())
        .collect()
}

/// The system clipboard per MIME: the platform's, or in memory under test.
#[cfg(not(test))]
mod system {
    #[cfg(target_os = "linux")]
    pub(super) use super::linux::{
        clipboard_mime_types as mime_types, read_clipboard_mimes as read_mimes, write_clipboard_mimes as write_mimes,
    };
    #[cfg(target_os = "macos")]
    pub(super) use super::macos::{
        general_mime_types as mime_types, read_general_mimes as read_mimes, write_general_mimes as write_mimes,
    };
    #[cfg(windows)]
    pub(super) use super::windows::{mime_types, read_mimes, write_mimes};
}

#[cfg(test)]
mod system {
    use super::{App, ClipboardContent, MimeFailure, fake::Fake};

    pub(super) fn mime_types(_: &App) -> Result<Vec<String>, MimeFailure> {
        Fake::System.available()?;
        Ok(Fake::System.mime_types())
    }

    pub(super) fn read_mimes(wanted: &[String], _: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
        Fake::System.available()?;
        Ok(Fake::System.read(wanted))
    }

    pub(super) fn write_mimes(contents: Vec<ClipboardContent>, _: &mut App) -> Result<(), MimeFailure> {
        Fake::System.available()?;
        Fake::System.write(contents);
        Ok(())
    }
}

/// The in-memory stores of a test build, per thread.
#[cfg(test)]
pub(super) mod fake {
    use std::cell::RefCell;

    use ngterm_term::ClipboardContent;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(in super::super) enum Fake {
        System,
        #[cfg_attr(
            not(target_os = "linux"),
            expect(dead_code, reason = "only Linux keeps its primary selection in memory under test")
        )]
        Primary,
    }

    use super::MimeFailure;

    thread_local! {
        static STORES: RefCell<[Vec<ClipboardContent>; 2]> = RefCell::default();
        static FAILURES: RefCell<[Option<MimeFailure>; 2]> = RefCell::default();
    }

    impl Fake {
        fn index(self) -> usize {
            match self {
                Self::System => 0,
                Self::Primary => 1,
            }
        }

        pub(in super::super) fn write(self, contents: Vec<ClipboardContent>) {
            STORES.with_borrow_mut(|stores| stores[self.index()] = contents);
        }

        pub(in super::super) fn contents(self) -> Vec<ClipboardContent> {
            STORES.with_borrow(|stores| stores[self.index()].clone())
        }

        pub(in super::super) fn mime_types(self) -> Vec<String> {
            self.contents().into_iter().map(|content| content.mime).collect()
        }

        pub(in super::super) fn read(self, wanted: &[String]) -> Vec<ClipboardContent> {
            super::pick(&self.contents(), wanted)
        }

        /// Makes every per-MIME access of this store fail with `failure` (none: succeed again).
        pub(in super::super) fn fail(self, failure: Option<MimeFailure>) {
            FAILURES.with_borrow_mut(|failures| failures[self.index()] = failure);
        }

        pub(in super::super) fn available(self) -> Result<(), MimeFailure> {
            FAILURES.with_borrow(|failures| failures[self.index()].clone().map_or(Ok(()), Err))
        }
    }
}

/// The selection clipboard per MIME: Linux's primary selection (in memory under test), macOS's selection pasteboard,
/// Windows's in-process buffer.
#[cfg(all(target_os = "linux", not(test)))]
mod selection_mimes {
    pub(super) use super::linux::{
        primary_mime_types as mime_types, read_primary_mimes as read_mimes, write_primary_mimes as write_mimes,
    };
}

#[cfg(all(target_os = "linux", test))]
mod selection_mimes {
    use super::{App, ClipboardContent, MimeFailure, fake::Fake};

    pub(super) fn mime_types(_: &App) -> Result<Vec<String>, MimeFailure> {
        Fake::Primary.available()?;
        Ok(Fake::Primary.mime_types())
    }

    pub(super) fn read_mimes(wanted: &[String], _: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
        Fake::Primary.available()?;
        Ok(Fake::Primary.read(wanted))
    }

    pub(super) fn write_mimes(contents: Vec<ClipboardContent>, _: &mut App) -> Result<(), MimeFailure> {
        Fake::Primary.available()?;
        Fake::Primary.write(contents);
        Ok(())
    }
}

#[cfg(target_os = "macos")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the pasteboard's reads share the other stores' signatures"
)]
mod selection_mimes {
    use super::{App, ClipboardContent, MimeFailure, store_pasteboard};

    pub(super) fn mime_types(_: &App) -> Result<Vec<String>, MimeFailure> {
        Ok(super::macos::mime_types(Some(&store_pasteboard())))
    }

    pub(super) fn read_mimes(wanted: &[String], _: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
        Ok(super::macos::read_mimes(Some(&store_pasteboard()), wanted))
    }

    pub(super) fn write_mimes(contents: Vec<ClipboardContent>, _: &mut App) -> Result<(), MimeFailure> {
        super::macos::write_mimes(Some(&store_pasteboard()), contents)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "an in-memory store shares the native stores' signatures"
)]
mod selection_mimes {
    use super::{App, ClipboardContent, MimeFailure, SelectionBuffer};

    fn contents(cx: &App) -> Vec<ClipboardContent> {
        cx.try_global::<SelectionBuffer>().map_or_else(Vec::new, |buffer| {
            buffer
                .contents
                .clone()
                .unwrap_or_else(|| super::item_contents(buffer.item.as_ref()))
        })
    }

    pub(super) fn mime_types(cx: &App) -> Result<Vec<String>, MimeFailure> {
        Ok(contents(cx).into_iter().map(|content| content.mime).collect())
    }

    pub(super) fn read_mimes(wanted: &[String], cx: &App) -> Result<Vec<ClipboardContent>, MimeFailure> {
        Ok(super::pick(&contents(cx), wanted))
    }

    pub(super) fn write_mimes(contents: Vec<ClipboardContent>, cx: &mut App) -> Result<(), MimeFailure> {
        let buffer = cx.default_global::<SelectionBuffer>();
        buffer.item = super::text_item(&contents);
        buffer.contents = Some(contents);
        Ok(())
    }
}

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

/// The pasteboard the selection lives on; a test's is its own thread's, so nothing reaches the desktop's.
#[cfg(all(target_os = "macos", not(test)))]
use self::selection_pasteboard_name as store_pasteboard;
#[cfg(all(target_os = "macos", test))]
use self::tests::store_pasteboard;

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

/// The selection buffer of an OS with no selection clipboard (Windows): one per app, shared by every terminal. A
/// per-MIME write keeps its representations beside their text; an item write replaces them.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[derive(Default)]
struct SelectionBuffer {
    item: Option<ClipboardItem>,
    contents: Option<Vec<ClipboardContent>>,
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl gpui_kit::Global for SelectionBuffer {}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_selection(cx: &App) -> Option<ClipboardItem> {
    cx.try_global::<SelectionBuffer>()
        .and_then(|buffer| buffer.item.clone())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn write_selection(item: &ClipboardItem, cx: &mut App) {
    let buffer = cx.default_global::<SelectionBuffer>();
    buffer.item = Some(item.clone());
    buffer.contents = None;
}
