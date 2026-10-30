//! The system's request to end the app on Windows (the parent module's docs say what each one does): a console
//! control handler (`SetConsoleCtrlHandler`), and the session's end as the app's hidden top-level window receives it
//! ([`crate::system_window`]).
//!
//! Windows ends the process once a handler for the console's closing returns, and may end the session once every
//! window returned from `WM_ENDSESSION`: both wait there, up to [`SHUTDOWN_GRACE`], for the orderly shutdown, which
//! ends the process itself (`main.rs`).

use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT, SetConsoleCtrlHandler,
};

use crate::state::{AppState, Poster, post};

/// How long a console's closing or the session's end waits for the orderly shutdown (the quit deadline of
/// `conformance/docs/rust-build.md`).
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);

/// Where the handlers post the quit.
static POSTER: OnceLock<Poster> = OnceLock::new();
/// A Ctrl+C or Ctrl+Break came before: the next one gets the default action.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Quits through the main loop behind `poster` on the console's Ctrl events and closing and on the session's end
/// (see the module docs).
///
/// # Errors
///
/// The console handler could not be installed or the app's hidden window not started; what was installed stays.
pub fn quit_on_termination(poster: Poster) -> io::Result<()> {
    if POSTER.set(poster).is_err() {
        return Err(io::Error::other("the quit handlers are installed already"));
    }
    // SAFETY: a handler that only posts to the main loop and sleeps, for the whole process.
    if unsafe { SetConsoleCtrlHandler(Some(on_console_event), TRUE) } == 0 {
        return Err(io::Error::last_os_error());
    }
    crate::system_window::on_session_end(|| {
        request_quit("the session is ending");
        wait_for_shutdown();
    })
}

fn request_quit(why: &str) {
    tracing::info!("{why}: quitting");
    if let Some(poster) = POSTER.get() {
        post(poster, AppState::request_quit);
    }
}

/// Gives the orderly shutdown its time: the process ends from the main thread meanwhile.
fn wait_for_shutdown() {
    thread::sleep(SHUTDOWN_GRACE);
}

unsafe extern "system" fn on_console_event(kind: u32) -> i32 {
    match kind {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            if INTERRUPTED.swap(true, Ordering::AcqRel) {
                // the default action: the process ends
                return FALSE;
            }
            request_quit(if kind == CTRL_C_EVENT { "Ctrl+C" } else { "Ctrl+Break" });
            TRUE
        }
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            request_quit("the console is closing");
            wait_for_shutdown();
            TRUE
        }
        _ => FALSE,
    }
}
