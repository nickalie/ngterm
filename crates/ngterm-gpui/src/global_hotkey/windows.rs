//! The global hotkey on Windows: `RegisterHotKey` for this app's thread of the hotkey, which Windows hands every press
//! of the chord, whichever application has the keyboard, and takes from it, as `RegisterEventHotKey` does upstream.
//!
//! `RegisterHotKey` names a virtual key, which the active layout puts on a key: the chord's key is kept as its
//! position ([`KeyPosition`]) and registered as the virtual key the foreground window's layout has there
//! (`MapVirtualKeyExW`), registered again when that changes: on a change of the foreground window
//! (`EVENT_SYSTEM_FOREGROUND`, whose window may have a layout of its own) and on a check each second (a layout switched
//! inside the same window). So the key left of 1 stays the hotkey's on German's layout, where it types `^`, and on
//! Russian's, where it types `ё`. Enter, Tab, Space, Backspace, the arrows and the function keys are the same virtual
//! key on every layout. The autorepeat of a held chord is no new press (`MOD_NOREPEAT`). A chord another application
//! holds is refused: logged, as the X11 grab's.
//!
//! A thread of its own holds the registration, since `RegisterHotKey` posts each press to the queue of the thread that
//! registered it; [`Registrar::set`] hands it the keymap's chord.

use std::io;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};

use ngterm_core::keymap::Chord;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MAPVK_VSC_TO_VK, MapVirtualKeyExW, RegisterHotKey, UnregisterHotKey,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, KillTimer,
    MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_APP,
    WM_HOTKEY, WM_TIMER,
};

use super::win32_keys::{KeyPosition, modifiers, position};

/// The one hotkey's id on its thread, and as `WM_HOTKEY`'s `wParam`.
const HOTKEY_ID: i32 = 1;
const HOTKEY_PARAM: usize = 1;
/// The keymap's chord changed ([`Registrar::set`]).
const SYNC: u32 = WM_APP + 1;
/// The foreground window changed: its layout may put another virtual key on the chord's position.
const REFRESH: u32 = WM_APP + 2;
/// `ERROR_HOTKEY_ALREADY_REGISTERED`: another application holds the chord.
const HOTKEY_TAKEN: i32 = 1409;
/// How often the foreground window's layout is read again while a chord's key follows it.
const LAYOUT_CHECK_MS: u32 = 1000;

/// The hotkey's thread, and the chord it is to hold.
pub struct Registrar {
    thread: u32,
    wanted: Arc<Mutex<Option<Chord>>>,
}

impl Registrar {
    /// Starts the hotkey's thread, which calls `fired` on every press of the chord it holds (none yet).
    ///
    /// # Errors
    ///
    /// The thread did not start.
    pub fn start(fired: impl Fn() + Send + 'static) -> io::Result<Self> {
        let wanted = Arc::new(Mutex::new(None));
        let (started, thread) = mpsc::channel();
        let held = Arc::clone(&wanted);
        std::thread::Builder::new()
            .name("ngterm-global-hotkey".to_owned())
            .spawn(move || run(&held, &fired, &started))?;
        let thread = thread
            .recv()
            .map_err(|_| io::Error::other("the global hotkey's thread ended at its start"))?;
        Ok(Self { thread, wanted })
    }

    /// Registers `chord` in place of the one held before; none for `None` or a key with no position.
    pub fn set(&self, chord: Option<&Chord>) {
        *self.wanted.lock().unwrap_or_else(PoisonError::into_inner) = chord.cloned();
        // SAFETY: a message with no pointers, to the thread's queue, which it made before sending its id.
        if unsafe { PostThreadMessageW(self.thread, SYNC, 0, 0) } == 0 {
            tracing::warn!(error = %io::Error::last_os_error(), "the global hotkey's thread is gone");
        }
    }
}

/// The registration the thread holds.
#[derive(Default)]
struct Held {
    /// The chord's position and `RegisterHotKey` flags.
    chord: Option<(KeyPosition, u32)>,
    /// The virtual key last tried, registered or refused: tried again only once the layout puts another there.
    tried: Option<u16>,
    registered: bool,
    /// The layout check's timer, while the chord follows the layout.
    timer: usize,
}

