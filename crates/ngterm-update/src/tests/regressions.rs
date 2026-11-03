use super::*;

#[test]
#[cfg(unix)]
fn bundle_updates_require_user_ownership_and_writable_directories() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm.app");
    fs::create_dir_all(root.join("Contents/Resources")).unwrap();
    let user = fs::metadata(&root).unwrap().uid();
    assert!(bundle_access(&root, user).is_ok());
    assert!(bundle_access(&root, user.wrapping_add(1)).is_err());
    let directory = root.join("Contents/Resources");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    assert!(bundle_access(&root, user).is_err());
    fs::set_permissions(directory, fs::Permissions::from_mode(0o755)).unwrap();
}

fn portable_tree(root: &Path, version: &str) -> PathBuf {
    let (prefix, marker, name) = if cfg!(windows) {
        (root.to_path_buf(), "windows-archive", "ngterm.exe")
    } else if cfg!(target_os = "macos") {
        (root.to_path_buf(), "macos-archive", "ngterm")
    } else {
        (root.join("usr"), "linux-archive", "ngterm")
    };
    fs::create_dir_all(prefix.join("bin")).unwrap();
    fs::create_dir_all(prefix.join("share/ngterm")).unwrap();
    fs::write(prefix.join(MARKER), marker).unwrap();
    fs::write(prefix.join("share/ngterm/app-BUILD"), format!("{version}\ncommit\n")).unwrap();
    let executable = prefix.join("bin").join(name);
    fs::write(&executable, "binary").unwrap();
    executable
}

#[test]
fn containment_resolves_existing_and_future_directories_before_every_install() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let executable = portable_tree(&root, "0.1.0");
    let install = Installation::at(&executable).unwrap();
    let outside = fixture.path().join("external");
    fs::create_dir(&outside).unwrap();
    install.validate_paths(std::slice::from_ref(&outside)).unwrap();
    let inside = root.join("config");
    fs::create_dir(&inside).unwrap();
    for path in [
        inside.clone(),
        fs::canonicalize(&inside).unwrap(),
        root.join("future/nested"),
    ] {
        assert!(install.validate_paths(&[outside.clone(), path]).is_err());
    }
    assert!(!install.contains_path(&fixture.path().join("future/external")));
    for reopen in [false, true] {
        let work = staging::create(&install, fixture.path()).unwrap();
        let stage = work.path().to_path_buf();
        let prepared = Prepared {
            installation: install.clone(),
            candidate: work.path().join("unused"),
            work,
        };
        assert!(
            prepared
                .launch(&outside.join("error"), reopen, std::slice::from_ref(&inside))
                .is_err()
        );
        assert!(executable.exists());
        assert!(!stage.exists());
    }
}

#[test]
#[cfg(windows)]
fn windows_containment_normalizes_both_disk_and_verbatim_roots() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let executable = portable_tree(&root, "0.1.0");
    let mut install = Installation::at(&executable).unwrap();
    let inside = root.join("state");
    fs::create_dir(&inside).unwrap();
    let verbatim_state = fs::canonicalize(&inside).unwrap();
    for spelling in [dunce::canonicalize(&root).unwrap(), fs::canonicalize(&root).unwrap()] {
        install.root = spelling;
        assert!(install.contains_path(&inside));
        assert!(install.contains_path(&verbatim_state));
        assert!(install.contains_path(&PathBuf::from(inside.to_string_lossy().to_lowercase())));
    }
}

#[test]
#[cfg(unix)]
fn launcher_symlinks_and_missing_configuration_under_symlinked_ancestors_resolve_to_the_installation() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let executable = portable_tree(&root, "0.1.0");
    let link = fixture.path().join("launcher");
    std::os::unix::fs::symlink(&executable, &link).unwrap();
    let install = Installation::at(&link).unwrap();
    assert_eq!(install.executable, dunce::canonicalize(executable).unwrap());
    let alias = fixture.path().join("config-alias");
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert!(install.contains_path(&alias.join("not-created/config")));
}

