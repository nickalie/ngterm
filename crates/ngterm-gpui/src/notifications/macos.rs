//! `UserNotifications` banners under the app bundle's identity.
use super::{Banner, Click};
use std::ffi::{CStr, CString, c_char};
use std::sync::OnceLock;

unsafe extern "C" {
    fn ngterm_notifications_start(clicked: extern "C" fn(*const c_char)) -> bool;
    fn ngterm_notify(identity: *const c_char, title: *const c_char, body: *const c_char);
    fn ngterm_withdraw(identity: *const c_char);
}
/// The native side of the banners. A parameter of [`run_on`] because `UserNotifications` needs the `.app` bundle's
/// identity, which a test binary does not have.
#[derive(Clone, Copy)]
struct Center {
    start: unsafe extern "C" fn(extern "C" fn(*const c_char)) -> bool,
    notify: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char),
    withdraw: unsafe extern "C" fn(*const c_char),
}
const NATIVE: Center = Center {
    start: ngterm_notifications_start,
    notify: ngterm_notify,
    withdraw: ngterm_withdraw,
};
static CLICKS: OnceLock<async_channel::Sender<String>> = OnceLock::new();
extern "C" fn clicked(identity: *const c_char) {
    // SAFETY: the delegate passes a live NUL-terminated identifier for this call.
    let identity = unsafe { CStr::from_ptr(identity) }.to_string_lossy().into_owned();
    if let Some(sender) = CLICKS.get() {
        let _ = sender.try_send(identity);
    }
}

pub async fn run(
    banners: async_channel::Receiver<Banner>,
    _: String,
    on_click: impl Fn(Click) + Send + Sync + 'static,
) {
    let (sender, clicks) = async_channel::unbounded();
    let _ = CLICKS.set(sender);
    run_on(NATIVE, banners, clicks, on_click).await;
}

/// The banner task over `center`: nothing without a bundle's identity, else `banners` in order and `clicks` back to
/// `on_click`, until the banners end.
async fn run_on(
    center: Center,
    banners: async_channel::Receiver<Banner>,
    clicks: async_channel::Receiver<String>,
    on_click: impl Fn(Click) + Send + Sync + 'static,
) {
    // SAFETY: the native delegate retains only a static callback; UN supports background posting.
    if !unsafe { (center.start)(clicked) } {
        tracing::warn!("desktop notifications require launching ngterm from its .app bundle");
        return;
    }
    let deliver = async {
        while let Ok(banner) = banners.recv().await {
            let string = |value: String| CString::new(value.replace('\0', "")).unwrap_or_default();
            match banner {
                Banner::Send { identity, title, body } => {
                    let (identity, title, body) = (string(identity), string(title), string(body));
                    // SAFETY: strings live through the call, which copies their values.
                    unsafe { (center.notify)(identity.as_ptr(), title.as_ptr(), body.as_ptr()) };
                }
                Banner::Withdraw { identity } => {
                    let identity = string(identity);
                    // SAFETY: the adapter copies this identifier during the call.
                    unsafe { (center.withdraw)(identity.as_ptr()) };
                }
            }
        }
    };
    let receive = async {
        while let Ok(identity) = clicks.recv().await {
            on_click(Click { identity, token: None });
        }
    };
    futures_lite::future::race(deliver, receive).await;
}

#[cfg(test)]
#[path = "macos_tests.rs"]
mod tests;
