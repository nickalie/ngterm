//! Unpackaged notification registration in disposable HKCU keys, and real `WinRT` toast identities.

use std::time::{Duration, Instant};

use windows_sys::Win32::System::Registry::{KEY_READ, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW};

use super::*;

struct Registration(String);

impl Registration {
    fn new() -> Self {
        Self(format!("ngterm.test.{}", uuid::Uuid::new_v4()))
    }

    fn read(&self, name: &str) -> Option<String> {
        let path = wide(&format!(r"{REGISTRATION_KEY}\{}", self.0));
        let mut key = std::ptr::null_mut();
        // SAFETY: valid output pointer, a NUL-terminated key path, closed below.
        let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &raw mut key) };
        assert_eq!(opened, 0);
        let mut kind = 0;
        let mut buffer = [0u16; 1024];
        let mut bytes = u32::try_from(std::mem::size_of_val(&buffer)).unwrap();
        // SAFETY: the open key and output buffer whose byte capacity is supplied.
        let result = unsafe {
            RegQueryValueExW(
                key,
                wide(name).as_ptr(),
                std::ptr::null(),
                &raw mut kind,
                buffer.as_mut_ptr().cast(),
                &raw mut bytes,
            )
        };
        // SAFETY: the key opened by this read, closed once.
        unsafe { RegCloseKey(key) };
        if result == 2 {
            return None;
        }
        assert_eq!(result, 0);
        assert_eq!(kind, REG_SZ);
        let units = usize::try_from(bytes / 2).unwrap();
        assert_eq!(buffer[units - 1], 0);
        Some(String::from_utf16(&buffer[..units - 1]).unwrap())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(history) = ToastNotificationManager::History() {
            let _ = history.ClearWithId(&HSTRING::from(&self.0));
        }
        let path = wide(&format!(r"{REGISTRATION_KEY}\{}", self.0));
        // SAFETY: only the UUID-named test key and its values are removed.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
    }
}

#[test]
fn registry_strings_round_trip_unicode_and_registration_can_omit_the_icon() {
    let registration = Registration::new();
    write_registration(&registration.0, None).unwrap();
    assert_eq!(registration.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(registration.read("IconUri"), None);
    let icon = Path::new(r"C:\Users\тест\通知.png");
    write_registration(&registration.0, Some(icon)).unwrap();
    assert_eq!(registration.read("IconUri"), Some(icon.to_string_lossy().into_owned()));
    write_registration(&registration.0, Some(Path::new(r"C:\moved.png"))).unwrap();
    assert_eq!(registration.read("IconUri").as_deref(), Some(r"C:\moved.png"));
    let invalid = format!("{}{}", registration.0, "x".repeat(256));
    assert!(
        write_registration(&invalid, None).is_err(),
        "a registry key component cannot exceed 255 characters"
    );
    // SAFETY: the null key is deliberately invalid; no registry data can be changed.
    assert_eq!(
        set_string(std::ptr::null_mut(), "value", "text")
            .unwrap_err()
            .raw_os_error(),
        Some(6)
    );
}

#[test]
fn process_registration_and_toasts_are_isolated_from_other_tests() {
    crate::system_window::tests::run_child("notifications::windows::tests::registration_child");
}

#[test]
#[ignore = "run by process_registration_and_toasts_are_isolated_from_other_tests"]
fn registration_child() {
    if !crate::system_window::tests::is_child("notifications::windows::tests::registration_child") {
        return;
    }
    let registration = Registration::new();
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    register_app_id(&registration.0, &state);
    assert_eq!(std::fs::read(state.join(ICON_FILE)).unwrap(), crate::app_icon::PNG);
    assert_eq!(registration.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(
        registration.read("IconUri"),
        Some(state.join(ICON_FILE).to_string_lossy().into_owned())
    );
    let no_icon = Registration::new();
    let blocked = dir.path().join("file");
    std::fs::write(&blocked, "not a directory").unwrap();
    register_app_id(&no_icon.0, &blocked);
    assert_eq!(no_icon.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(no_icon.read("IconUri"), None);
    registration_failures(&no_icon.0, dir.path());
    // SAFETY: this child owns its test thread's COM apartment until every toast object is dropped.
    assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED.cast_unsigned()) } >= 0);
    toast_round_trip(&registration.0);
    drop(no_icon);
    drop(registration);
    // SAFETY: balances the successful initialization above.
    unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
}

fn contains(center: &Center, identity: &str) -> bool {
    let Slot { group, tag } = toast::slot(identity);
    let history = center.history.GetHistoryWithId(&center.app_id).unwrap();
    (0..history.Size().unwrap()).any(|index| {
        let toast = history.GetAt(index).unwrap();
        toast.Group().unwrap() == group && toast.Tag().unwrap() == tag
    })
}

