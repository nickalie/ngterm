//! The app's hidden top-level window on Windows, on a thread of its own: what Windows broadcasts to every top-level
//! window (a message-only window gets no broadcast) and no GPUI window hands on: the session's end
//! (`WM_QUERYENDSESSION`/`WM_ENDSESSION`, `signals.rs`), a changed system setting (`WM_SETTINGCHANGE`, the motion
//! switch in `status_glyph.rs`), and the power events (`WM_POWERBROADCAST`, the remote links' resume in
//! `remote/windows.rs`): the resume from sleep and, registered for at the window's start, the console display's state
//! (`GUID_CONSOLE_DISPLAY_STATE`), which also reports a wake from modern standby. It starts with the first handler and
//! lives as long as the process.

use std::io;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Power::{POWERBROADCAST_SETTING, RegisterPowerSettingNotification};
use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DispatchMessageW, GetMessageW, MSG,
    PBT_APMRESUMEAUTOMATIC, PBT_POWERSETTINGCHANGE, RegisterClassW, TranslateMessage, WM_ENDSESSION, WM_POWERBROADCAST,
    WM_QUERYENDSESSION, WM_SETTINGCHANGE, WNDCLASSW, WS_OVERLAPPED,
};

/// The window's class: ngterm's own name.
const CLASS: &str = "ngterm-system-window";

type Handler = Box<dyn Fn(usize) + Send + Sync>;

/// What runs when the session ends, before the window procedure returns (the session may end once it did).
static SESSION_END: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();
/// What runs on each `WM_SETTINGCHANGE`, with its `wParam` (the `SPI_SET*` action of a `SystemParametersInfo` change).
static SETTING_CHANGES: Mutex<Vec<Handler>> = Mutex::new(Vec::new());
/// A power event the window received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    /// The machine resumed from sleep or hibernation (`PBT_APMRESUMEAUTOMATIC`).
    Resumed,
    /// The console display's state: 0 off, 1 on, 2 dimmed; the first report is the state at the window's start.
    Display(u32),
}

/// What runs on each power event.
static POWER: Mutex<Vec<PowerHandler>> = Mutex::new(Vec::new());

type PowerHandler = Box<dyn Fn(Power) + Send + Sync>;
/// The window's thread started, or why it did not.
static STARTED: OnceLock<Result<(), String>> = OnceLock::new();

/// Runs `handler` when the session ends (once: a second one is refused).
///
/// # Errors
///
/// A handler is set already, or the window could not start.
pub fn on_session_end(handler: impl Fn() + Send + Sync + 'static) -> io::Result<()> {
    if SESSION_END.set(Box::new(handler)).is_err() {
        return Err(io::Error::other("the session's end has its handler already"));
    }
    start()
}

/// Runs `handler` on every changed system setting, with the change's `wParam`.
///
/// # Errors
///
/// The window could not start.
pub fn on_setting_change(handler: impl Fn(usize) + Send + Sync + 'static) -> io::Result<()> {
    SETTING_CHANGES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Box::new(handler));
    start()
}

/// Runs `handler` on every power event ([`Power`]).
///
/// # Errors
///
/// The window could not start.
pub fn on_power(handler: impl Fn(Power) + Send + Sync + 'static) -> io::Result<()> {
    POWER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Box::new(handler));
    start()
}

fn power(event: Power) {
    for handler in POWER.lock().unwrap_or_else(PoisonError::into_inner).iter() {
        handler(event);
    }
}

/// The power event a `WM_POWERBROADCAST` carries, if any of [`Power`]'s.
///
/// # Safety
///
/// `lparam` is the message's own: for `PBT_POWERSETTINGCHANGE` a `POWERBROADCAST_SETTING` alive for the call.
unsafe fn power_event(wparam: WPARAM, lparam: LPARAM) -> Option<Power> {
    match u32::try_from(wparam).ok()? {
        PBT_APMRESUMEAUTOMATIC => Some(Power::Resumed),
        PBT_POWERSETTINGCHANGE if lparam != 0 => {
            let setting = std::ptr::with_exposed_provenance::<POWERBROADCAST_SETTING>(lparam.cast_unsigned());
            // SAFETY: the caller's: Windows's setting, alive for the call.
            let header = unsafe { setting.read_unaligned() };
            let display = GUID_CONSOLE_DISPLAY_STATE;
            let guid = header.PowerSetting;
            let same = (guid.data1, guid.data2, guid.data3, guid.data4)
                == (display.data1, display.data2, display.data3, display.data4);
            if !same || header.DataLength < 4 {
                return None;
            }
            let data = setting
                .cast::<u8>()
                .wrapping_add(std::mem::offset_of!(POWERBROADCAST_SETTING, Data))
                .cast::<[u8; 4]>();
            // SAFETY: the setting's data, `DataLength` bytes of it: the display state's DWORD.
            Some(Power::Display(u32::from_ne_bytes(unsafe { data.read_unaligned() })))
        }
        _ => None,
    }
}

fn start() -> io::Result<()> {
    STARTED
        .get_or_init(|| {
            thread::Builder::new()
                .name("ngterm-system-window".to_owned())
                .spawn(|| {
                    if let Err(error) = run() {
                        tracing::error!(%error, "the app's hidden window did not open: the session's end and setting changes go unseen");
                    }
                })
                .map(drop)
                .map_err(|error| error.to_string())
        })
        .clone()
        .map_err(io::Error::other)
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // the session may end: the quit runs once it does
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wparam != 0
                && let Some(handler) = SESSION_END.get()
            {
                handler();
            }
            0
        }
        WM_SETTINGCHANGE => {
            for handler in SETTING_CHANGES.lock().unwrap_or_else(PoisonError::into_inner).iter() {
                handler(wparam);
            }
            0
        }
        WM_POWERBROADCAST => {
            // SAFETY: the message's own parameters.
            if let Some(event) = unsafe { power_event(wparam, lparam) } {
                power(event);
            }
            1
        }
        // SAFETY: the arguments Windows passed this window procedure.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Creates the hidden window and pumps its messages until the process ends.
fn run() -> io::Result<()> {
    let class: Vec<u16> = CLASS.encode_utf16().chain([0]).collect();
    // SAFETY: a null name asks for this executable's module, which lives as long as the process.
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let description = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        // SAFETY: WNDCLASSW is plain data, valid zeroed (no icon, cursor, brush or menu).
        ..unsafe { std::mem::zeroed() }
    };
    // SAFETY: `description` and the class name it points to live through the call.
    if unsafe { RegisterClassW(&raw const description) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the class registered above; a hidden top-level window (never shown) with no parent, menu or data.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    let setting = GUID_CONSOLE_DISPLAY_STATE;
    // SAFETY: this thread's window and a GUID alive for the call; the registration lasts as long as the window.
    let display = unsafe { RegisterPowerSettingNotification(window, &raw const setting, DEVICE_NOTIFY_WINDOW_HANDLE) };
    if display == 0 {
        tracing::debug!(error = %io::Error::last_os_error(), "the app's hidden window gets no display state");
    }
    // SAFETY: MSG is plain data the loop fills.
    let mut message: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: `message` is a valid MSG; a null window takes every message of this thread.
    while unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) } > 0 {
        // SAFETY: the message GetMessageW filled.
        unsafe { TranslateMessage(&raw const message) };
        // SAFETY: as above.
        unsafe { DispatchMessageW(&raw const message) };
    }
    Ok(())
}
