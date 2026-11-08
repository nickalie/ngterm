//! Windows broadcasts delivered to the real hidden window, and malformed power payloads rejected safely.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId, SendMessageW, WM_NULL};

use super::*;

/// Serializes synthetic power, session and setting messages within one child process, whose subscribers live as long as it.
pub(crate) static EVENTS: Mutex<()> = Mutex::new(());

/// Sends to this process's hidden window once its thread has created it.
pub(crate) fn send(message: u32, wparam: usize, lparam: isize) -> isize {
    start().unwrap();
    let window = opened_window();
    // SAFETY: the live window; callers keep any message payload alive until this synchronous call returns.
    unsafe { SendMessageW(window, message, wparam, lparam) }
}

/// The number of `WM_SETTINGCHANGE` handlers installed in this process.
pub(crate) fn setting_handlers() -> usize {
    SETTING_CHANGES.lock().unwrap_or_else(PoisonError::into_inner).len()
}

/// This process's hidden window, waited for until its thread has created it.
fn opened_window() -> HWND {
    let class: Vec<u16> = CLASS.encode_utf16().chain([0]).collect();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let window = own_window(&class);
        if !window.is_null() {
            return window;
        }
        assert!(Instant::now() < deadline, "the hidden window did not open");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Another test child may own the same class on this desktop: never send it our synthetic messages.
fn own_window(class: &[u16]) -> HWND {
    let mut previous = std::ptr::null_mut();
    loop {
        // SAFETY: NUL-terminated class, null parent/title, and a window returned by the preceding search.
        let window = unsafe { FindWindowExW(std::ptr::null_mut(), previous, class.as_ptr(), std::ptr::null()) };
        if window.is_null() {
            return window;
        }
        let mut process = 0;
        // SAFETY: the discovered window and a writable process id.
        unsafe { GetWindowThreadProcessId(window, &raw mut process) };
        if process == std::process::id() {
            return window;
        }
        previous = window;
    }
}

/// The display payload, including the complete DWORD beyond `POWERBROADCAST_SETTING`'s one-byte trailing array.
#[repr(C)]
struct DisplaySetting {
    guid: windows_sys::core::GUID,
    length: u32,
    state: u32,
}

pub(crate) fn display(state: u32) {
    let setting = DisplaySetting {
        guid: GUID_CONSOLE_DISPLAY_STATE,
        length: 4,
        state,
    };
    send(
        WM_POWERBROADCAST,
        PBT_POWERSETTINGCHANGE as usize,
        (&raw const setting).expose_provenance().cast_signed(),
    );
}

/// Runs a process-wide subscription test without callbacks left by another GPUI test context.
pub(crate) fn run_child(name: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") {
            child.env_remove(key);
        }
    }
    let output = child
        .args(["--exact", name, "--ignored", "--nocapture", "--test-threads=1"])
        .env("W_GPUI_SYSTEM_CHILD", name)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{name}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

pub(crate) fn is_child(name: &str) -> bool {
    std::env::var("W_GPUI_SYSTEM_CHILD").as_deref() == Ok(name)
}

#[test]
fn session_and_setting_broadcasts_are_delivered_in_an_isolated_process() {
    run_child("system_window::tests::broadcasts_reach_every_subscriber_and_a_cancelled_session_does_not_end");
}

#[test]
#[ignore = "run by session_and_setting_broadcasts_are_delivered_in_an_isolated_process"]
fn broadcasts_reach_every_subscriber_and_a_cancelled_session_does_not_end() {
    if !is_child("system_window::tests::broadcasts_reach_every_subscriber_and_a_cancelled_session_does_not_end") {
        return;
    }
    let _events = EVENTS.lock().unwrap();
    let (settings, changed) = mpsc::channel();
    for subscriber in 0..2 {
        let settings = settings.clone();
        on_setting_change(move |action| {
            let _ = settings.send((subscriber, action));
        })
        .unwrap();
    }
    let (ended, ends) = mpsc::channel();
    on_session_end(move || {
        let _ = ended.send(());
    })
    .unwrap();
    assert_eq!(
        on_session_end(|| {}).unwrap_err().to_string(),
        "the session's end has its handler already"
    );
    assert_eq!(send(WM_QUERYENDSESSION, 0, 0), 1);
    assert_eq!(send(WM_ENDSESSION, 0, 0), 0);
    assert!(ends.try_recv().is_err(), "a cancelled logoff must not quit");
    assert_eq!(send(WM_ENDSESSION, 1, 0), 0);
    ends.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(send(WM_SETTINGCHANGE, 0x7fff, 0), 0);
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let event = changed.recv_timeout(Duration::from_secs(2)).unwrap();
        if event.1 == 0x7fff {
            seen.push(event);
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, [(0, 0x7fff), (1, 0x7fff)]);
    assert_eq!(send(WM_NULL, 0, 0), 0, "unhandled messages use DefWindowProcW");
    // The first run owns the class: a second registration reports the OS error rather than pumping messages.
    assert_eq!(run().unwrap_err().raw_os_error(), Some(1410));
}

