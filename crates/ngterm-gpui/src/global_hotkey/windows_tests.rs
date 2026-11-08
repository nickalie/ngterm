//! The real hotkey's ownership and message queue, isolated from other tests' desktop registrations.

use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use ngterm_core::keymap::{Modifiers, parse_keybind};
use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{AttachThreadInput, OpenThread, THREAD_SYNCHRONIZE, WaitForSingleObject};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayoutList, HKL, KLF_NOTELLSHELL, KLF_SETFORPROCESS, LoadKeyboardLayoutW,
    UnloadKeyboardLayout, VK_F19, VK_OEM_3, VK_OEM_5,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, PM_REMOVE, SetForegroundWindow, WM_NULL, WM_QUIT, WS_POPUP, WS_VISIBLE,
};

use super::*;
use crate::global_hotkey::{GlobalHotkey, win32_keys};
use crate::state::AppState;

const CHILD: &str = "global_hotkey::windows::tests::hotkey_child";
const FLAGS: u32 = win32_keys::MOD_CONTROL | win32_keys::MOD_ALT | win32_keys::MOD_SHIFT;
/// The key left of 1, which the layouts below give different virtual keys.
const LEFT_OF_ONE: &str = "ctrl+alt+shift+`";
const LEFT_OF_ONE_SCAN: u32 = 0x29;

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
    the_registrar_moves_the_chord_with_the_foreground_layout();
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
    held.set(Some(&chord(LEFT_OF_ONE)));
    assert!(available(VK_F19), "replacement releases the old key");
    let vk = foreground_virtual_key(LEFT_OF_ONE_SCAN).unwrap();
    assert!(
        !available(vk),
        "the physical key is registered in the foreground layout"
    );
    held.set(Some(&Chord::new(Modifiers::CONTROL, "é")));
    assert!(available(vk), "a key without a position clears the registration");
    held.set(None);
    held.unregister();
    assert_eq!(foreground_virtual_key(u32::MAX), None);
    a_moved_key_replaces_the_stale_registration(&mut held);
    a_taken_chord_is_refused(&mut held);
    foreground_callback_posts_a_layout_refresh();
}

/// A chord left holding another virtual key, as a change of layout leaves it, gets the foreground layout's key instead.
fn a_moved_key_replaces_the_stale_registration(held: &mut Held) {
    held.set(Some(&chord(LEFT_OF_ONE)));
    assert_ne!(held.timer, 0, "a key that follows the layout arms the check");
    let vk = held.tried.unwrap();
    held.unregister();
    // SAFETY: a thread hotkey of this thread under the id `Held` would hold it by.
    let stale = unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, FLAGS, u32::from(VK_F19)) };
    assert_ne!(stale, 0);
    held.tried = Some(VK_F19);
    held.registered = true;
    held.refresh();
    assert!(available(VK_F19), "the key the layout moved away from is released");
    assert!(!available(vk), "the key the layout has there now is registered");
    held.set(Some(&chord("ctrl+alt+shift+f19")));
    assert_eq!(held.timer, 0, "a key every layout agrees on needs no check");
    held.set(None);
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
        "a refused key is not retried while its virtual key stays the same"
    );
    held.set(None);
}

/// Held while this test depends on the foreground: the browser hand-off test's handler console would take it (and the
/// foreground hook would stand in for the check this test watches). Shared with it across processes, released at the end.
fn foreground_lock() -> std::fs::File {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(std::env::temp_dir().join("ngterm-test-foreground.lock"))
        .unwrap();
    file.lock().unwrap();
    file
}

/// A window of this thread in the foreground, so the layout the registrar reads is one this test can change.
struct Foreground(HWND);

impl Foreground {
    fn new() -> Self {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a built-in window class named by a NUL-terminated string; a parentless 1x1 popup, destroyed in Drop.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        Self(window)
    }

    fn holds(&self) -> bool {
        // SAFETY: a plain query.
        unsafe { GetForegroundWindow() == self.0 }
    }

    /// Takes the foreground, or takes it back from whatever raised itself meanwhile.
    fn raise(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.holds() {
            assert!(Instant::now() < deadline, "the test window cannot take the foreground");
            self.try_raise();
            self.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Windows lets a window be raised by a thread that shares its input state with the foreground's.
    fn try_raise(&self) {
        // SAFETY: a plain query.
        let foreground = unsafe { GetForegroundWindow() };
        // SAFETY: a plain query of a window handle; no process id asked for.
        let owner = unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) };
        // SAFETY: a plain query.
        let this = unsafe { GetCurrentThreadId() };
        // SAFETY: both are live thread ids, or the call fails harmlessly on a foreground that has just gone.
        let attached = owner != 0 && owner != this && unsafe { AttachThreadInput(this, owner, 1) } != 0;
        // SAFETY: this thread's own live window.
        unsafe { SetForegroundWindow(self.0) };
        if attached {
            // SAFETY: undoes the attachment made above.
            unsafe { AttachThreadInput(this, owner, 0) };
        }
    }

