//! The banners on Windows: toasts through the Windows Runtime's `ToastNotificationManager` (the `windows` crate), under the app id
//! the instance runs as, which is the toasts' `AppUserModelID`.
//!
//! - **The app id**: an app that is not packaged shows toasts only under an `AppUserModelID` Windows knows. ngterm
//!   registers its own at every launch, before any window opens ([`register_app_id`]): the process's explicit
//!   `AppUserModelID` (so its windows group under it on the taskbar) and the per-user registration
//!   `HKCU\Software\Classes\AppUserModelId\<app id>` with the name and the icon the toasts show (`DisplayName`,
//!   `IconUri`, the bundled icon written to the state directory). A build run from its folder (a dev build, a copy run
//!   without installing) shows toasts through that alone; the installer's Start-menu shortcut carries the same id
//!   (`System.AppUserModel.ID`), so the shortcut, the taskbar and the toasts are one app. The registration is
//!   rewritten on each launch, so a moved build points at its own icon; an instance under another app id (a test's)
//!   registers that id.
//! - **Slots**: each banner's identity is its toast's group and tag ([`super::toast::slot`]): a repeat in one slot
//!   replaces the toast there (the same group and tag), so a pane's banner never replaces a note and a newer note
//!   does. A withdrawal removes the slot's toast from the screen and the notification center
//!   (`ToastNotificationHistory.Remove`).
//! - **Click**: the toast's `Activated` event, while the app runs, reveals the pane that fired and raises its window
//!   (the core's reveal, [`super::Click`] with no token: Windows hands the foreground to the app the user clicked a
//!   toast of). A toast left in the notification center after the app quit opens nothing: no COM activator is
//!   registered, as no freedesktop server reaches an app that is gone either. A toast the user dismissed is
//!   forgotten.
//! - **Sound**: the toasts are silent; the configured notification sound plays beside them through the status
//!   sounds' player ([`crate::status_sound`]), as on Linux.
//!
//! The toasts are posted on a thread of their own in the multithreaded apartment, in the order the core queued them;
//! the `Activated` and `Dismissed` events arrive on the Windows Runtime's threads.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use ::windows::Data::Xml::Dom::XmlDocument;
use ::windows::Foundation::TypedEventHandler;
use ::windows::UI::Notifications::{
    ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationHistory,
    ToastNotificationManager, ToastNotifier,
};
use ::windows::core::{HSTRING, IInspectable};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegSetValueExW,
};
use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

use super::toast::{self, Slot};
use super::{APP_NAME, Banner, Click};

/// Where Windows reads an unpackaged app's `AppUserModelID` registration, under `HKEY_CURRENT_USER`.
const REGISTRATION_KEY: &str = r"Software\Classes\AppUserModelId";

/// The icon the registration names, written to the state directory.
const ICON_FILE: &str = "notification-icon.png";

/// Makes `app_id` this process's `AppUserModelID` and registers it for the user with ngterm's name and icon (the
/// module docs say why). Before any window opens; a failure is logged and leaves the toasts unshown.
pub fn register_app_id(app_id: &str, state_dir: &Path) {
    let wide = wide(app_id);
    // SAFETY: a NUL-terminated wide string alive for the call.
    let set = unsafe { SetCurrentProcessExplicitAppUserModelID(wide.as_ptr()) };
    if set < 0 {
        tracing::warn!(app_id, hresult = set, "notify: the process's app id was not set");
    }
    let icon = state_dir.join(ICON_FILE);
    let icon = match std::fs::create_dir_all(state_dir).and_then(|()| std::fs::write(&icon, crate::app_icon::PNG)) {
        Ok(()) => Some(icon),
        Err(error) => {
            tracing::warn!(%error, path = %icon.display(), "notify: the toasts' icon was not written");
            None
        }
    };
    match write_registration(app_id, icon.as_deref()) {
        Ok(()) => tracing::info!(app_id, "notify: the toasts' app id is registered"),
        Err(error) => tracing::warn!(%error, app_id, "notify: the toasts' app id was not registered; no toast shows"),
    }
}

/// `HKCU\Software\Classes\AppUserModelId\<app_id>`: `DisplayName`, and `IconUri` when the icon was written.
fn write_registration(app_id: &str, icon: Option<&Path>) -> io::Result<()> {
    let path = wide(&format!(r"{REGISTRATION_KEY}\{app_id}"));
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: a NUL-terminated key path and an out-pointer for the opened key, closed below.
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &raw mut key,
            std::ptr::null_mut(),
        )
    };
    if created != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(created.cast_signed()));
    }
    let mut outcome = set_string(key, "DisplayName", APP_NAME);
    if let (Ok(()), Some(icon)) = (&outcome, icon) {
        outcome = set_string(key, "IconUri", &icon.to_string_lossy());
    }
    // SAFETY: the key opened above, closed once.
    unsafe { RegCloseKey(key) };
    outcome
}

fn set_string(key: HKEY, name: &str, value: &str) -> io::Result<()> {
    let name = wide(name);
    let value = wide(value);
    let bytes = u32::try_from(value.len() * 2).map_err(io::Error::other)?;
    // SAFETY: an open key, a NUL-terminated name, and the value's bytes with their NUL, alive for the call.
    let set = unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr().cast(), bytes) };
    if set == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(set.cast_signed()))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The banner task: hands `banners` in order to the toast thread, which posts them under `app_id`, a click calling
