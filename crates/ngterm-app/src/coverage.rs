//! The LLVM coverage profile of an instrumented build (`cargo llvm-cov show-env`, which sets `cfg(coverage)` beside
//! `-C instrument-coverage`; the conformance runners' `--coverage`). The profile runtime writes it from an exit
//! handler, which a process ending past its exit handlers never runs: the app's `exit_now` (`_exit`, or
//! `TerminateProcess` of itself), and `ngterm-headless`'s relaunch by `exec`. Those write it here first.

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

#[cfg(test)]
mod tests {
    #[test]
    fn writing_the_profile_leaves_the_process_running() {
        super::write_profile();
        super::write_profile();
    }

    /// Under coverage (`cargo llvm-cov test -p ngterm-app`): a process that writes its profile and then ends past its
    /// exit handlers, as `exit_now` does, leaves a raw profile; one that skips the write leaves none, which shows the
    /// exit handlers did not run. Each runs as this test binary's [`child_ends_past_its_exit_handlers`].
    #[cfg(coverage)]
    #[test]
    fn a_process_ending_past_its_exit_handlers_keeps_its_profile_only_when_written() {
        // compiler-rt's raw profile magic (`__llvm_profile_raw_magic`: 0xff, "lprofr", 0x81), little-endian on every
        // target CI builds
        const RAW_MAGIC: u64 = 0xff6c_7072_6f66_7281;
        let dir = tempfile::tempdir().expect("temporary directory");
        for (mode, written) in [("write", true), ("skip", false)] {
            let profile = dir.path().join(format!("{mode}.profraw"));
            let status = std::process::Command::new(std::env::current_exe().expect("this test binary"))
                .args([
                    "coverage::tests::child_ends_past_its_exit_handlers",
                    "--exact",
                    "--test-threads=1",
                ])
                .env(CHILD, mode)
                .env("LLVM_PROFILE_FILE", &profile)
                .status()
                .expect("the child test");
            assert!(status.success(), "{mode}: {status}");
            if written {
                let bytes = std::fs::read(&profile).expect("the written profile");
                let magic = bytes.first_chunk::<8>().map(|head| u64::from_le_bytes(*head));
                assert_eq!(magic, Some(RAW_MAGIC), "{mode}: not a raw profile");
            } else {
                assert!(!profile.exists(), "{mode}: the exit handlers ran");
            }
        }
    }

    /// What tells [`child_ends_past_its_exit_handlers`] it runs as the child, and whether to write the profile.
    #[cfg(coverage)]
    const CHILD: &str = "COVERAGE_TEST_CHILD";

    /// The child of the test above; nothing when run on its own.
    #[cfg(coverage)]
    #[test]
    fn child_ends_past_its_exit_handlers() {
        let Ok(mode) = std::env::var(CHILD) else { return };
        if mode == "write" {
            super::write_profile();
        }
        #[cfg(unix)]
        // SAFETY: `_exit` only ends the process.
        unsafe {
            libc::_exit(0)
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
            // SAFETY: the current process's pseudo-handle; nothing of this process runs after the call.
            unsafe { TerminateProcess(GetCurrentProcess(), 0) };
        }
    }
}
