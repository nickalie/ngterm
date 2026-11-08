//! The shipped worker installs the app's logging before serving its socket, even when its peer has gone away.

use std::os::fd::AsRawFd as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use ngterm_wpe::worker::WORKER_FD_ENV;
use rustix::net::{AddressFamily, SocketFlags, SocketType};

fn worker(state: &Path, filter: &str) -> std::io::Result<Output> {
    let (app, socket) =
        rustix::net::socketpair(AddressFamily::UNIX, SocketType::SEQPACKET, SocketFlags::CLOEXEC, None)?;
    // No engine is needed: the worker initializes logging, then exits when it finds its peer gone.
    drop(app);
    let mut command = Command::new(env!("CARGO_BIN_EXE_ngterm-web"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("NGTERM_") || key.to_string_lossy().starts_with("AGTERM_") {
            command.env_remove(key);
        }
    }
    command
        .env(WORKER_FD_ENV, socket.as_raw_fd().to_string())
        .env(ngterm_core::paths::STATE_DIR_ENV, state)
        .env(ngterm_app::logging::LOG_ENV, filter)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: the child only clears close-on-exec on its own socket, using an async-signal-safe fcntl.
    unsafe {
        command.pre_exec(move || {
            rustix::io::fcntl_setfd(&socket, rustix::io::FdFlags::empty()).map_err(std::io::Error::from)
        });
    }
    let mut child = command.spawn()?;
    drop(command);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output()?;
            return Err(std::io::Error::other(format!(
                "worker did not exit: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn worker_opens_the_apps_log_in_the_inherited_state_directory() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let output = worker(&state, "warn").unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(state.join(ngterm_app::logging::LOG_FILE_NAME).is_file());
}

#[test]
fn worker_logs_to_stderr_when_the_log_is_unavailable_and_honors_the_filter() {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("file-not-directory");
    std::fs::write(&state, "").unwrap();
    for (filter, expected) in [("warn", true), ("off", false)] {
        let output = worker(&state, filter).unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            stderr.contains("log file unavailable; logging to stderr only"),
            expected
        );
    }
}
