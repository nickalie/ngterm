//! The real hotkey's ownership and message queue, isolated from other tests' desktop registrations.

use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use ngterm_core::keymap::{Modifiers, parse_keybind};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{OpenThread, THREAD_SYNCHRONIZE, WaitForSingleObject};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F19;
use windows_sys::Win32::UI::WindowsAndMessaging::{PM_REMOVE, WM_NULL, WM_QUIT};

use super::*;
use crate::global_hotkey::{GlobalHotkey, win32_keys};
use crate::state::AppState;

const CHILD: &str = "global_hotkey::windows::tests::hotkey_child";
const FLAGS: u32 = win32_keys::MOD_CONTROL | win32_keys::MOD_ALT | win32_keys::MOD_SHIFT;

fn chord(text: &str) -> Chord {
    parse_keybind(text).unwrap().remove(0)
}

#[test]
fn windows_hotkeys_follow_keymap_changes_and_deliver_presses() {
    crate::system_window::tests::run_child(CHILD);
}

/// Owns a waitable handle so the registrar's detached thread is stopped and joined even on a failed assertion.
struct HotkeyThread {
    id: u32,
    handle: HANDLE,
}

impl HotkeyThread {
    fn new(id: u32) -> Self {
        // SAFETY: the registrar has sent the id of its live thread; only waiting is requested.
        let handle = unsafe { OpenThread(THREAD_SYNCHRONIZE, 0, id) };
        assert!(!handle.is_null());
        Self { id, handle }
    }

    fn send(&self, message: u32, parameter: usize) {
        // SAFETY: the thread owns a message queue; these messages contain no pointers.
        assert_ne!(unsafe { PostThreadMessageW(self.id, message, parameter, 0) }, 0);
    }
}

impl Drop for HotkeyThread {
    fn drop(&mut self) {
        // SAFETY: the registrar's queue, told to leave its message loop.
        unsafe { PostThreadMessageW(self.id, WM_QUIT, 0, 0) };
        // SAFETY: the live owned handle; Windows signals it when the thread has ended.
        let waited = unsafe { WaitForSingleObject(self.handle, 5000) };
        // SAFETY: the handle opened above, closed once.
        unsafe { CloseHandle(self.handle) };
        assert_eq!(waited, WAIT_OBJECT_0, "the hotkey thread did not end");
    }
}

/// A second thread can reserve the chord only when the registrar has released it.
fn available(vk: u16) -> bool {
    std::thread::spawn(move || {
        // SAFETY: a thread-owned registration with a test-private id and no HWND.
        let registered = unsafe { RegisterHotKey(std::ptr::null_mut(), 0x613, FLAGS, u32::from(vk)) } != 0;
        if registered {
            // SAFETY: the registration just taken by this thread.
            assert_ne!(unsafe { UnregisterHotKey(std::ptr::null_mut(), 0x613) }, 0);
        }
        registered
    })
    .join()
    .unwrap()
}

fn released(vk: u16) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !available(vk) {
        assert!(Instant::now() < deadline, "virtual key {vk} was not released");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct Queue(mpsc::Sender<MainTask<AppState>>);

impl MainLoop<AppState> for Queue {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.send(task).map_err(|_| MainLoopClosed)
    }

    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        Err(MainLoopClosed)
    }
}

