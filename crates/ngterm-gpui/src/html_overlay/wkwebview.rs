//! Native `WKWebView` pages, sharing the native child-view lifecycle and page policy with `WebView2`.
#[path = "webview2/bridge.rs"]
mod bridge;
mod browser;
mod keys;
mod native;
#[path = "webview2/page.rs"]
mod page;
#[path = "webview2/registry.rs"]
mod registry;
mod scheme;
mod store;
#[path = "webview2/view.rs"]
mod view;
mod runtime {
    pub fn installed_version() -> Result<String, String> {
        unsafe extern "C" {
            fn ngterm_web_supported() -> bool;
        }
        // SAFETY: a platform version check without side effects.
        if unsafe { ngterm_web_supported() } {
            Ok("system WebKit".into())
        } else {
            Err("HTML pages require macOS 14 or newer".into())
        }
    }
}
pub use registry::{
    answer, apply, init, install, key_down, mount, page_holds_keyboard, refresh_themes, reject, set_covered, settle,
    shutdown,
};
pub(super) use registry::{copy_link, navigate};
pub use view::PageView;
pub fn hide(element: &gpui_kit::Entity<PageView>, cx: &mut gpui_kit::App) {
    let page = std::rc::Rc::clone(element.read(cx).page());
    page.apply(&page.overlay(), false, cx);
}
