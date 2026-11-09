//! The seam between a page and its web view: what `page` and `registry` ask of one ([`NativeView`]), and the
//! calls that make or clear one ([`create`], [`clear_profile`]) or open the system browser ([`open_browser`]). Each
//! platform's `native::Native` is the view; a unit test stands in for the view and all three calls (`fake_native`), as
//! the WPE host's tests stand in for its engine.

use std::rc::Rc;

use async_channel::Receiver;

use super::super::text_page::Answer;
use super::native::{self, Bounds, Environment, Native, Spec, Store};

/// How a replica's read is answered, once its origin did: the responder, as the protocol handed it over.
pub type ReadReply = Box<dyn FnOnce(wry::http::Response<Vec<u8>>) + Send>;

/// A page's web view, as a page and the registry drive it.
pub trait NativeView {
    /// Replaces the theme script, which later documents run; the page hears once it is in place.
    fn set_theme(&self, script: &str);
    /// Loads `address`, as the web view spells it.
    fn load_url(&self, address: &str);
    fn reload(&self);
    fn go_back(&self);
    fn go_forward(&self);
    fn set_zoom(&self, zoom: f64);
    fn set_background(&self, color: (u8, u8, u8));
    fn set_bounds(&self, bounds: Bounds);
    fn set_visible(&self, visible: bool);
    /// Gives the web view the keyboard.
    fn focus(&self);
    /// Hands the keyboard back to the window.
    fn focus_parent(&self);
    /// Moves the web view into window `parent` (its session went to another window).
    fn reparent(&self, parent: isize);
    /// Posts `json` to the page as a web message.
    fn post_json(&self, json: &str);
    /// Gives the page back a key its script stopped and the keymap did not take: `events` are its
    /// `Input.dispatchKeyEvent` calls, in order (`bridge::redelivery`).
    #[cfg(windows)]
    fn redeliver_key(&self, events: &[serde_json::Value]);
    /// Evaluates `expression` in the renderer's world; its answer arrives on the returned channel.
    fn call_renderer(&self, expression: &str) -> Receiver<Answer>;
}

/// Makes the web view `spec` describes, with the environment it runs in, which later pages share.
pub fn create(spec: &Spec) -> Result<(Rc<dyn NativeView>, Option<Environment>), String> {
    #[cfg(test)]
    if let Some(engine) = super::fake_native::engine() {
        return engine.create(spec).map(|view| (view, None));
    }
    let native = Native::create(spec)?;
    let environment = native.environment();
    Ok((Rc::new(native), Some(environment)))
}

/// Opens `uri` in the default web browser; false when there is none.
pub fn open_browser(uri: &str) -> bool {
    #[cfg(test)]
    if let Some(engine) = super::fake_native::engine() {
        return engine.open_browser(uri);
    }
    super::browser::open(uri)
}

/// Empties `store`'s profile through a web view in window `parent`; `done` gets the outcome. The environment it ran
/// in, for the pages after it.
pub fn clear_profile(
    parent: isize,
    store: Store,
    done: Box<dyn FnOnce(Result<(), String>) + Send>,
) -> Option<Environment> {
    #[cfg(test)]
    if let Some(engine) = super::fake_native::engine() {
        return engine.clear_profile(parent, store, done);
    }
    native::clear_profile(parent, store, done)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// No stand-in here: the real web view is asked for, and a window that is no window refuses it at once.
    #[test]
    fn a_web_view_with_no_parent_window_cannot_be_made() {
        std::thread::spawn(|| {
            let spec = Spec {
                parent: 0,
                store: Store {
                    environment: None,
                    user_data: std::env::temp_dir(),
                    profile: "seam-test".to_owned(),
                    private: true,
                },
                javascript: false,
                background: None,
                zoom: 1.0,
                bounds: None,
                visible: false,
                theme: None,
                scripts: Vec::new(),
                trusted_document_script: None,
                text_channel: false,
                grant: None,
                replica: None,
                text: None,
                block_network: false,
                decide: Rc::new(|_| true),
                post: Rc::new(|_| {}),
                take_key: Rc::new(|_, _, _| None),
            };
            assert!(create(&spec).is_err());
        })
        .join()
        .unwrap();
    }
}
