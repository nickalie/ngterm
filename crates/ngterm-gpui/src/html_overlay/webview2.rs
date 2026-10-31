//! HTML overlays on Windows: each page a `WebView2` child window of its GPUI window, made through tauri's wry
//! (docs/plans/20261029-windows-support.md, Decisions > HTML overlays), as upstream places a native `WKWebView` over
//! its window. The app draws the panel, the strip and a failure as on Linux (`panel`); the page itself is the native
//! window over the content area, which `WebView2` draws, so there are no frames, no popups of the app's and no input
//! relayed: the pointer, the keys, the input method, `<select>` popups, the context menu and the clipboard are the web
//! view's own, but for the app's chords, which the keymap gets first as on Linux (`keys`). The policy, the load
//! tracker, the page reports and the hand-offs are the WPE host's (`policy`, core's `html`), read through `WebView2`'s
//! events (`page`, `native`). GPUI's `DirectComposition` visual sits under its child windows (a `gpui-pre-windows`
//! patch) so the native windows show; anything the app draws over the content (a dialog, the palette, a popover, the
//! switcher, an alert) hides the pages of its window meanwhile ([`set_covered`]), as a native window cannot sit under
//! the app's drawing.
//!
//! - Stores (`store`): one `WebView2` environment per state directory, the saved store a profile named by the browser
//!   profile id, every other page an `InPrivate` profile of its own. Saved logins live under
//!   `<state>/browser/webview2`.
//! - Files (`scheme`): a granted file page loads through a protocol of its own that serves only files inside its
//!   grant, a replica through one that reads from the machine that opened it, a file page without a grant its text
//!   through one that serves that text alone, read back as the `about:blank` it loads at on Linux.
//! - The bridge (`bridge`): ngterm's scripts in the page's world from one closure, their messages signed with a nonce
//!   page code never sees, replies posted back as web messages.
//! - What a page may not do (`native`): new windows, script dialogs, permissions, downloads, dropped and pasted files,
//!   file choosers, devtools, the browser's accelerator keys and its menu's save, print, share and inspect items.
//! - The runtime (`runtime`): checked at the start; without it every page is refused with where to get it.
//!
//! What `ngterm-wpe` holds that has no `WebView2` meaning: frames and their buffers (dmabuf and SHM, their formats,
//! the fall-back between them, `crate::external_texture`), input forwarding (keysyms, buttons, the wheel, cursor
//! names), the app-drawn popups (`<select>` rows and the context menu, `html_overlay::menus`) and editing commands,
//! the engine's own clipboard handed text before a paste, the input method relay, the engine worker process and its
//! launcher (the vendored WPE's overlay of `/usr/lib`), the bubblewrap sandbox check (`html_overlay::sandbox`), the
//! engine thread's main context, and the response policy (`WebView2` asks again for each redirect before it is
//! followed, so a main frame is never checked on its response).

mod bridge;
mod browser;
mod keys;
mod native;
mod page;
mod registry;
mod runtime;
mod scheme;
mod store;
mod view;

pub use registry::{
    answer, apply, init, install, key_down, mount, refresh_themes, reject, set_covered, settle, shutdown,
};
pub(super) use registry::{copy_link, navigate};
pub use view::PageView;

/// Marks `element`'s page off screen: its deck page is held out of the tree (a pending close) or hidden.
pub fn hide(element: &gpui_kit::Entity<PageView>, cx: &mut gpui_kit::App) {
    let page = std::rc::Rc::clone(element.read(cx).page());
    let overlay = page.overlay();
    page.apply(&overlay, false, cx);
}
