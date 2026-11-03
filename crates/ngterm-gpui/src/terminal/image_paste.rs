//! A remote program cannot read the viewer's desktop clipboard. Upload a user-pasted image over a separate SSH
//! connection, then send its remote filename as a bracketed paste. The private file remains in the remote temporary
//! directory so an agent can read it after the paste completes. No image bytes travel through the terminal's PTY.

use std::rc::Rc;

use gpui_kit::{ClipboardEntry, ClipboardItem, Context};
use ngterm_core::command_restore::shell_quoted_line;
use ngterm_term::{ClipboardLocation, PasteSource};

use super::{Shared, Terminal};

/// Whether an image paste was accepted. Text continues through the ordinary paste path.
pub(super) fn start(shared: &Shared, host: &str, item: &ClipboardItem, cx: &mut Context<Terminal>) -> bool {
    // Some platforms offer text and an image together (for example an Office selection on Windows).
    // Preserve the ordinary text paste in that case, as the Linux and macOS clipboard readers do.
    if item.text().is_some() {
        return false;
    }
    let Some(image) = item.entries().iter().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image.clone()),
        _ => None,
    }) else {
        return false;
    };
    let Some(pane) = shared.pane() else {
        return true;
    };
    let pane = Rc::downgrade(&pane);
    let host = host.to_owned();
    let upload = cx
        .background_executor()
        .spawn(async move { ngterm_core::remote::clipboard::upload(&host, image.format.extension(), &image.bytes) });
    cx.spawn(async move |this, cx| {
        let result = upload.await;
        let _ = this.update(cx, |this, cx| {
            let shared = this.shared();
            // A replacement, teardown or read-only toggle during the transfer must not receive late input.
            let (Some(current), Some(original)) = (shared.pane(), pane.upgrade()) else {
                return;
            };
            if !Rc::ptr_eq(&current, &original) || shared.is_readonly() {
                return;
            }
            let result = result.and_then(|path| paste_filename(&current, &path));
            if let Err(error) = result {
                show_error(shared.key(), &error, cx);
            }
        });
    })
    .detach();
    true
}

/// A generated path is inserted text: Kitty paste-event mode must not replace it with a clipboard request.
pub(super) fn paste_filename(pane: &ngterm_term::Pane, path: &str) -> Result<(), String> {
    let path = shell_quoted_line(&[path]);
    pane.paste(&path, PasteSource::Text, ClipboardLocation::Standard, true)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn show_error(key: u64, error: &str, cx: &mut gpui_kit::App) {
    tracing::warn!(target: "ngterm::surface", key, error, "image paste failed");
    if let Some(window) = super::keyboard::window_of(key) {
        let spec = crate::modal::AlertSpec::info("Failed to paste image", error);
        let _ = window.update(cx, |_, window, cx| {
            crate::modal::open_alert(window, spec, |_, _, _| {}, cx);
        });
    }
}