    /// Answers the window's messages, so Windows does not swap a hung window's ghost into the foreground.
    fn pump(&self) {
        // SAFETY: MSG is plain data the call fills.
        let mut message: MSG = unsafe { std::mem::zeroed() };
        // SAFETY: a valid MSG; only this window's messages are taken and dispatched to its built-in class.
        while unsafe { PeekMessageW(&raw mut message, self.0, 0, 0, PM_REMOVE) } != 0 {
            // SAFETY: the message PeekMessageW filled.
            unsafe { DispatchMessageW(&raw const message) };
        }
    }
}

impl Drop for Foreground {
    fn drop(&mut self) {
        // SAFETY: this thread's own window, destroyed once.
        unsafe { DestroyWindow(self.0) };
    }
}

/// The US and German layouts for this process; what it loaded is unloaded and its layout restored in Drop.
struct Layouts {
    original: HKL,
    added: Vec<HKL>,
}

impl Layouts {
    fn new() -> Self {
        Self {
            // SAFETY: a query of this thread's layout.
            original: unsafe { GetKeyboardLayout(0) },
            added: Vec::new(),
        }
    }

    fn load(&mut self, id: &str) -> HKL {
        let before = installed_layouts();
        let id: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated system layout id; no activation or shell notification is requested.
        let layout = unsafe { LoadKeyboardLayoutW(id.as_ptr(), KLF_NOTELLSHELL) };
        assert!(!layout.is_null());
        if !before.contains(&layout) {
            self.added.push(layout);
        }
        layout
    }
}

impl Drop for Layouts {
    fn drop(&mut self) {
        activate(self.original);
        for layout in &self.added {
            // SAFETY: only layouts this process loaded, after switching away from them.
            unsafe { UnloadKeyboardLayout(*layout) };
        }
    }
}

fn installed_layouts() -> Vec<HKL> {
    // SAFETY: a null buffer of size 0 asks for the count.
    let count = unsafe { GetKeyboardLayoutList(0, std::ptr::null_mut()) };
    let mut layouts = vec![std::ptr::null_mut(); usize::try_from(count).unwrap()];
    // SAFETY: a writable buffer `count` long.
    let written = unsafe { GetKeyboardLayoutList(count, layouts.as_mut_ptr()) };
    layouts.truncate(usize::try_from(written).unwrap());
    layouts
}

fn activate(layout: HKL) {
    // SAFETY: a layout handle loaded in this process.
    assert!(!unsafe { ActivateKeyboardLayout(layout, KLF_SETFORPROCESS) }.is_null());
}

/// Returns once the registrar has handled every message posted before this: it handles them in order, one at a time.
fn settled(thread: &HotkeyThread, pressed: &mpsc::Receiver<()>) {
    thread.send(WM_HOTKEY, HOTKEY_PARAM);
    pressed.recv_timeout(Duration::from_secs(5)).unwrap();
}

/// The registrar reads the layout only on its messages: `REFRESH` here, and the armed check once nothing is posted.
fn the_registrar_moves_the_chord_with_the_foreground_layout() {
    let _foreground = foreground_lock();
    let (us_key, german_key) = (VK_OEM_3, VK_OEM_5);
    let window = Foreground::new();
    let mut layouts = Layouts::new();
    let (us, german) = (layouts.load("00000409"), layouts.load("00000407"));
    activate(us);
    window.raise();
    assert_eq!(foreground_virtual_key(LEFT_OF_ONE_SCAN), Some(us_key));
    let (press, pressed) = mpsc::channel();
    let registrar = Registrar::start(move || press.send(()).unwrap()).unwrap();
    let thread = HotkeyThread::new(registrar.thread);
    registrar.set(Some(&chord(LEFT_OF_ONE)));
    settled(&thread, &pressed);
    assert!(!available(us_key), "the chord starts on the layout's key");
    assert!(available(german_key));

    activate(german);
    loop {
        window.raise();
        thread.send(REFRESH, 0);
        settled(&thread, &pressed);
        if window.holds() {
            break;
        }
    }
    assert!(
        available(us_key),
        "a refresh releases the key the layout moved away from"
    );
    assert!(!available(german_key), "and registers the one it moved to");

    // only the released key is polled: a probe of the arriving key could take it from the registrar
    activate(us);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !available(german_key) {
        assert!(Instant::now() < deadline, "the layout check never ran");
        window.raise();
        window.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
    settled(&thread, &pressed);
    assert!(
        !available(us_key),
        "the check registers the layout's key with nothing posted"
    );
    registrar.set(None);
    settled(&thread, &pressed);
    assert!(available(us_key));
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
