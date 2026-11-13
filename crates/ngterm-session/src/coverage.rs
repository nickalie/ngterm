//! The LLVM coverage profile of an instrumented build (`cargo llvm-cov show-env`, which sets `cfg(coverage)` beside
//! `-C instrument-coverage`; the e2e coverage runs: the conformance runners' `--coverage`, scripts/test-mobile-e2e.sh
//! `--coverage`). The profile runtime writes it from an exit handler, which a process ending past its exit handlers
//! never runs. The exit paths of ngterm's programs that skip them, and what writes the profile first:
//!
//! - the desktop app's `exit_now` (`_exit`, or `TerminateProcess` of itself): [`write_profile`];
//! - `ngterm-headless`'s relaunch by `exec`: [`write_profile`];
//! - a termination signal's default action, which ends the process at once: [`write_profile_on_termination`], which
//!   `ngterm-session` installs (an attach client on an ssh login's pty ends by the hangup when the viewer goes);
//! - a kill (`SIGKILL`, `TerminateProcess` by another process): nothing can; the e2e runners count those they cause.
//!
//! `ngterm_app::coverage` re-exports these for the programs above the session crate.

/// Writes this process's coverage profile now (to `LLVM_PROFILE_FILE`, merged into what is there under `%m`) and
/// zeroes the counters, so a process that goes on and later exits normally does not count the same run twice. Nothing
/// in a build without coverage.
pub fn write_profile() {
    #[cfg(coverage)]
    {
        // compiler-rt's profile runtime, linked into every `-C instrument-coverage` binary
        unsafe extern "C" {
            fn __llvm_profile_write_file() -> std::ffi::c_int;
            fn __llvm_profile_reset_counters();
        }
        // SAFETY: the runtime's own writer, the one its exit handler calls; it touches only the runtime's state and may
        // run at any point. A write that fails reports on stderr; nothing more to do.
        unsafe { __llvm_profile_write_file() };
        // SAFETY: zeroes the runtime's own counters only.
        unsafe { __llvm_profile_reset_counters() };
    }
}

/// In an instrumented build on Unix: a termination signal (`SIGHUP`, `SIGINT`, `SIGTERM`) that would end the process
/// by its default action writes the profile first ([`write_profile`]), then ends it by that default action, so the
/// process's status and its parent's view stay what they were. A signal the process started with ignored stays
/// ignored, and nothing else changes: a program that handles a signal itself installs its handler after this one,
/// replacing it. Nothing in a build without coverage, or on Windows.
pub fn write_profile_on_termination() {
    #[cfg(all(coverage, unix))]
    if let Err(error) = before_termination(write_profile) {
        tracing::warn!(%error, "coverage: the profile is not written before a termination signal");
    }
}

/// The termination signals a default action ends the process by.
#[cfg(all(unix, any(coverage, test)))]
const TERMINATION: [i32; 3] = [libc::SIGHUP, libc::SIGINT, libc::SIGTERM];

/// Runs `action` on a termination signal, then ends the process by the signal's default action; a signal ignored now
/// is left ignored.
///
/// # Errors
///
/// The handlers or their thread could not be set up.
#[cfg(all(unix, any(coverage, test)))]
fn before_termination(action: fn()) -> std::io::Result<()> {
    let handled: Vec<i32> = TERMINATION.into_iter().filter(|signal| !ignored(*signal)).collect();
    if handled.is_empty() {
        return Ok(());
    }
    let mut signals = signal_hook::iterator::Signals::new(&handled)?;
    std::thread::Builder::new()
        .name("coverage-termination".to_owned())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                action();
                // resets the signal to its default action and raises it again: the process ends as it would have
                let _ = signal_hook::low_level::emulate_default_handler(signal);
            }
        })?;
    Ok(())
}

/// Whether `signal` is ignored, as it is when the process started with it ignored, before any handler is installed.
#[cfg(all(unix, any(coverage, test)))]
fn ignored(signal: i32) -> bool {
    let mut current = std::mem::MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: with no new action, sigaction only writes the current one into `current`, a whole `sigaction`.
    let read = unsafe { libc::sigaction(signal, std::ptr::null(), current.as_mut_ptr()) };
    // SAFETY: an all-zero `sigaction` is valid, and a successful call filled it.
    let current = unsafe { current.assume_init() };
    read == 0 && current.sa_sigaction == libc::SIG_IGN
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
