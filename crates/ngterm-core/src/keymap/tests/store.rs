//! Ported from upstream's `KeymapTests` store cases: loading `keymap.conf` from disk, plus seeding the starter.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::super::store::{StarterStep, write_starter};
use super::*;

fn upstream() -> ShippedDefaults {
    ShippedDefaults::upstream()
}

fn no_step(step: StarterStep) {
    panic!("the starter reached {step:?}");
}

/// The entries of `dir`, by name.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    entries.sort();
    entries
}

#[test]
fn a_missing_file_is_the_empty_keymap_and_an_existing_one_parses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let (keymap, diagnostics) = load_keymap(&path, &upstream());
    assert_eq!(keymap, Keymap::empty(&upstream()));
    assert!(diagnostics.is_empty());

    fs::write(&path, "command \"Greet\" ctrl+shift+y echo hi\n").unwrap();
    let (keymap, diagnostics) = load_keymap(&path, &upstream());
    assert!(keymap.commands().iter().any(|command| command.name == "Greet"));
    assert!(diagnostics.is_empty());
}

#[test]
fn a_malformed_keymap_reports_its_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    fs::write(
        &path,
        "map cmd+d not_an_action\ncommand \"Good\" ctrl+shift+y echo ok\n",
    )
    .unwrap();
    let (keymap, diagnostics) = load_keymap(&path, &upstream());
    assert!(keymap.commands().iter().any(|command| command.name == "Good"));
    assert_eq!(diagnostics, [diagnostic(1, "unknown action 'not_an_action'")]);
}

#[test]
fn an_unreadable_file_is_the_empty_keymap_with_one_whole_file_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();
    let (keymap, diagnostics) = load_keymap(&path, &upstream());
    assert_eq!(keymap, Keymap::empty(&upstream()));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].line, 0);
    assert_eq!(
        diagnostics[0].message,
        "could not read keymap.conf: the file isn't valid UTF-8 text"
    );

    // a directory where the file should be exists but cannot be read either.
    let (_, diagnostics) = load_keymap(dir.path(), &upstream());
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].message.starts_with("could not read keymap.conf: "));
    assert!(
        !diagnostics[0].message.contains("os error"),
        "{}",
        diagnostics[0].message
    );
}

#[test]
fn the_starter_is_written_only_when_no_file_is_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config").join("ngterm").join("keymap.conf");
    ensure_starter_keymap(&path, &upstream());
    assert_eq!(fs::read_to_string(&path).unwrap(), starter_keymap_conf(&upstream()));

    fs::write(&path, "map cmd+shift+e toggle_split\n").unwrap();
    ensure_starter_keymap(&path, &upstream());
    assert_eq!(fs::read_to_string(&path).unwrap(), "map cmd+shift+e toggle_split\n");
}

#[test]
fn a_starter_failure_preserves_the_file_blocking_its_directory() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("config");
    fs::write(&parent, "user data").unwrap();
    ensure_starter_keymap(&parent.join("keymap.conf"), &upstream());
    assert_eq!(fs::read_to_string(parent).unwrap(), "user data");
}

#[cfg(unix)]
#[test]
fn a_starter_cannot_write_in_a_read_only_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let permissions = fs::metadata(dir.path()).unwrap().permissions();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();
    ensure_starter_keymap(&path, &upstream());
    fs::set_permissions(dir.path(), permissions).unwrap();
    assert!(!path.exists());
}

#[cfg(unix)]
#[test]
fn a_dangling_link_is_replaced_by_the_starter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    ensure_starter_keymap(&path, &upstream());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
    assert_eq!(fs::read_to_string(&path).unwrap(), starter_keymap_conf(&upstream()));
    assert!(!target.exists());
    assert_eq!(entries(dir.path()), [path]);
}

#[test]
fn a_file_created_after_the_existence_check_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    write_starter(
        &path,
        || {
            fs::write(&path, "map ctrl+shift+y toggle_split\n").unwrap();
            starter_keymap_conf(&upstream())
        },
        no_step,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "map ctrl+shift+y toggle_split\n");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_file_replacing_a_dangling_link_during_starter_creation_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    write_starter(
        &path,
        || {
            fs::remove_file(&path).unwrap();
            fs::write(&path, "map ctrl+shift+y toggle_split\n").unwrap();
            starter_keymap_conf(&upstream())
        },
        |step| assert_eq!(step, StarterStep::MovedAside),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "map ctrl+shift+y toggle_split\n");
    assert!(!target.exists());
    assert_eq!(entries(dir.path()), [path]);
}

