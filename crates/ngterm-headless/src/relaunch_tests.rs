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

/// Names the child that relaunches on Windows.
#[cfg(windows)]
const RELAUNCH_CHILD: &str = "HEADLESS_RELAUNCH_CHILD";

/// On Windows the relaunch starts the binary again with the next instance's arguments and returns. It runs in a child
/// whose output the new instance inherits, so the parent's read ends with the new instance: this test binary, which
/// refuses the grid option it was started with.
#[cfg(windows)]
#[test]
fn the_binary_is_started_again_with_the_next_instances_arguments() {
    if std::env::var_os(RELAUNCH_CHILD).is_some() {
        let options = Options {
            grid: (120, 40),
            ..Options::default()
        };
        relaunch(&options).unwrap();
        return;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "relaunch::tests::the_binary_is_started_again_with_the_next_instances_arguments",
            "--nocapture",
        ])
        .env(RELAUNCH_CHILD, "1");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(name);
        }
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Unrecognized option: 'grid'"), "{output:?}");
}

/// Names the child, running from a copy of this test binary, that moves its own file away before relaunching.
#[cfg(windows)]
const MOVED_CHILD: &str = "HEADLESS_RELAUNCH_MOVED_CHILD";

/// Windows lets a running binary's file be renamed, and the process still names it by its old path: with no file there
/// the relaunch starts nothing and says why.
#[cfg(windows)]
#[test]
fn a_binary_moved_away_is_not_relaunched() {
    if let Some(binary) = std::env::var_os(MOVED_CHILD) {
        let binary = std::path::PathBuf::from(binary);
        std::fs::rename(&binary, binary.with_extension("moved.exe")).unwrap();
        assert_eq!(std::env::current_exe().unwrap(), binary);
        let error = relaunch(&Options::default()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "the binary to restart is gone");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("relaunch-test.exe");
    ngterm_host::fakes::copy_executable(&std::env::current_exe().unwrap(), &binary).unwrap();
    let mut command = std::process::Command::new(&binary);
    command
        .args([
            "--exact",
            "relaunch::tests::a_binary_moved_away_is_not_relaunched",
            "--nocapture",
        ])
        .env(MOVED_CHILD, &binary);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(name);
        }
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!binary.exists(), "the child did not move its binary");
}
