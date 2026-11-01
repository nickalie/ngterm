//! HTML overlays on WPE `WebKit` (upstream's `HtmlOverlayRegistry.swift`, `HtmlOverlayView.swift`, the handler side of
//! `HtmlOverlayBridge.swift` and the page parts of `WindowContentView+Detail.swift`; port of the GTK build's
//! `html_overlay*` from `WebKitGTK` onto `ngterm-wpe`). The GPUI side of a page — its element, its bounds, visibility,
//! focus and input — is modeled on gpui-kit's `crates/webview` (Apache-2.0); the engine renders offscreen and its
//! frames are drawn through [`crate::external_texture`].
//!
//! A registry owns every page, keyed by the page's id rather than by a host element or a pane, so a page survives
//! session switches, pane swaps and the soft-close window; it drops a page only when the core releases it ([`settle`]).
//! The session-wide page lies over the page as a program does and a pane page over the pane it covers, mounted by
//! `crate::overlay` ([`mount`]); every page is drawn in the terminal's colors and its scripts carry them as the page's
//! theme ([`refresh_themes`]). A page's strip offers Hide only while Settings ▸ Interface ▸ Hide overlays puts the
//! title bar's overlays button on, live on open pages ([`sync_settings`], upstream's c43bf01); the View menu, the
//! palette, the `toggle_overlay` chord and `session overlay show` bring a hidden page back either way. The registry is
//! the core's [`ngterm_core::html::HtmlHost`] ([`install`]).
//!
//! While a page holds the keyboard ([`page_holds_keyboard`]) the `undo_close` chord is the page's own undo, not the
//! pending close's (`crate::keymap_dispatch`, upstream's 00fc83d). On macOS, which has no page host yet, upstream's
//! form is the responder check in `UndoCloseShortcut.yieldsChord`.
//!
//! What a page may not do is refused as in the GTK build: new windows, script dialogs (answered silently), file
//! choosers, permissions, full screen, notifications and downloads (the engine's), a context menu's window and
//! download items (the app draws the menu, `html_overlay::menus`). Files never reach a page's script: the engine's
//! clipboard carries only the text the app hands it before a paste, and nothing is dropped onto a page. Where WPE's
//! sandbox cannot start, which would end the app at the first page, the host refuses every page instead (`sandbox`).
//!
//! A page holding the keyboard consumes every key it forwards to its engine, Escape included, so a key the page leaves
//! unhandled reaches nothing behind it, and full screen changes only through the app's own action (upstream's #753).
//! The other frontends' web-view ports carry the same rule: on macOS the page's `WKWebView` answers `cancelOperation:`
//! itself (upstream's `HtmlOverlayWebView`), as `WebKit` hands an unhandled Esc back up the responder chain and
//! `NSWindow` takes a full-screen window out of full screen on it; on Windows Win32 has no default Esc that leaves full
//! screen, and the `WebView2` host must not re-dispatch an unhandled Esc (`AcceleratorKeyPressed`) to a full-screen
//! handler.
//!
//! On Windows the pages are `WebView2` child windows of the GPUI window instead (`webview2`, on wry), sharing the
//! panel and the policy here; the engine, its frames, input, popups and sandbox are WPE's alone.
//!
//! Without the `html` cargo feature no host is installed, so the core refuses every page and nothing here mounts.

#[cfg(all(feature = "html", target_os = "linux"))]
mod engine;
#[cfg(all(feature = "html", target_os = "linux"))]
mod input;
#[cfg(all(feature = "html", target_os = "linux"))]
mod menus;
#[cfg(all(feature = "html", target_os = "linux"))]
mod page;
#[cfg(all(feature = "html", any(target_os = "linux", windows)))]
mod panel;
#[cfg(all(feature = "html", any(target_os = "linux", windows)))]
mod policy;
#[cfg(all(feature = "html", target_os = "linux"))]
mod registry;
#[cfg(all(feature = "html", target_os = "linux"))]
mod sandbox;
#[cfg(all(test, feature = "html", target_os = "linux"))]
pub(crate) mod testing;
#[cfg(all(feature = "html", target_os = "linux"))]
mod view;
#[cfg(all(feature = "html", windows))]
mod webview2;

#[cfg(all(feature = "html", any(target_os = "linux", windows)))]
pub use panel::{TerminalColors, sync_hide_shown as sync_strip_hide};
#[cfg(all(feature = "html", target_os = "linux"))]
pub use registry::{
    ReplicaLink, answer, answer_resource, answer_value, apply, init, install, key_down, mount, page_holds_keyboard,
    refresh_themes, reject, set_replica_link, settle, shutdown,
};
#[cfg(all(feature = "html", target_os = "linux"))]
use registry::{copy_link, navigate};
#[cfg(all(feature = "html", target_os = "linux"))]
pub use view::{FrameStats, PageView};
#[cfg(all(feature = "html", windows))]
pub use webview2::{
    PageView, answer, apply, hide, init, install, key_down, mount, page_holds_keyboard, refresh_themes, reject,
    set_covered, settle, shutdown,
};
#[cfg(all(feature = "html", windows))]
use webview2::{copy_link, navigate};

