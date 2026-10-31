//! The system's request to end the process quits it with the quit save, through the main loop: on Unix SIGTERM,
//! SIGINT and SIGHUP (a service manager's stop, Ctrl+C, the terminal that started it closing), waited for on a thread
//! of its own, a second one of the same signal getting the default action (the GPUI frontend's `signals.rs`); on
//! Windows the console's Ctrl events and its close.

use std::io;

use crate::state::{HeadlessState, Poster, post};

/// The signals that quit.
#[cfg(unix)]
const QUITTING: [i32; 3] = [
    signal_hook::consts::SIGTERM,
    signal_hook::consts::SIGINT,
    signal_hook::consts::SIGHUP,
];

/// Quits through the main loop behind `poster` when the system asks the process to end.
///
/// # Errors
///
/// The handler could not be installed; the system's request then ends the process without the quit save.
#[cfg(unix)]
pub fn quit_on_termination(poster: Poster) -> io::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use signal_hook::iterator::Signals;

    // the conditional default first, so it sees the flag the next registration sets on the first signal
    for signal in QUITTING {
        let seen = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register_conditional_default(signal, Arc::clone(&seen))?;
        signal_hook::flag::register(signal, seen)?;
    }
    let mut signals = Signals::new(QUITTING)?;
    std::thread::Builder::new()
        .name("ngterm-headless-signals".to_owned())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                tracing::info!(signal, "quitting");
                post(&poster, HeadlessState::request_quit);
            }
        })?;
    Ok(())
}

/// How long the console's closing or a shutdown waits for the quit save (the GPUI frontend's
/// `SHUTDOWN_GRACE`): Windows ends the process once the handler returns, and the save ends it from the main thread
/// meanwhile.
#[cfg(windows)]
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// Quits through the main loop behind `poster` on the console's Ctrl+C and Ctrl+Break (a second one gets the default
/// action, as a second Unix signal does) and on its closing or a shutdown, which wait for the quit save (the GPUI
/// frontend's `signals/windows.rs`). A logoff is ignored: an interactive console's process is ended at its own
/// session's logoff without the event, and a service's gets it at any user's.
///
/// # Errors
///
/// The handler could not be installed, or was installed already.
#[cfg(windows)]
pub fn quit_on_termination(poster: Poster) -> io::Result<()> {
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler,
    };
    use windows_sys::core::BOOL;

    static POSTER: OnceLock<Poster> = OnceLock::new();
    /// A Ctrl+C or Ctrl+Break came before: the next one gets the default action.
    static INTERRUPTED: AtomicBool = AtomicBool::new(false);

    fn request_quit(why: &str) {
        tracing::info!("{why}: quitting");
        if let Some(poster) = POSTER.get() {
            post(poster, HeadlessState::request_quit);
        }
    }

    unsafe extern "system" fn handler(kind: u32) -> BOOL {
        match kind {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => {
                if INTERRUPTED.swap(true, Ordering::AcqRel) {
                    // the default action: the process ends
                    return FALSE;
                }
                request_quit(if kind == CTRL_C_EVENT { "Ctrl+C" } else { "Ctrl+Break" });
                TRUE
            }
            // only a service's process gets a logoff, and on any user's logoff: not this instance's end
            CTRL_LOGOFF_EVENT => TRUE,
            CTRL_CLOSE_EVENT | CTRL_SHUTDOWN_EVENT => {
                request_quit("the console is closing");
                std::thread::sleep(SHUTDOWN_GRACE);
                TRUE
            }
            _ => FALSE,
        }
    }

    if POSTER.set(poster).is_err() {
        return Err(io::Error::other("the quit handler is installed already"));
    }
    // SAFETY: a handler that only posts to the main loop and sleeps, for the whole process.
    if unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
