use super::*;
use crate::tests::installation;

#[test]
fn cleanup_removes_nested_payload_and_preserves_unrecognized_entries() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let install = installation(&root);
    let abandoned = create(&install, fixture.path()).unwrap();
    transfer(abandoned.path(), &install, 99_999_999).unwrap();
    fs::create_dir_all(abandoned.path().join("previous/nested")).unwrap();
    fs::write(abandoned.path().join("previous/nested/data"), "old").unwrap();
    let abandoned = abandoned.keep();
    let malformed = fixture.path().join(".ngterm-update-malformed");
    fs::create_dir(&malformed).unwrap();
    fs::write(malformed.join(OWNER), "{}").unwrap();
    let ordinary = fixture.path().join("ordinary-directory");
    fs::create_dir(&ordinary).unwrap();
    let file = fixture.path().join(".ngterm-update-file");
    fs::write(&file, "not a stage").unwrap();
    assert_eq!(install.cleanup_stale().unwrap(), 1);
    assert!(!abandoned.exists());
    assert!(malformed.is_dir());
    assert!(ordinary.is_dir());
    assert_eq!(fs::read_to_string(file).unwrap(), "not a stage");
    assert_eq!(install.cleanup_stale().unwrap(), 0);
}

#[test]
fn cleanup_locks_requires_a_dead_valid_owner() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let install = installation(&root);
    let lock = fixture.path().join("ngterm.ngterm-update-lock");
    fs::create_dir(&lock).unwrap();
    for owner in ["invalid".to_owned(), std::process::id().to_string(), "0".into()] {
        fs::write(lock.join("owner.pid"), owner).unwrap();
        assert_eq!(install.cleanup_stale().unwrap(), 0);
        assert!(lock.join("owner.pid").exists());
    }
    fs::write(lock.join("owner.pid"), "99999999\n").unwrap();
    assert_eq!(install.cleanup_stale().unwrap(), 0);
    assert!(!lock.exists());
}

#[test]
#[cfg(unix)]
fn ambiguous_process_ids_preserve_stages() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let install = installation(&root);
    for pid in [0, u32::MAX] {
        let stage = create(&install, fixture.path()).unwrap();
        transfer(stage.path(), &install, pid).unwrap();
        assert_eq!(install.cleanup_stale().unwrap(), 0);
        assert!(stage.path().join(OWNER).exists());
    }
}

#[test]
fn staging_reports_missing_installations_and_unwritable_owner_paths() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installation(&fixture.path().join("missing"));
    assert!(matches!(create(&install, fixture.path()), Err(Error::Io(_))));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 0);
    assert!(matches!(install.cleanup_stale(), Err(Error::Io(_))));
    fs::create_dir(&install.root).unwrap();
    let stage = fixture.path().join("stage");
    fs::create_dir_all(stage.join(OWNER)).unwrap();
    assert!(matches!(transfer(&stage, &install, 0), Err(Error::Io(_))));
    let no_parent = installation(Path::new(""));
    assert_eq!(
        no_parent.cleanup_stale().unwrap_err().to_string(),
        "Installation has no parent directory."
    );
}

#[test]
#[cfg(unix)]
fn cleanup_unlinks_stage_symlinks_without_following_them() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let install = installation(&root);
    let stage = create(&install, fixture.path()).unwrap();
    transfer(stage.path(), &install, 99_999_999).unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("data"), "keep").unwrap();
    std::os::unix::fs::symlink(&outside, stage.path().join("link")).unwrap();
    let alias = fixture.path().join(".ngterm-update-alias");
    std::os::unix::fs::symlink(&outside, &alias).unwrap();
    assert_eq!(install.cleanup_stale().unwrap(), 1);
    assert_eq!(fs::read_to_string(outside.join("data")).unwrap(), "keep");
    assert!(alias.is_symlink());
}