#[cfg(unix)]
#[test]
fn a_dangling_link_whose_target_appears_during_starter_creation_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    write_starter(
        &path,
        || {
            fs::write(&target, "map ctrl+shift+y toggle_split\n").unwrap();
            starter_keymap_conf(&upstream())
        },
        |step| assert_eq!(step, StarterStep::MovedAside),
    )
    .unwrap();
    assert_eq!(fs::read_link(&path).unwrap(), target);
    assert_eq!(fs::read_to_string(&path).unwrap(), "map ctrl+shift+y toggle_split\n");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[cfg(unix)]
#[test]
fn a_different_dangling_link_created_during_starter_creation_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    let replacement = dir.path().join("other.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    write_starter(
        &path,
        || {
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&replacement, &path).unwrap();
            starter_keymap_conf(&upstream())
        },
        |step| assert_eq!(step, StarterStep::MovedAside),
    )
    .unwrap();
    assert_eq!(fs::read_link(&path).unwrap(), replacement);
    assert!(!target.exists());
    assert!(!replacement.exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_link_removed_before_it_is_moved_aside_gets_the_starter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    write_starter(
        &path,
        || {
            fs::remove_file(&path).unwrap();
            starter_keymap_conf(&upstream())
        },
        no_step,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), starter_keymap_conf(&upstream()));
    assert!(!target.exists());
    assert_eq!(entries(dir.path()), [path]);
}

#[cfg(unix)]
#[test]
fn a_file_written_just_before_the_starter_is_placed_survives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let mut steps = Vec::new();
    write_starter(
        &path,
        || starter_keymap_conf(&upstream()),
        |step| {
            steps.push(step);
            if step == StarterStep::Placing {
                // the link is aside, so the path is free for whoever writes it now.
                assert_eq!(fs::symlink_metadata(&path).unwrap_err().kind(), io::ErrorKind::NotFound);
                fs::write(&path, "map ctrl+shift+y toggle_split\n").unwrap();
            }
        },
    )
    .unwrap();
    assert_eq!(steps, [StarterStep::MovedAside, StarterStep::Placing]);
    assert_eq!(fs::read_to_string(&path).unwrap(), "map ctrl+shift+y toggle_split\n");
    assert!(!target.exists());
    assert_eq!(entries(dir.path()), [path]);
}

#[cfg(unix)]
#[test]
fn a_relative_dangling_link_is_replaced_by_the_starter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    std::os::unix::fs::symlink("missing.conf", &path).unwrap();
    ensure_starter_keymap(&path, &upstream());
    assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
    assert_eq!(fs::read_to_string(&path).unwrap(), starter_keymap_conf(&upstream()));
    assert_eq!(entries(dir.path()), [path]);
}

#[cfg(unix)]
#[test]
fn a_relative_link_whose_target_appears_while_it_is_aside_is_moved_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink("missing.conf", &path).unwrap();
    write_starter(
        &path,
        || starter_keymap_conf(&upstream()),
        |step| {
            assert_eq!(step, StarterStep::MovedAside);
            fs::write(&target, "map ctrl+shift+y toggle_split\n").unwrap();
        },
    )
    .unwrap();
    assert_eq!(fs::read_link(&path).unwrap(), Path::new("missing.conf"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "map ctrl+shift+y toggle_split\n");
    assert_eq!(entries(dir.path()), [path, target]);
}

#[cfg(unix)]
#[test]
fn a_failed_placement_moves_the_link_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let error = write_starter(
        &path,
        || starter_keymap_conf(&upstream()),
        |step| {
            if step == StarterStep::Placing {
                // the staged starter is the directory's only regular file: the link sits in a directory beside it.
                let staged: Vec<_> = entries(dir.path())
                    .into_iter()
                    .filter(|entry| entry.is_file())
                    .collect();
                assert_eq!(staged.len(), 1);
                fs::remove_file(&staged[0]).unwrap();
            }
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert_eq!(fs::read_link(&path).unwrap(), target);
    assert!(!target.exists());
    assert_eq!(entries(dir.path()), [path]);
}

#[cfg(unix)]
#[test]
fn an_entry_that_cannot_be_moved_back_is_kept_aside_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.conf");
    let target = dir.path().join("missing.conf");
    std::os::unix::fs::symlink(&target, &path).unwrap();
    let error = write_starter(
        &path,
        || {
            fs::remove_file(&path).unwrap();
            fs::write(&path, "first\n").unwrap();
            starter_keymap_conf(&upstream())
        },
        |step| {
            assert_eq!(step, StarterStep::MovedAside);
            fs::write(&path, "second\n").unwrap();
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&path).unwrap(), "second\n");
    let aside: Vec<_> = entries(dir.path()).into_iter().filter(|entry| entry.is_dir()).collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read_to_string(aside[0].join("keymap.conf")).unwrap(), "first\n");
    let message = error.to_string();
    assert!(
        message.starts_with(&format!(
            "could not move {} back from {}: ",
            path.display(),
            aside[0].display()
        )),
        "{message}"
    );
    assert_eq!(entries(dir.path()).len(), 2);
}