/// The terminal's colors of `config`, which the pages are drawn in.
#[cfg(all(feature = "html", any(target_os = "linux", windows)))]
#[must_use]
pub fn colors_of(config: &crate::terminal::TerminalConfig) -> TerminalColors {
    TerminalColors::of(&config.theme)
}

/// Keeps the pages in step with `settings` (the windows' sync): their strips offer Hide while Settings ▸ Interface ▸
/// Hide overlays shows the title bar's overlays button (upstream's c43bf01).
pub fn sync_settings(settings: &ngterm_core::persist::AppSettings, cx: &mut gpui_kit::App) {
    let overlay_toggle = ngterm_core::persist::settings::InterfaceElement::OverlayToggle;
    sync_strip_hide(!settings.is_interface_element_hidden(overlay_toggle), cx);
}

/// Marks `element`'s page off screen: its deck page is held out of the tree (a pending close) or hidden.
#[cfg(all(feature = "html", target_os = "linux"))]
pub fn hide(element: &gpui_kit::Entity<PageView>, cx: &mut gpui_kit::App) {
    let page = std::rc::Rc::clone(element.read(cx).page());
    let overlay = page.overlay();
    page.apply(&overlay, false, cx);
}

/// Whether a layer the app draws over `window`'s content (a dialog, the palette, a popover, an alert) is up: native
/// pages are hidden under it on Windows, where they sit above everything the window draws. Nothing to do where the
/// app draws the pages itself.
#[cfg(not(all(feature = "html", windows)))]
pub fn set_covered(_: &gpui_kit::Window, _: bool, _: &mut gpui_kit::App) {}

#[cfg(not(all(feature = "html", any(target_os = "linux", windows))))]
mod unavailable {
    //! The build without WPE `WebKit`: no host is installed, so no page ever reaches these.

    use gpui_kit::{App, Entity, FocusHandle, Focusable, IntoElement, Render, Window, div};
    use ngterm_core::Core;
    use ngterm_core::html::HtmlOverlay;
    use ngterm_proto::Response;

    use crate::state::Poster;

    /// A page's element: there is none in this build.
    pub struct PageView {
        focus: FocusHandle,
    }

    impl Render for PageView {
        fn render(&mut self, _: &mut Window, _: &mut gpui_kit::Context<Self>) -> impl IntoElement {
            div()
        }
    }

    impl Focusable for PageView {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus.clone()
        }
    }

    /// The pages' colors: unused in this build.
    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct TerminalColors;

    #[must_use]
    pub fn colors_of(_: &crate::terminal::TerminalConfig) -> TerminalColors {
        TerminalColors
    }

    /// Installs nothing: the core refuses every page.
    pub fn install(_: &mut Core, _: std::path::PathBuf, _: Poster) {}

    pub fn init(_: &mut App) {}

    /// No page has an element.
    #[must_use]
    pub fn mount(_: &HtmlOverlay, _: Option<&str>, _: &mut Window, _: &mut App) -> Option<Entity<PageView>> {
        None
    }

    pub fn apply(_: &HtmlOverlay, _: bool, _: &mut App) {}

    pub fn hide(_: &Entity<PageView>, _: &mut App) {}

    pub fn settle(_: &mut Core, _: &mut App) {}

    pub fn refresh_themes(_: &TerminalColors, _: &mut App) {}

    pub fn sync_strip_hide(_: bool, _: &mut App) {}

    /// No page holds the keyboard.
    #[must_use]
    pub fn page_holds_keyboard(_: &Window, _: &App) -> bool {
        false
    }

    pub fn answer(_: u64, _: &Response) {}

    pub fn reject(_: u64, _: &str) {}

    pub fn shutdown() {}

    #[must_use]
    pub fn key_down(
        _: &gpui_kit::Keystroke,
        _: &crate::terminal::input::KeyStroke,
        _: &mut Window,
        _: &mut App,
    ) -> bool {
        false
    }
}

#[cfg(not(all(feature = "html", any(target_os = "linux", windows))))]
pub use unavailable::{
    PageView, TerminalColors, answer, apply, colors_of, hide, init, install, key_down, mount, page_holds_keyboard,
    refresh_themes, reject, settle, shutdown, sync_strip_hide,
};
