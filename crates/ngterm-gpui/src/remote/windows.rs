//! Windows's link sources:
//!
//! - **Network**: the IP helper's connectivity hint (`NotifyNetworkConnectivityHintChange`, Windows 10 2004 and later,
//!   what the network list manager derives its connectivity from), its first call the state at start: each hint that
//!   differs from the last is reported, its reachability by [`available`]. On a Windows without it (resolved at run
//!   time, so the app still starts there) the network reads as always reachable (logged).
//! - **Resume**: the app's hidden window's power events ([`crate::system_window::on_power`]): the resume from sleep or
//!   hibernation (`PBT_APMRESUMEAUTOMATIC`), and the console display turning on after it was off, which is also how a
//!   wake from modern standby arrives (upstream retries on the screens' wake, `NSWorkspace.screensDidWakeNotification`)
//!   ([`DisplayWake`]).

use std::ffi::c_void;
use std::sync::{Mutex, OnceLock, PoisonError};

use windows_sys::Win32::Foundation::{HANDLE, NO_ERROR, WIN32_ERROR};
use windows_sys::Win32::Networking::WinSock::{
    NL_NETWORK_CONNECTIVITY_HINT, NL_NETWORK_CONNECTIVITY_LEVEL_HINT, NetworkConnectivityLevelHintNone,
};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};

use super::{LinkEvent, Report};
use crate::system_window::{self, Power};

/// Starts the sources for the app's life.
pub fn start(report: &Report) {
    follow_network(report);
    let resume = report.clone();
    let wake = Mutex::new(DisplayWake::default());
    let started = system_window::on_power(move |event| {
        let resumed = match event {
            Power::Resumed => true,
            Power::Display(state) => wake.lock().unwrap_or_else(PoisonError::into_inner).woke(state),
        };
        if resumed {
            resume(LinkEvent::Resumed);
        }
    });
    if let Err(error) = started {
        tracing::warn!(%error, "remote links: no resume source; the links retry on their backoff alone");
    }
}

/// Whether a connectivity hint lets anything past the machine be reached: none is down; local access, internet
/// access, a constrained one, and a hint unknown or hidden are up (the links then try, as `NetworkManager`'s unknown
/// connectivity on a default route does on Linux).
#[must_use]
pub fn available(level: NL_NETWORK_CONNECTIVITY_LEVEL_HINT) -> bool {
    level != NetworkConnectivityLevelHintNone
}

/// The console display's wakes: a report of it on (1) after one of it off (0). Dimmed (2) is neither: the display
/// dims and comes back without the machine sleeping.
#[derive(Debug, Default)]
pub struct DisplayWake {
    off: bool,
}

impl DisplayWake {
    /// Whether display state `state` is a wake.
    pub fn woke(&mut self, state: u32) -> bool {
        match state {
            0 => {
                self.off = true;
                false
            }
            1 => std::mem::take(&mut self.off),
            _ => false,
        }
    }
}

/// The report the connectivity callback calls, and the last hint it reported.
struct Network {
    report: Report,
    last: Mutex<Option<(i32, i32)>>,
}

static NETWORK: OnceLock<Network> = OnceLock::new();

type NotifyHintChange = unsafe extern "system" fn(
    callback: Option<unsafe extern "system" fn(*const c_void, NL_NETWORK_CONNECTIVITY_HINT)>,
    context: *const c_void,
    initial: bool,
    handle: *mut HANDLE,
) -> WIN32_ERROR;

fn follow_network(report: &Report) {
    if NETWORK
        .set(Network {
            report: report.clone(),
            last: Mutex::new(None),
        })
        .is_err()
    {
        return;
    }
    let Some(notify) = notify_hint_change() else {
        tracing::info!(
            "remote links: this Windows has no connectivity hint; the network reads as always reachable and the \
             links retry on their backoff alone"
        );
        return;
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    // SAFETY: a callback that only reads its hint and reports, for the app's life (the handle is never cancelled); no
    // context.
    let status = unsafe { notify(Some(hint_changed), std::ptr::null(), true, &raw mut handle) };
    if status != NO_ERROR {
        tracing::warn!(
            status,
            "remote links: the connectivity hint did not start; the network reads as reachable"
        );
    }
}

/// `NotifyNetworkConnectivityHintChange` from the system's `iphlpapi.dll`, when this Windows has it.
fn notify_hint_change() -> Option<NotifyHintChange> {
    let library: Vec<u16> = "iphlpapi.dll".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: a NUL-terminated name, loaded from System32 only; the module stays loaded for the process.
    let module = unsafe { LoadLibraryExW(library.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return None;
    }
    // SAFETY: a loaded module and a NUL-terminated export name.
    let function = unsafe { GetProcAddress(module, c"NotifyNetworkConnectivityHintChange".as_ptr().cast()) }?;
    // SAFETY: the export of that name has this signature (`netioapi.h`).
    Some(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, NotifyHintChange>(function) })
}

unsafe extern "system" fn hint_changed(_context: *const c_void, hint: NL_NETWORK_CONNECTIVITY_HINT) {
    let Some(network) = NETWORK.get() else {
        return;
    };
    let now = (hint.ConnectivityLevel, hint.ConnectivityCost);
    let changed = network.last.lock().unwrap_or_else(PoisonError::into_inner).replace(now) != Some(now);
    if changed {
        tracing::debug!(
            level = hint.ConnectivityLevel,
            cost = hint.ConnectivityCost,
            "remote links: connectivity hint"
        );
        (network.report)(LinkEvent::Network {
            available: available(hint.ConnectivityLevel),
        });
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::Networking::WinSock::{
        NetworkConnectivityLevelHintConstrainedInternetAccess, NetworkConnectivityLevelHintHidden,
        NetworkConnectivityLevelHintInternetAccess, NetworkConnectivityLevelHintLocalAccess,
        NetworkConnectivityLevelHintUnknown,
    };

    use super::*;

    #[test]
    fn only_a_hint_of_no_connectivity_is_down() {
        assert!(!available(NetworkConnectivityLevelHintNone));
        for up in [
            NetworkConnectivityLevelHintUnknown,
            NetworkConnectivityLevelHintLocalAccess,
            NetworkConnectivityLevelHintInternetAccess,
            NetworkConnectivityLevelHintConstrainedInternetAccess,
            NetworkConnectivityLevelHintHidden,
        ] {
            assert!(available(up), "{up}");
        }
    }

    #[test]
    fn the_display_turning_on_after_it_was_off_is_a_wake() {
        let mut wake = DisplayWake::default();
        assert!(!wake.woke(1), "on at the start is no wake");
        assert!(!wake.woke(2), "dimmed");
        assert!(!wake.woke(1), "back from dimmed is no wake");
        assert!(!wake.woke(0), "off");
        assert!(!wake.woke(2), "dimmed while off");
        assert!(wake.woke(1), "on after off");
        assert!(!wake.woke(1), "a repeat is no second wake");
    }

    #[test]
    fn the_hint_function_is_there_on_this_windows() {
        assert!(notify_hint_change().is_some(), "Windows 10 2004 and later have it");
    }
}
