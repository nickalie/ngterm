//! Disposable HKCU registrations and real `WinRT` toast objects, with a notifier handoff for synthetic events.

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
        let deleted = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
        assert!(
            deleted == ERROR_SUCCESS || deleted == 2,
            "test registration cleanup: {deleted}"
        );
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
    assert_eq!(process_app_id(), registration.0);
    assert_eq!(std::fs::read(state.join(ICON_FILE)).unwrap(), crate::app_icon::PNG);
    assert_eq!(registration.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(
        registration.read("IconUri"),
        Some(state.join(ICON_FILE).to_string_lossy().into_owned())
    );
    std::fs::write(state.join(ICON_FILE), "old icon").unwrap();
    register_app_id(&registration.0, &state);
    assert_eq!(std::fs::read(state.join(ICON_FILE)).unwrap(), crate::app_icon::PNG);
    let no_icon = Registration::new();
    let blocked = dir.path().join("file");
    std::fs::write(&blocked, "not a directory").unwrap();
    register_app_id(&no_icon.0, &blocked);
    assert_eq!(no_icon.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(no_icon.read("IconUri"), None);
    let unwritable_icon = Registration::new();
    let icon_directory = dir.path().join("icon-directory");
    std::fs::create_dir_all(icon_directory.join(ICON_FILE)).unwrap();
    register_app_id(&unwritable_icon.0, &icon_directory);
    assert_eq!(unwritable_icon.read("DisplayName").as_deref(), Some(APP_NAME));
    assert_eq!(unwritable_icon.read("IconUri"), None);
    register_app_id(&no_icon.0, &blocked);
    registration_failures(&no_icon.0, dir.path());
    // SAFETY: this child owns its test thread's COM apartment until every toast object is dropped.
    assert!(unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED.cast_unsigned()) } >= 0);
    toast_round_trip(&registration.0);
    notifier_handoff(&registration.0);
    drop(unwritable_icon);
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
    let replacement = center.shown.lock().unwrap()[identity].clone();
    assert_ne!(replacement, first, "the second toast replaced the first");
    forget(&center.shown, identity, Some(&first));
    assert_eq!(
        center.shown.lock().unwrap().get(identity),
        Some(&replacement),
        "a late dismissal cannot remove the replacement"
    );
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
    hidden_and_retained(center, toast, identity, &reason);
}

/// Waits for the report of a `Hide`. `TimedOut` is a separate report that the toast's banner ended, within milliseconds
/// when Windows gives it none, so it can arrive first; the toast stays retained through both.
fn hidden_and_retained(
    center: &Center,
    toast: &ToastNotification,
    identity: &str,
    reasons: &std::sync::mpsc::Receiver<ToastDismissalReason>,
) {
    loop {
        let reason = reasons.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            center.shown.lock().unwrap().get(identity),
            Some(toast),
            "only a user's cancellation forgets the retained notification"
        );
        if reason == ToastDismissalReason::ApplicationHidden {
            return;
        }
        assert_eq!(reason, ToastDismissalReason::TimedOut);
    }
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
    let invalid = format!("{}.{}", long.0, "x".repeat(100));
    register_app_id(&invalid, &state);
    assert_eq!(std::fs::read(state.join(ICON_FILE)).unwrap(), crate::app_icon::PNG);
    let path = wide(&format!(r"{REGISTRATION_KEY}\{invalid}"));
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

fn notifier_handoff(app_id: &str) {
    let cases: [fn(&Center); 5] = [
        a_toast_without_a_banner_stays_retained_through_its_timeout_and_hide,
        notifier_receives_real_xml_slots_and_activation_reveals_the_original_identity,
        only_cancelling_the_current_toast_forgets_it_and_other_slots_survive,
        refused_show_preserves_the_previous_toast_and_returns_the_notifier_error,
        withdraw_removes_the_exact_slot_even_when_unknown_or_the_history_refuses_it,
    ];
    for case in cases {
        let center = Center::open(app_id).unwrap();
        case(&center);
    }
}

/// What an interactive notifier receives, kept alive as Windows would keep its event subscriptions.
struct Posted {
    toast: ToastNotification,
    activated: Activated,
    dismissed: Dismissed,
}

fn post(center: &Center, identity: &str, clicked: &Clicked) -> Posted {
    let mut posted = None;
    center
        .show_with(
            identity,
            "Title <通知>",
            "Body & \"quoted\"",
            clicked,
            |toast, activated, dismissed| {
                posted = Some(Posted {
                    toast: toast.clone(),
                    activated,
                    dismissed,
                });
                Ok(())
            },
        )
        .unwrap();
    posted.unwrap()
}