#[test]
fn power_broadcasts_are_delivered_in_an_isolated_process() {
    run_child("system_window::tests::power_broadcasts_report_resume_and_the_full_display_dword");
}

#[test]
#[ignore = "run by power_broadcasts_are_delivered_in_an_isolated_process"]
fn power_broadcasts_report_resume_and_the_full_display_dword() {
    if !is_child("system_window::tests::power_broadcasts_report_resume_and_the_full_display_dword") {
        return;
    }
    let _events = EVENTS.lock().unwrap();
    let (power, received) = mpsc::channel();
    on_power(move |event| {
        let _ = power.send(event);
    })
    .unwrap();
    assert_eq!(send(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC as usize, 0), 1);
    display(0x1234_5678);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    while !seen.contains(&Power::Display(0x1234_5678)) {
        seen.push(
            received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap(),
        );
    }
    assert!(seen.contains(&Power::Resumed));
    assert_eq!(
        send(WM_POWERBROADCAST, 0xffff, 0),
        1,
        "unknown power events are acknowledged"
    );
}

#[test]
fn irrelevant_or_short_power_payloads_have_no_event() {
    let mut setting = DisplaySetting {
        guid: GUID_CONSOLE_DISPLAY_STATE,
        length: 3,
        state: 1,
    };
    assert_eq!(parsed(PBT_POWERSETTINGCHANGE as usize, Some(&setting)), None);
    setting.length = 4;
    setting.guid.data1 ^= 1;
    assert_eq!(parsed(PBT_POWERSETTINGCHANGE as usize, Some(&setting)), None);
    assert_eq!(parsed(PBT_POWERSETTINGCHANGE as usize, None), None);
    assert_eq!(parsed(usize::MAX, None), None);
    assert_eq!(parsed(0xffff, None), None);
}

fn parsed(wparam: usize, setting: Option<&DisplaySetting>) -> Option<Power> {
    let lparam = setting.map_or(0, |setting| {
        std::ptr::from_ref(setting).expose_provenance().cast_signed()
    });
    // SAFETY: a null pointer or a complete payload kept alive by the borrowed setting.
    unsafe { power_event(wparam, lparam) }
}

#[test]
fn the_hidden_window_pump_ends_when_its_thread_receives_quit() {
    run_child("system_window::tests::hidden_window_child");
}

#[test]
#[ignore = "run by the_hidden_window_pump_ends_when_its_thread_receives_quit"]
fn hidden_window_child() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, PostThreadMessageW, WM_QUIT};

    if !is_child("system_window::tests::hidden_window_child") {
        return;
    }
    let pump = thread::spawn(run);
    let window = opened_window();
    // SAFETY: the live window and a message with no payload, queued for the pump to dispatch.
    assert_ne!(unsafe { PostMessageW(window, WM_NULL, 0, 0) }, 0);
    // SAFETY: the live window; a null output pointer asks only for its owning thread.
    let owner = unsafe { GetWindowThreadProcessId(window, std::ptr::null_mut()) };
    assert_ne!(owner, 0);
    // SAFETY: the pump's thread and its queue, created by the window; WM_QUIT has no pointer payload.
    assert_ne!(unsafe { PostThreadMessageW(owner, WM_QUIT, 0, 0) }, 0);
    pump.join().unwrap().unwrap();
}