impl Held {
    fn set(&mut self, chord: Option<&Chord>) {
        self.unregister();
        self.tried = None;
        self.chord = chord.and_then(|chord| {
            let found = position(&chord.key);
            if found.is_none() {
                tracing::warn!(key = chord.key, "the global hotkey's key has no position");
            }
            found.map(|found| (found, modifiers(chord.mods)))
        });
        let follows = matches!(self.chord, Some((KeyPosition::Scan(_), _)));
        if follows && self.timer == 0 {
            // SAFETY: a thread timer (no window, no callback): its `WM_TIMER` comes to this thread's queue.
            self.timer = unsafe { SetTimer(std::ptr::null_mut(), 0, LAYOUT_CHECK_MS, None) };
        } else if !follows && self.timer != 0 {
            // SAFETY: the thread timer set above.
            unsafe { KillTimer(std::ptr::null_mut(), self.timer) };
            self.timer = 0;
        }
        self.refresh();
    }

    /// Registers the chord's key as the virtual key the foreground layout has there, when that is not the one tried.
    fn refresh(&mut self) {
        let Some((key, flags)) = self.chord else {
            return;
        };
        let vk = match key {
            KeyPosition::Fixed(vk) => Some(vk),
            KeyPosition::Scan(scan) => foreground_virtual_key(scan),
        };
        if vk == self.tried {
            return;
        }
        self.unregister();
        self.tried = vk;
        let Some(vk) = vk else {
            tracing::warn!(?key, "the global hotkey's key types nothing on this layout");
            return;
        };
        // SAFETY: a thread hotkey (no window) of this thread, its id this thread's own.
        if unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, flags, u32::from(vk)) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(HOTKEY_TAKEN) {
                tracing::warn!(vk, "the global hotkey is taken by another application");
            } else {
                tracing::warn!(%error, vk, "the global hotkey was not registered");
            }
            return;
        }
        self.registered = true;
        tracing::info!(vk, flags, "global hotkey: registered");
    }

    fn unregister(&mut self) {
        if std::mem::take(&mut self.registered) {
            // SAFETY: the thread hotkey this thread registered.
            unsafe { UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID) };
        }
    }
}

/// The virtual key the foreground window's layout puts on scan code `scan`; `None` when it puts none.
fn foreground_virtual_key(scan: u32) -> Option<u16> {
    // SAFETY: a plain query.
    let window = unsafe { GetForegroundWindow() };
    let thread = if window.is_null() {
        // none in the foreground: thread 0 reads this thread's layout
        0
    } else {
        // SAFETY: a plain query of a window handle; no process id asked for.
        unsafe { GetWindowThreadProcessId(window, std::ptr::null_mut()) }
    };
    // SAFETY: a plain query.
    let layout = unsafe { GetKeyboardLayout(thread) };
    // SAFETY: a plain query of a layout handle.
    let vk = unsafe { MapVirtualKeyExW(scan, MAPVK_VSC_TO_VK, layout) };
    u16::try_from(vk).ok().filter(|vk| *vk != 0)
}

/// The hotkey's thread: makes its queue, says its id, and runs the presses, the chord's changes and the layout checks.
fn run(wanted: &Mutex<Option<Chord>>, fired: &impl Fn(), started: &mpsc::Sender<u32>) {
    // SAFETY: MSG is plain data the calls fill.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: a valid MSG; peeking makes this thread's message queue, so a message posted after the id is sent finds
    // it.
    unsafe { PeekMessageW(&raw mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    // SAFETY: a plain query.
    if started.send(unsafe { GetCurrentThreadId() }).is_err() {
        return;
    }
    // SAFETY: an out-of-context hook with a callback that only posts to this thread; it lives as long as the thread.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_changed),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_null() {
        tracing::debug!("the global hotkey sees no foreground change: its layout is checked each second only");
    }
    let mut held = Held::default();
    // SAFETY: `message` is a valid MSG; a null window takes this thread's messages, its thread messages included.
    while unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        match message.message {
            WM_HOTKEY if message.wParam == HOTKEY_PARAM => fired(),
            SYNC => held.set(wanted.lock().unwrap_or_else(PoisonError::into_inner).as_ref()),
            REFRESH | WM_TIMER if message.hwnd.is_null() => held.refresh(),
            _ => {
                // SAFETY: the message GetMessageW filled.
                unsafe { TranslateMessage(&raw const message) };
                // SAFETY: as above.
                unsafe { DispatchMessageW(&raw const message) };
            }
        }
    }
}

/// `EVENT_SYSTEM_FOREGROUND`, on the hotkey's thread: read the layout again.
unsafe extern "system" fn foreground_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _window: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    // SAFETY: a plain query.
    let thread = unsafe { GetCurrentThreadId() };
    // SAFETY: a message with no pointers to this thread's own queue.
    unsafe { PostThreadMessageW(thread, REFRESH, 0, 0) };
}