/// A toast Windows gives no banner reports `TimedOut` right after `Show`, before any `Hide`, and stays in the
/// notification center. Windows no longer lists it once `Hide` is reported, so from there only the app's own record is
/// checked. The handler attached before `Show` cannot miss the first report, and it waits for `show_with` to retain the
/// toast, because forgetting a toast not yet retained would change nothing.
fn a_toast_without_a_banner_stays_retained_through_its_timeout_and_hide(center: &Center) {
    let clicked: Clicked = Arc::new(|_| panic!("no toast was clicked"));
    let identity = "note:no-banner";
    let (reported, reasons) = std::sync::mpsc::channel();
    let (retained, retained_yet) = std::sync::mpsc::channel::<()>();
    center
        .show_with(
            identity,
            "No banner",
            "body",
            &clicked,
            |toast, _activated, dismissed| {
                toast.SetSuppressPopup(true)?;
                toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
                    move |sender, args| {
                        let _ = retained_yet.recv_timeout(Duration::from_secs(5));
                        let reason = args.as_ref().unwrap().Reason()?;
                        dismissed(sender.as_ref(), Some(reason));
                        let _ = reported.send(reason);
                        Ok(())
                    },
                ))?;
                center.notifier.Show(toast)
            },
        )
        .unwrap();
    drop(retained);
    let toast = center.shown.lock().unwrap()[identity].clone();
    assert_eq!(
        reasons.recv_timeout(Duration::from_secs(5)).unwrap(),
        ToastDismissalReason::TimedOut
    );
    assert_eq!(
        center.shown.lock().unwrap().get(identity),
        Some(&toast),
        "a timed-out toast stays clickable"
    );
    wait_history(center, identity, true);
    center.notifier.Hide(&toast).unwrap();
    hidden_and_retained(center, &toast, identity, &reasons);
}

fn notifier_receives_real_xml_slots_and_activation_reveals_the_original_identity(center: &Center) {
    let (sender, clicks) = std::sync::mpsc::channel();
    let clicked: Clicked = Arc::new(move |click| sender.send(click).unwrap());
    let identity = "9caf35d8-1b14-44ca-a489-8a110c8d00f5:session:3";
    let posted = post(center, identity, &clicked);
    assert_eq!(posted.toast.Group().unwrap(), "9caf35d8-1b14-44ca-a489-8a110c8d00f5");
    assert_eq!(posted.toast.Tag().unwrap(), "session:3");
    let xml = posted.toast.Content().unwrap();
    let texts = xml.GetElementsByTagName(&HSTRING::from("text")).unwrap();
    assert_eq!(texts.Length().unwrap(), 2);
    assert_eq!(texts.Item(0).unwrap().InnerText().unwrap(), "Title <通知>");
    assert_eq!(texts.Item(1).unwrap().InnerText().unwrap(), "Body & \"quoted\"");
    let audio = xml.SelectSingleNode(&HSTRING::from("/toast/audio/@silent")).unwrap();
    assert_eq!(audio.InnerText().unwrap(), "true");
    for _ in 0..2 {
        (posted.activated)();
        let click = clicks.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(click.identity, identity);
        assert_eq!(click.token, None);
    }
}

fn only_cancelling_the_current_toast_forgets_it_and_other_slots_survive(center: &Center) {
    let clicked: Clicked = Arc::new(|_| panic!("no click was requested"));
    let first = post(center, "note:replaced", &clicked);
    let other = post(center, "note:other", &clicked);
    let replacement = post(center, "note:replaced", &clicked);
    assert_ne!(first.toast, replacement.toast);
    assert_eq!(replacement.toast.Group().unwrap(), "ngterm");
    assert_eq!(replacement.toast.Tag().unwrap(), "note:replaced");
    for reason in [
        None,
        Some(ToastDismissalReason::TimedOut),
        Some(ToastDismissalReason::ApplicationHidden),
    ] {
        (replacement.dismissed)(Some(&replacement.toast), reason);
        assert_eq!(
            center.shown.lock().unwrap().get("note:replaced"),
            Some(&replacement.toast)
        );
    }
    let canceled = Some(ToastDismissalReason::UserCanceled);
    (first.dismissed)(Some(&first.toast), canceled);
    (replacement.dismissed)(None, canceled);
    (replacement.dismissed)(Some(&other.toast), canceled);
    assert_eq!(
        center.shown.lock().unwrap().get("note:replaced"),
        Some(&replacement.toast)
    );
    (replacement.dismissed)(Some(&replacement.toast), canceled);
    (replacement.dismissed)(Some(&replacement.toast), canceled);
    let shown = center.shown.lock().unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown.get("note:other"), Some(&other.toast));
}

fn refused_show_preserves_the_previous_toast_and_returns_the_notifier_error(center: &Center) {
    let clicked: Clicked = Arc::new(|_| panic!("a refused toast cannot be clicked"));
    let previous = post(center, "note:kept", &clicked);
    for identity in ["note:kept", "note:new"] {
        let error = center
            .show_with(identity, "Replacement", "refused", &clicked, |_, _, _| {
                Err(::windows::core::Error::from_hresult(::windows::core::HRESULT(
                    0x8007_0005_u32.cast_signed(),
                )))
            })
            .unwrap_err();
        assert_eq!(error.code().0.cast_unsigned(), 0x8007_0005);
    }
    let shown = center.shown.lock().unwrap();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown.get("note:kept"), Some(&previous.toast));
}

fn withdraw_removes_the_exact_slot_even_when_unknown_or_the_history_refuses_it(center: &Center) {
    let clicked: Clicked = Arc::new(|_| panic!("no click was requested"));
    let _posted = post(center, "note:withdrawn", &clicked);
    let other = post(center, "note:other", &clicked);
    for identity in ["note:withdrawn", "note:withdrawn", "note:from-an-earlier-run"] {
        let mut removed = false;
        center.withdraw_with(identity, |slot, app_id| {
            assert_eq!(slot.group, "ngterm");
            assert_eq!(slot.tag, identity);
            assert_eq!(app_id, &center.app_id);
            removed = true;
            Err(::windows::core::Error::from_hresult(::windows::core::HRESULT(
                0x8007_0005_u32.cast_signed(),
            )))
        });
        assert!(removed, "history removal is attempted even without a retained object");
        let shown = center.shown.lock().unwrap();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown.get("note:other"), Some(&other.toast));
    }
}
