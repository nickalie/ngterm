use super::*;

#[test]
fn the_next_instance_keeps_the_grid_and_agent_setup_and_waits_for_the_lock() {
    let grid = Options {
        grid: (120, 40),
        ..Options::default()
    };
    assert_eq!(
        arguments(&grid),
        ["--grid", "120x40", RELAUNCHED_FLAG].map(OsString::from)
    );
    let skipped = Options {
        skip_agent_setup: true,
        ..grid.clone()
    };
    assert_eq!(
        arguments(&skipped),
        ["--grid", "120x40", "--skip-agent-setup", RELAUNCHED_FLAG].map(OsString::from)
    );
    let pinned = Options {
        opencode_version: Some(ngterm_app::integrations::opencode::Version::V2),
        ..grid
    };
    assert_eq!(
        arguments(&pinned),
        ["--grid", "120x40", "--opencode-version", "v2", RELAUNCHED_FLAG].map(OsString::from)
    );
}

#[test]
fn an_existing_binary_is_its_own_target() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("ngterm-headless");
    std::fs::write(&binary, b"").unwrap();
    assert_eq!(target(&binary), Some(binary.clone()));
    assert_eq!(target(&dir.path().join("gone")), None);
}

#[cfg(unix)]
#[test]
fn a_replaced_binary_is_restarted_from_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("ngterm-headless");
    std::fs::write(&binary, b"").unwrap();
    let reported = dir.path().join("ngterm-headless (deleted)");
    assert_eq!(target(&reported), Some(binary));
}

#[test]
fn a_deleted_path_without_a_replacement_has_no_target() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(target(&dir.path().join("missing (deleted)")), None);
    assert!(executable().unwrap().is_file());
}

#[cfg(unix)]
#[test]
fn a_failed_exec_reports_the_os_error_in_an_isolated_process() {
    use std::os::unix::fs::PermissionsExt as _;
    let Ok(path) = std::env::var("HEADLESS_RELAUNCH_CHILD") else {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("relaunch-test");
        ngterm_host::fakes::copy_executable(&std::env::current_exe().unwrap(), &binary).unwrap();
        let mut command = std::process::Command::new(&binary);
        command
            .args([
                "--exact",
                "relaunch::tests::a_failed_exec_reports_the_os_error_in_an_isolated_process",
                "--nocapture",
            ])
            .env("HEADLESS_RELAUNCH_CHILD", &binary);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("NGTERM_") {
                command.env_remove(name);
            }
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = relaunch(&Options::default());
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
}

#[cfg(unix)]
fn fork_children_holding_inherited_descriptors(stop: &std::sync::atomic::AtomicBool) {
    use std::os::unix::process::CommandExt as _;

    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "exit 0"]).env_clear();
        // Keep the child before exec long enough to retain write descriptors from other test threads.
        let delay_exec = || {
            let delay = libc::timespec {
                tv_sec: 0,
                tv_nsec: 20_000_000,
            };
            // SAFETY: a live timespec and no remainder requested.
            unsafe { libc::nanosleep(&raw const delay, std::ptr::null_mut()) };
            Ok(())
        };
        // SAFETY: the child only calls the async-signal-safe nanosleep before exec.
        unsafe { command.pre_exec(delay_exec) };
        assert!(command.status().unwrap().success());
    }
}

#[cfg(unix)]
#[test]
fn the_relaunch_fixture_starts_while_other_threads_fork() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let stop = AtomicBool::new(false);
    let ready = std::sync::Barrier::new(5);
    let result = std::thread::scope(|threads| {
        for _ in 0..4 {
            threads.spawn(|| {
                ready.wait();
                fork_children_holding_inherited_descriptors(&stop);
            });
        }
        ready.wait();
        // Stop and reap the forkers even when a fixture fails, before propagating its panic.
        let result = std::panic::catch_unwind(|| {
            for _ in 0..16 {
                a_failed_exec_reports_the_os_error_in_an_isolated_process();
            }
        });
        stop.store(true, Ordering::Relaxed);
        result
    });
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
