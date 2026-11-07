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
        std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
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