#[test]
fn cleanup_preserves_live_helpers_other_installations_and_recovery_backups() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let install = Installation::at(&portable_tree(&root, "0.1.0")).unwrap();
    let live = staging::create(&install, fixture.path()).unwrap().keep();
    let abandoned = staging::create(&install, fixture.path()).unwrap();
    staging::transfer(abandoned.path(), &install, 99_999_999).unwrap();
    fs::write(abandoned.path().join("payload"), "orphan").unwrap();
    let abandoned = abandoned.keep();
    let recovery = staging::create(&install, fixture.path()).unwrap();
    staging::transfer(recovery.path(), &install, 99_999_999).unwrap();
    fs::write(recovery.path().join("recovery-required"), "old version").unwrap();
    let recovery = recovery.keep();
    let other_root = fixture.path().join("other/ngterm");
    let other = Installation::at(&portable_tree(&other_root, "0.1.0")).unwrap();
    let other_stage = staging::create(&other, fixture.path()).unwrap();
    staging::transfer(other_stage.path(), &other, 99_999_999).unwrap();
    let other_stage = other_stage.keep();
    let unmarked = fixture.path().join(".ngterm-update-not-owned");
    fs::create_dir(&unmarked).unwrap();
    assert_eq!(install.cleanup_stale().unwrap(), 1);
    assert!(!abandoned.exists());
    for retained in [live, recovery, other_stage, unmarked] {
        assert!(retained.exists());
    }
}

#[test]
#[cfg(unix)]
fn relaunch_restores_a_replaced_working_directory_and_relative_state_path() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("install");
    let work = fixture.path().join("stage");
    let candidate = work.join("candidate");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&candidate).unwrap();
    let identity = record_test_installation(&root, &work);
    let state = fixture.path().join("state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("sentinel"), "saved state").unwrap();
    fs::write(
        candidate.join("ngterm"),
        "#!/bin/sh\ncat \"$NGTERM_STATE_DIR/sentinel\" > \"$NGTERM_UPDATE_TEST_RESULT\"\n",
    )
    .unwrap();
    fs::set_permissions(candidate.join("ngterm"), fs::Permissions::from_mode(0o755)).unwrap();
    let result = fixture.path().join("result");
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(install::UNIX_HELPER)
        .arg("test-updater")
        .arg("99999999")
        .arg(&root)
        .arg(&candidate)
        .arg(&work)
        .arg(root.join("ngterm"))
        .arg(fixture.path().join("error"))
        .arg("tree")
        .arg("reopen")
        .arg(identity)
        .arg("identity")
        .arg(&root)
        .current_dir(&root)
        .env("NGTERM_STATE_DIR", "../state")
        .env("NGTERM_UPDATE_TEST_RESULT", &result)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(result).unwrap(), "saved state");
}

#[cfg(windows)]
struct HeldProcess(std::process::Child);
#[cfg(windows)]
impl Drop for HeldProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[cfg(windows)]
fn windows_update_escapes_the_install_cwd_and_preserves_a_running_session_image() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let executable = portable_tree(&root, "0.1.0");
    let install = Installation::at(&executable).unwrap();
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/ping.exe");
    fs::copy(system, &executable).unwrap();
    let mut child = HeldProcess(
        std::process::Command::new(&executable)
            .args(["127.0.0.1", "-t"])
            .current_dir(fixture.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let work = staging::create(&install, fixture.path()).unwrap();
    install::record_installation(&install, work.path()).unwrap();
    let candidate = work.path().join("candidate");
    portable_tree(&candidate, "0.2.0");
    fs::write(candidate.join("bin/ngterm.exe"), "replacement").unwrap();
    let script = work.path().join("install.ps1");
    fs::write(&script, install::WINDOWS_HELPER).unwrap();
    let error_file = fixture.path().join("error");
    let status = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(script)
        .arg("99999999")
        .arg(&root)
        .arg(candidate)
        .arg(work.path())
        .arg(&executable)
        .arg(&error_file)
        .arg("tree")
        .arg("quit")
        .arg(install.identity_file())
        .arg("identity")
        .arg(executable.parent().unwrap())
        .current_dir(executable.parent().unwrap())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!error_file.exists(), "{:?}", fs::read_to_string(&error_file));
    assert_eq!(fs::read_to_string(&executable).unwrap(), "replacement");
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(
        work.path().join(staging::OWNER).exists(),
        "ownership survives a busy old executable"
    );
    staging::transfer(work.path(), &install, 99_999_999).unwrap();
    let work = work.keep();
    drop(child);
    assert_eq!(install.cleanup_stale().unwrap(), 1);
    assert!(!work.exists());
}