fn wait_history(center: &Center, identity: &str, expected: bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while contains(center, identity) != expected {
        assert!(
            Instant::now() < deadline,
            "toast history did not reach expected presence {expected}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn toast_round_trip(app_id: &str) {
    let center = Center::open(app_id).unwrap();
    let clicked: Clicked = Arc::new(|_| panic!("no toast was clicked"));
    let identity = "note:coverage";
    center.withdraw(identity);
    center
        .show(identity, "Coverage <one>", "Unicode 通知 & body", &clicked)
        .unwrap();
    wait_history(&center, identity, true);
    let first = center.shown.lock().unwrap()[identity].clone();
    let Slot { group, tag } = toast::slot(identity);
    assert_eq!(first.Group().unwrap(), group);
    assert_eq!(first.Tag().unwrap(), tag);
    let xml = first.Content().unwrap().GetXml().unwrap().to_string();
    assert!(
        xml.contains("Coverage &lt;one&gt;") && xml.contains("Unicode 通知 &amp; body"),
        "{xml}"
    );
    dismissed_but_retained(&center, &first, identity);
    center.show(identity, "Replacement", "newer", &clicked).unwrap();
    wait_history(&center, identity, true);
    forget(&center.shown, identity, Some(&first));
    assert!(
        contains(&center, identity),
        "a late dismissal cannot remove the replacement"
    );
    let replacement = center.shown.lock().unwrap()[identity].clone();
    forget(&center.shown, identity, None);
    assert!(center.shown.lock().unwrap().contains_key(identity));
    forget(&center.shown, identity, Some(&replacement));
    assert!(center.shown.lock().unwrap().is_empty());
    center.withdraw(identity);
    wait_history(&center, identity, false);
    center.withdraw(identity);
    refused_notifier_drains_its_queue();
    queued_banners(app_id, &center);
}

fn queued_banners(app_id: &str, center: &Center) {
    let clicked: Clicked = Arc::new(|_| panic!("an oversized toast cannot be clicked"));
    let oversized = "x".repeat(64 * 1024);
    assert!(
        center
            .show("note:oversized", "Too large", &oversized, &clicked)
            .is_err()
    );
    let (sender, banners) = async_channel::unbounded();
    sender
        .send_blocking(Banner::Send {
            identity: "note:oversized".into(),
            title: "Too large".into(),
            body: oversized,
        })
        .unwrap();
    for identity in ["note:withdrawn", "note:queued"] {
        sender
            .send_blocking(Banner::Send {
                identity: identity.into(),
                title: "Queued".into(),
                body: "body".into(),
            })
            .unwrap();
    }
    sender
        .send_blocking(Banner::Withdraw {
            identity: "note:withdrawn".into(),
        })
        .unwrap();
    drop(sender);
    futures_lite::future::block_on(run(banners, app_id.to_owned(), |_| panic!("no banner was clicked")));
    wait_history(center, "note:queued", true);
    assert!(!contains(center, "note:oversized"));
    wait_history(center, "note:withdrawn", false);
    center.withdraw("note:queued");
    wait_history(center, "note:queued", false);
}

fn dismissed_but_retained(center: &Center, toast: &ToastNotification, identity: &str) {
    let (dismissed, reason) = std::sync::mpsc::channel();
    toast
        .Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
            move |_, args| {
                let _ = dismissed.send(args.as_ref().unwrap().Reason()?);
                Ok(())
            },
        ))
        .unwrap();
    center.notifier.Hide(toast).unwrap();
    assert_eq!(
        reason.recv_timeout(Duration::from_secs(5)).unwrap(),
        ToastDismissalReason::ApplicationHidden
    );
    assert_eq!(
        center.shown.lock().unwrap().get(identity),
        Some(toast),
        "only a user's cancellation forgets the retained notification"
    );
}

fn refused_notifier_drains_its_queue() {
    assert!(
        Center::open("").is_err(),
        "an unpackaged notifier needs an explicit app id"
    );
    let (sender, banners) = std::sync::mpsc::channel();
    sender
        .send(Banner::Withdraw {
            identity: "note:no-notifier".into(),
        })
        .unwrap();
    drop(sender);
    std::thread::spawn(move || {
        use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoUninitialize};
        // SAFETY: a deliberately different apartment on this private test thread; post_toasts must survive the refused MTA join.
        assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED.cast_unsigned()) } >= 0);
        let clicked: Clicked = Arc::new(|_| panic!("no toast can be shown without a notifier"));
        post_toasts(&banners, "", &clicked);
        // SAFETY: balances only this thread's successful STA initialization; the MTA attempt failed.
        unsafe { CoUninitialize() };
    })
    .join()
    .unwrap();
}

fn process_app_id() -> String {
    let mut value = std::ptr::null_mut();
    // SAFETY: an initialized out-pointer; Windows allocates the returned NUL-terminated string for this process.
    let status = unsafe { windows_sys::Win32::UI::Shell::GetCurrentProcessExplicitAppUserModelID(&raw mut value) };
    assert!(status >= 0);
    // SAFETY: a successful getter returned a live NUL-terminated UTF-16 string.
    let id = unsafe { widestring::U16CStr::from_ptr_str(value) }.to_string_lossy();
    // SAFETY: the getter's allocation is freed once, after the string was copied.
    unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(value.cast()) };
    id
}

fn registration_failures(previous: &str, dir: &Path) {
    let long = Registration(format!("{previous}.{}", "x".repeat(160)));
    let state = dir.join("long-id");
    register_app_id(&long.0, &state);
    assert_eq!(
        process_app_id(),
        previous,
        "Windows refuses a process app id longer than 128 characters"
    );
    assert_eq!(
        long.read("DisplayName").as_deref(),
        Some(APP_NAME),
        "the registration still succeeds"
    );
    let invalid = Registration(format!("{}.{}", long.0, "x".repeat(100)));
    register_app_id(&invalid.0, &state);
    assert_eq!(std::fs::read(state.join(ICON_FILE)).unwrap(), crate::app_icon::PNG);
    let path = wide(&format!(r"{REGISTRATION_KEY}\{}", invalid.0));
    let mut key = std::ptr::null_mut();
    // SAFETY: a NUL-terminated test-key path and an out-pointer, closed below if unexpectedly opened.
    let opened = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &raw mut key) };
    if opened == 0 {
        // SAFETY: the unexpected handle opened above is still ours to close before the assertion fails.
        unsafe { RegCloseKey(key) };
    }
    assert_ne!(
        opened, 0,
        "a registry component longer than 255 characters cannot be registered"
    );
}
