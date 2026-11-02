//! macOS starts GUI apps and ssh sessions with only 256 descriptors. A session server retains several per pane.

/// Raise the soft descriptor limit for the frontend or session server, preserving the inherited hard limit.
pub fn raise_open_files() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: writes this process's limits into a live output structure.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0 {
        tracing::warn!(error = %std::io::Error::last_os_error(), "cannot read the descriptor limit");
        return;
    }
    // Below macOS's default kern.maxfilesperproc; never lower a larger inherited limit.
    let desired = 8192.min(limit.rlim_max);
    if limit.rlim_cur >= desired {
        return;
    }
    limit.rlim_cur = desired;
    // SAFETY: adjusts only this process's soft limit, within its existing hard limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) } != 0 {
        tracing::warn!(error = %std::io::Error::last_os_error(), "cannot raise the descriptor limit");
    }
}