#[gpui_kit::test]
#[ignore = "run by windows_hotkeys_follow_keymap_changes_and_deliver_presses"]
fn hotkey_child(cx: &mut TestAppContext) {
    if !crate::system_window::tests::is_child(CHILD) {
        return;
    }
    held_chords_reserve_and_release_the_os_keys();
    let (sender, tasks) = mpsc::channel();
    let owner: Arc<dyn MainLoop<AppState>> = Arc::new(Queue(sender));
    let poster = Arc::downgrade(&owner);
    let mut hotkey = GlobalHotkey::default();
    cx.update(|cx| hotkey.sync(None, &poster, cx));
    assert!(available(VK_F19));
    let wanted = chord("ctrl+alt+shift+f19");
    cx.update(|cx| hotkey.sync(Some(&wanted), &poster, cx));
    let registrar = hotkey.windows.as_ref().unwrap();
    let thread = HotkeyThread::new(registrar.thread);
    thread.send(WM_HOTKEY, HOTKEY_PARAM + 1);
    thread.send(WM_NULL, 0);
    thread.send(REFRESH, 0);
    thread.send(WM_TIMER, 0);
    thread.send(WM_HOTKEY, HOTKEY_PARAM);
    let mut state = AppState::new(ngterm_core::Core::new(
        ngterm_core::CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("test", None),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState {
            library: ngterm_core::model::WindowLibrary::seeded(r"C:\test"),
            ..ngterm_core::CoreState::default()
        },
    ));
    tasks.recv_timeout(Duration::from_secs(5)).unwrap()(&mut state);
    assert!(state.core.quick_visible(), "WM_HOTKEY posts the quick-terminal toggle");
    assert!(tasks.try_recv().is_err(), "other messages never post a toggle");
    assert!(!available(VK_F19), "the registrar really owns the Windows chord");
    cx.update(|cx| hotkey.sync(Some(&wanted), &poster, cx));
    thread.send(WM_HOTKEY, HOTKEY_PARAM);
    tasks.recv_timeout(Duration::from_secs(5)).unwrap()(&mut state);
    assert!(!state.core.quick_visible(), "a second press hides the panel");
    cx.update(|cx| hotkey.sync(None, &poster, cx));
    released(VK_F19);
    drop(thread);
    hotkey.windows.as_ref().unwrap().set(Some(&wanted));
    assert!(available(VK_F19), "a stopped registrar cannot reserve a chord");
}

fn held_chords_reserve_and_release_the_os_keys() {
    let mut held = Held::default();
    held.refresh();
    assert!(available(VK_F19));
    held.set(Some(&chord("ctrl+alt+shift+f19")));
    assert!(!available(VK_F19));
    held.refresh();
    assert!(!available(VK_F19), "refresh retains an unchanged registration");
    held.set(Some(&chord("ctrl+alt+shift+`")));
    assert!(available(VK_F19), "replacement releases the old key");
    let vk = foreground_virtual_key(0x29).unwrap();
    assert!(
        !available(vk),
        "the physical key is registered in the foreground layout"
    );
    held.set(Some(&Chord::new(Modifiers::CONTROL, "é")));
    assert!(available(vk), "a key without a position clears the registration");
    held.set(None);
    held.unregister();
    assert_eq!(foreground_virtual_key(u32::MAX), None);
    a_taken_chord_is_refused(&mut held);
    foreground_callback_posts_a_layout_refresh();
}

fn a_taken_chord_is_refused(held: &mut Held) {
    let (ready, started) = mpsc::channel();
    let (release, done) = mpsc::channel();
    let owner = std::thread::spawn(move || {
        // SAFETY: this thread owns its private id; no window or callback.
        let registered = unsafe { RegisterHotKey(std::ptr::null_mut(), 0x614, FLAGS, u32::from(VK_F19)) };
        assert_ne!(registered, 0);
        ready.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(10)).unwrap();
        // SAFETY: the registration this thread just made.
        assert_ne!(unsafe { UnregisterHotKey(std::ptr::null_mut(), 0x614) }, 0);
    });
    started.recv_timeout(Duration::from_secs(5)).unwrap();
    held.set(Some(&chord("ctrl+alt+shift+f19")));
    release.send(()).unwrap();
    owner.join().unwrap();
    assert!(
        available(VK_F19),
        "a refused chord takes no registration from its owner"
    );
    held.refresh();
    assert!(
        available(VK_F19),
        "a refused key is retried only when its layout changes"
    );
    held.set(None);
}

fn foreground_callback_posts_a_layout_refresh() {
    // SAFETY: MSG is plain data filled by PeekMessageW.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: peeking creates this thread's queue.
    unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    // SAFETY: the callback reads none of its parameters; it only posts a pointer-free refresh to this thread.
    unsafe {
        foreground_changed(
            std::ptr::null_mut(),
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
        );
    };
    // SAFETY: the local buffer is writable; the filter selects this thread's refresh message.
    let received = unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), REFRESH, REFRESH, PM_REMOVE) };
    assert_ne!(received, 0);
    assert_eq!(message.message, REFRESH);
}

#[test]
fn a_registrar_whose_start_receiver_is_gone_ends_without_calling_back() {
    let (started, receiver) = mpsc::channel();
    drop(receiver);
    let (fired, events) = mpsc::channel();
    std::thread::spawn(move || run(&Mutex::new(None), &|| fired.send(()).unwrap(), &started))
        .join()
        .unwrap();
    assert!(events.try_recv().is_err());
}
