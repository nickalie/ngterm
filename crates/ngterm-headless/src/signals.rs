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

/// Quits through the main loop behind `poster` on the console's Ctrl+C, Ctrl+Break and close.
///
/// # Errors
///
/// The handler could not be installed.
#[cfg(windows)]
pub fn quit_on_termination(poster: Poster) -> io::Result<()> {
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{FALSE, TRUE};
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::core::BOOL;

    static POSTER: OnceLock<Poster> = OnceLock::new();

    unsafe extern "system" fn handler(_event: u32) -> BOOL {
        let Some(poster) = POSTER.get() else {
            return FALSE;
        };
        tracing::info!("console control event: quitting");
        post(poster, HeadlessState::request_quit);
        TRUE
    }

    if POSTER.set(poster).is_err() {
        return Ok(());
    }
    // SAFETY: `handler` is a valid handler routine for the life of the process.
    if unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) } == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
