//! The system's request to end the app quits it without a prompt (`conformance/docs/rust-build.md`: within 10 s): the
//! quit is posted to the main loop, whose consumer runs the orderly shutdown (the quit save, the control channel
//! stopped) and GPUI's quit. Installed only once this process is the primary: a launch that forwards to a running
//! ngterm keeps the defaults.
//!
//! - Unix: SIGTERM, waited for on a thread of its own. A second SIGTERM gets the default action, as in the GTK build.
//! - Windows ([`windows`]): the console's Ctrl+C and Ctrl+Break (a second one gets the default action, as a second
//!   SIGTERM does) and its closing, and the session's end (`WM_QUERYENDSESSION`/`WM_ENDSESSION`, which Windows sends
//!   a GUI process instead of the console's logoff and shutdown events) to a hidden window of its own.

#[cfg(unix)]
use std::io;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::thread;

#[cfg(unix)]
use signal_hook::SigId;
#[cfg(unix)]
use signal_hook::consts::SIGTERM;
#[cfg(unix)]
use signal_hook::iterator::Signals;

#[cfg(unix)]
use crate::state::{AppState, Poster, post};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::quit_on_termination;

/// Quits through the main loop behind `poster` when the system asks the app to end (see the module docs).
///
/// # Errors
///
/// The handler could not be installed; the system's request then ends the app without the quit save.
#[cfg(unix)]
pub fn quit_on_termination(poster: Poster) -> io::Result<()> {
    quit_on_sigterm(poster)
}

/// Handles the next SIGTERM through the main loop behind `poster`; a second one terminates the process.
///
/// # Errors
///
/// The handler could not be registered or its thread not started; nothing of it stays installed then, so SIGTERM
/// keeps its default action.
#[cfg(unix)]
pub fn quit_on_sigterm(poster: Poster) -> io::Result<()> {
    let mut signals = Signals::new([SIGTERM])?;
    let flags = second_sigterm_terminates()?;
    let spawned = thread::Builder::new().name("ngterm-sigterm".to_owned()).spawn(move || {
        if signals.forever().next().is_some() {
            tracing::info!("SIGTERM: quitting");
            post(&poster, AppState::request_quit);
        }
    });
    if let Err(error) = spawned {
        // the closure, and with it the iterator's registration, was dropped with the failed spawn
        unregister(&flags);
        return Err(error);
    }
    Ok(())
}

/// The first SIGTERM sets a flag, and a SIGTERM finding it set gets the default action. The conditional default is
/// registered first, so it sees the flag the next registration sets on the first signal.
#[cfg(unix)]
fn second_sigterm_terminates() -> io::Result<[SigId; 2]> {
    let seen = Arc::new(AtomicBool::new(false));
    let default = signal_hook::flag::register_conditional_default(SIGTERM, Arc::clone(&seen))?;
    match signal_hook::flag::register(SIGTERM, seen) {
        Ok(flag) => Ok([default, flag]),
        Err(error) => {
            unregister(&[default]);
            Err(error)
        }
    }
}

#[cfg(unix)]
fn unregister(ids: &[SigId]) {
    for id in ids {
        signal_hook::low_level::unregister(*id);
    }
}
