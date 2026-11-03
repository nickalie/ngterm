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
}