/// `clicked`; ends when every sink is gone.
pub async fn run(
    banners: async_channel::Receiver<Banner>,
    app_id: String,
    clicked: impl Fn(Click) + Send + Sync + 'static,
) {
    let clicked: Clicked = Arc::new(clicked);
    let (toasts, queue) = std::sync::mpsc::channel();
    let started = std::thread::Builder::new()
        .name("ngterm-toasts".to_owned())
        .spawn(move || post_toasts(&queue, &app_id, &clicked));
    if let Err(error) = started {
        tracing::warn!(%error, "notify: the toast thread did not start; banners are not shown");
    }
    while let Ok(banner) = banners.recv().await {
        if toasts.send(banner).is_err() {
            tracing::debug!("notify: the toast thread is gone");
        }
    }
}

type Clicked = Arc<dyn Fn(Click) + Send + Sync>;

/// The toasts on screen or in the notification center, by identity: kept, since a toast's events reach the app only
/// while its object lives.
type Shown = Arc<Mutex<HashMap<String, ToastNotification>>>;

/// The toast thread's notifier and history, made once.
struct Center {
    app_id: HSTRING,
    notifier: ToastNotifier,
    history: ToastNotificationHistory,
    shown: Shown,
}

fn post_toasts(banners: &std::sync::mpsc::Receiver<Banner>, app_id: &str, clicked: &Clicked) {
    // SAFETY: once, on this thread of our own, which never leaves the apartment.
    let joined = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED.cast_unsigned()) };
    if joined < 0 {
        tracing::warn!(hresult = joined, "notify: the toast thread joined no apartment");
    }
    let center = match Center::open(app_id) {
        Ok(center) => Some(center),
        Err(error) => {
            tracing::warn!(%error, app_id, "notify: no toast notifier; banners are not shown");
            None
        }
    };
    while let Ok(banner) = banners.recv() {
        let Some(center) = &center else { continue };
        match banner {
            Banner::Send { identity, title, body } => {
                if let Err(error) = center.show(&identity, &title, &body, clicked) {
                    tracing::warn!(%error, %identity, "notify: the toast was not shown");
                }
            }
            Banner::Withdraw { identity } => center.withdraw(&identity),
        }
    }
}

impl Center {
    fn open(app_id: &str) -> ::windows::core::Result<Self> {
        let app_id = HSTRING::from(app_id);
        Ok(Self {
            notifier: ToastNotificationManager::CreateToastNotifierWithId(&app_id)?,
            history: ToastNotificationManager::History()?,
            app_id,
            shown: Shown::default(),
        })
    }

    /// Shows the toast of `identity` in its slot, replacing the one there.
    fn show(&self, identity: &str, title: &str, body: &str, clicked: &Clicked) -> ::windows::core::Result<()> {
        let content = XmlDocument::new()?;
        content.LoadXml(&HSTRING::from(toast::xml(title, body)))?;
        let toast = ToastNotification::CreateToastNotification(&content)?;
        let Slot { group, tag } = toast::slot(identity);
        toast.SetGroup(&HSTRING::from(group))?;
        toast.SetTag(&HSTRING::from(tag))?;
        let click = Arc::clone(clicked);
        let clicked_identity = identity.to_owned();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                tracing::info!(identity = %clicked_identity, "notify: toast clicked");
                click(Click {
                    identity: clicked_identity.clone(),
                    token: None,
                });
                Ok(())
            },
        ))?;
        let shown = Arc::clone(&self.shown);
        let dismissed_identity = identity.to_owned();
        toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
            move |sender, args| {
                let reason = args.as_ref().and_then(|args| args.Reason().ok());
                tracing::debug!(identity = %dismissed_identity, ?reason, "notify: toast dismissed");
                // one that timed out stays in the notification center, clickable
                if reason == Some(ToastDismissalReason::UserCanceled) {
                    forget(&shown, &dismissed_identity, sender.as_ref());
                }
                Ok(())
            },
        ))?;
        self.notifier.Show(&toast)?;
        self.shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(identity.to_owned(), toast);
        tracing::debug!(identity, "notify: toast shown");
        Ok(())
    }

    /// Removes the toast of `identity`'s slot from the screen and the notification center, if one is there.
    fn withdraw(&self, identity: &str) {
        let toast = self
            .shown
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(identity);
        if toast.is_none() {
            // none shown by this run: its slot may still hold one an earlier run left
            tracing::trace!(identity, "notify: withdrawing a toast this run did not show");
        }
        let Slot { group, tag } = toast::slot(identity);
        if let Err(error) =
            self.history
                .RemoveGroupedTagWithId(&HSTRING::from(tag), &HSTRING::from(group), &self.app_id)
        {
            tracing::debug!(%error, identity, "notify: nothing to withdraw");
        }
    }
}

/// Forgets the toast of `identity` when it is still `toast` (a later one may have replaced it).
fn forget(shown: &Shown, identity: &str, toast: Option<&ToastNotification>) {
    let mut shown = shown.lock().unwrap_or_else(PoisonError::into_inner);
    if toast.is_some() && shown.get(identity) == toast {
        shown.remove(identity);
    }
}
