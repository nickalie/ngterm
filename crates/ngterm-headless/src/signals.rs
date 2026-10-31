//! The system's request to end the process quits it with the quit save, through the main loop: on Unix SIGTERM,
//! SIGINT and SIGHUP (a service manager's stop, Ctrl+C, the terminal that started it closing), waited for on a thread
//! of its own, a second one of the same signal getting the default action (the GPUI frontend's `signals.rs`). SIGINT
//! or SIGHUP ignored when the process started stays ignored, as a daemon's does: `nohup` ignores SIGHUP so the process
//! outlives the ssh session that started it, and a shell without job control starts a background job with SIGINT
//! ignored. On Windows the console's Ctrl events and its close.

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

    let quitting: Vec<i32> = QUITTING
        .into_iter()
        .filter(|signal| *signal == signal_hook::consts::SIGTERM || !ignored_at_start(*signal))
        .collect();
    // the conditional default first, so it sees the flag the next registration sets on the first signal
    for signal in quitting.iter().copied() {
        let seen = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register_conditional_default(signal, Arc::clone(&seen))?;
        signal_hook::flag::register(signal, seen)?;
    }
    let mut signals = Signals::new(&quitting)?;
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

/// Whether `signal` is ignored, as it is when the process started with it ignored, before any handler is installed.
#[cfg(unix)]
fn ignored_at_start(signal: i32) -> bool {
    let mut current = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: with no new action, sigaction only writes the current one into `current`, a whole `sigaction`.
    let read = unsafe { libc::sigaction(signal, std::ptr::null(), current.as_mut_ptr()) };
    // SAFETY: an all-zero `sigaction` is valid, and a successful call filled it.
    let current = unsafe { current.assume_init() };
    read == 0 && current.sa_sigaction == libc::SIG_IGN
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

#[cfg(all(test, unix))]
mod tests {
    use super::ignored_at_start;

    #[test]
    fn a_signal_ignored_before_the_handlers_is_read_back() {
        // SIGUSR2: nothing in the tests sends it, so its action is this test's to set and put back
        let signal = libc::SIGUSR2;
        // SAFETY: `SIG_IGN` installs no handler code.
        let previous = unsafe { libc::signal(signal, libc::SIG_IGN) };
        assert!(ignored_at_start(signal));
        // SAFETY: `SIG_DFL` installs no handler code.
        unsafe { libc::signal(signal, libc::SIG_DFL) };
        assert!(!ignored_at_start(signal));
        // SAFETY: the action read back above, put back as it was.
        unsafe { libc::signal(signal, previous) };
    }
}
