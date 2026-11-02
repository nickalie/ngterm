//! `UserNotifications` banners under the app bundle's identity.
use super::{Banner, Click};
use std::ffi::{CStr, CString, c_char};
use std::sync::OnceLock;

unsafe extern "C" {
    fn ngterm_notifications_start(clicked: extern "C" fn(*const c_char)) -> bool;
    fn ngterm_notify(identity: *const c_char, title: *const c_char, body: *const c_char);
    fn ngterm_withdraw(identity: *const c_char);
}
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
    // SAFETY: the native delegate retains only a static callback; UN supports background posting.
    if !unsafe { ngterm_notifications_start(clicked) } {
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
                    unsafe { ngterm_notify(identity.as_ptr(), title.as_ptr(), body.as_ptr()) };
                }
                Banner::Withdraw { identity } => {
                    let identity = string(identity);
                    // SAFETY: the adapter copies this identifier during the call.
                    unsafe { ngterm_withdraw(identity.as_ptr()) };
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
