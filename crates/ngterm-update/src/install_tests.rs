use super::*;
use crate::tests::installation;
#[cfg(target_os = "linux")]
use crate::tests::tree;

#[test]
fn recorded_identity_preserves_exact_bytes_and_requires_an_image_stamp() {
    let fixture = tempfile::tempdir().unwrap();
    let mut install = installation(fixture.path());
    fs::create_dir_all(install.identity_file().parent().unwrap()).unwrap();
    let identity = b"0.1.0\ncommit\nextra\n";
    fs::write(install.identity_file(), identity).unwrap();
    let work = tempfile::tempdir().unwrap();
    record_installation(&install, work.path()).unwrap();
    assert_eq!(fs::read(work.path().join("expected-identity")).unwrap(), identity);
    install.format = Format::AppImage;
    assert_eq!(
        record_installation(&install, work.path()).unwrap_err().to_string(),
        "Application image has no recorded identity."
    );
    install.image_identity = Some(b"device:inode:size:time\n".to_vec());
    record_installation(&install, work.path()).unwrap();
    assert_eq!(
        fs::read(work.path().join("expected-identity")).unwrap(),
        b"device:inode:size:time\n"
    );
    assert!(matches!(
        record_installation(&install, &work.path().join("missing")),
        Err(Error::Io(_))
    ));
}

#[test]
fn single_file_formats_keep_the_download_as_the_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    let asset = fixture.path().join("asset");
    fs::write(&asset, "verified release bytes").unwrap();
    for format in [Format::AppImage, Format::WindowsMsi] {
        let work = tempfile::tempdir().unwrap();
        let mut install = installation(fixture.path());
        install.format = format;
        assert_eq!(extract(&install, &asset, work.path(), &version).unwrap(), asset);
        assert_eq!(fs::read_to_string(&asset).unwrap(), "verified release bytes");
        #[cfg(unix)]
        if format == Format::AppImage {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(fs::metadata(&asset).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }
}

#[test]
#[cfg(unix)]
fn extraction_commands_report_stderr_and_spawn_failures() {
    run(Command::new("/bin/sh").args(["-c", "exit 0"])).unwrap();
    assert_eq!(
        run(Command::new("/bin/sh").args(["-c", "printf 'bad archive' >&2; exit 7"]))
            .unwrap_err()
            .to_string(),
        "Release extraction or signature validation failed: bad archive"
    );
    let fixture = tempfile::tempdir().unwrap();
    assert!(matches!(
        run(&mut Command::new(fixture.path().join("missing"))),
        Err(Error::Io(_))
    ));
}

#[test]
#[cfg(target_os = "linux")]
fn image_stamp_identifies_the_file_and_reports_missing_images() {
    let fixture = tempfile::tempdir().unwrap();
    let image = fixture.path().join("image with spaces");
    fs::write(&image, "image").unwrap();
    let before = image_stamp(&image).unwrap();
    assert_eq!(image_stamp(&image).unwrap(), before);
    fs::write(&image, "a different size").unwrap();
    assert_ne!(image_stamp(&image).unwrap(), before);
    fs::remove_file(&image).unwrap();
    assert_eq!(
        image_stamp(&image).unwrap_err().to_string(),
        "Could not identify the installed application image."
    );
}

#[test]
#[cfg(target_os = "linux")]
fn archive_validation_reports_identity_format_and_each_missing_binary() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let version = Version::parse("0.2.0").unwrap();
    tree(&root, "0.1.0");
    assert_eq!(
        validate(&root, Format::LinuxArchive, &version).unwrap_err().to_string(),
        "Downloaded archive identity does not match the release."
    );
    tree(&root, "0.2.0");
    fs::write(root.join("usr").join(MARKER), "windows-archive").unwrap();
    assert_eq!(
        validate(&root, Format::LinuxArchive, &version).unwrap_err().to_string(),
        "Downloaded archive has an incompatible installation format."
    );
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless", "ngterm-web"] {
        tree(&root, "0.2.0");
        fs::remove_file(root.join("usr/bin").join(name)).unwrap();
        assert_eq!(
            validate(&root, Format::LinuxArchive, &version).unwrap_err().to_string(),
            format!("Downloaded archive has no {name}.")
        );
    }
    fs::remove_file(root.join("usr").join(MARKER)).unwrap();
    assert!(matches!(
        validate(&root, Format::LinuxArchive, &version),
        Err(Error::Io(_))
    ));
}

#[test]
fn invalid_archives_and_existing_unpack_directories_are_refused() {
    let fixture = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let asset = fixture.path().join("bad.tar.gz");
    fs::write(&asset, "not gzip").unwrap();
    let install = installation(fixture.path());
    let version = Version::parse("0.2.0").unwrap();
    let refusal = || match extract(&install, &asset, work.path(), &version).unwrap_err() {
        Error::Io(error) => error.kind(),
        other => panic!("expected an I/O error, got {other:?}"),
    };
    assert_ne!(refusal(), std::io::ErrorKind::AlreadyExists);
    // The first attempt left its unpack directory behind; a stale one is never reused.
    assert_eq!(refusal(), std::io::ErrorKind::AlreadyExists);
}

#[test]
fn failed_helper_script_write_drops_the_prepared_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let work = crate::staging::create(&installation(&root), fixture.path()).unwrap();
    fs::create_dir(
        work.path()
            .join(if cfg!(windows) { "install.ps1" } else { "install.sh" }),
    )
    .unwrap();
    let stage = work.path().to_owned();
    let prepared = Prepared {
        installation: installation(&root),
        candidate: work.path().join("unused"),
        work,
    };
    assert!(matches!(
        prepared.launch(&fixture.path().join("error"), false, &[]),
        Err(Error::Io(_))
    ));
    assert!(!stage.exists());
    assert!(root.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn launch_transfers_ownership_and_installs_after_the_parent_exits() {
    const CHILD_FIXTURE: &str = "UPDATE_LAUNCH_FIXTURE";
    if let Some(path) = std::env::var_os(CHILD_FIXTURE) {
        launch_child(Path::new(&path));
        return;
    }
    for (image, reopen) in [(false, false), (false, true), (true, false), (true, true)] {
        let fixture = tempfile::tempdir().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "install::tests::launch_transfers_ownership_and_installs_after_the_parent_exits",
                "--nocapture",
            ])
            .env(CHILD_FIXTURE, fixture.path())
            .env("UPDATE_IMAGE", if image { "yes" } else { "no" })
            .env("UPDATE_REOPEN", if reopen { "yes" } else { "no" })
            .env("UPDATE_REOPEN_RESULT", fixture.path().join("reopened"))
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        let pid = fs::read_to_string(fixture.path().join("helper.pid")).unwrap();
        wait_for_helper(pid.trim());
        if image {
            assert_eq!(
                fs::read_to_string(fixture.path().join("ngterm")).unwrap(),
                reopen_script()
            );
        } else {
            assert_eq!(
                fs::read_to_string(fixture.path().join("ngterm/usr/share/ngterm/app-BUILD")).unwrap(),
                "0.2.0\ncommit\n"
            );
        }
        let stage = fs::read_to_string(fixture.path().join("stage-path")).unwrap();
        assert!(!Path::new(&stage).exists());
        assert!(!fixture.path().join("error").exists());
        assert_eq!(fixture.path().join("reopened").exists(), reopen);
    }
}

#[cfg(target_os = "linux")]
fn launch_child(fixture: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let root = fixture.join("ngterm");
    let image = std::env::var("UPDATE_IMAGE").unwrap() == "yes";
    let mut install = installation(&root);
    if image {
        fs::write(&root, "old image").unwrap();
        install.format = Format::AppImage;
        install.executable = root.clone();
        install.image_identity = Some(image_stamp(&root).unwrap());
    } else {
        tree(&root, "0.1.0");
    }
    let work = tempfile::Builder::new().prefix("stage").tempdir_in(fixture).unwrap();
    record_installation(&install, work.path()).unwrap();
    let stage = work.path().to_owned();
    let candidate = work.path().join("candidate");
    let executable = if image {
        candidate.clone()
    } else {
        tree(&candidate, "0.2.0");
        candidate.join("usr/bin/ngterm")
    };
    fs::write(&executable, reopen_script()).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let prepared = Prepared {
        installation: install,
        work,
        candidate,
    };
    prepared
        .launch(
            &fixture.join("error"),
            std::env::var("UPDATE_REOPEN").unwrap() == "yes",
            &[],
        )
        .unwrap();
    let owner: serde_json::Value =
        serde_json::from_slice(&fs::read(stage.join(crate::staging::OWNER)).unwrap()).unwrap();
    fs::write(fixture.join("helper.pid"), owner["pid"].to_string()).unwrap();
    fs::write(fixture.join("stage-path"), stage.to_str().unwrap()).unwrap();
}

#[cfg(target_os = "linux")]
fn wait_for_helper(pid: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Ok(stat) if stat.rsplit_once(") ").is_some_and(|(_, rest)| rest.starts_with('Z')) => return,
            result => assert!(
                std::time::Instant::now() < deadline,
                "helper {pid} did not exit: {result:?}"
            ),
        }
        std::thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn reopen_script() -> &'static str {
    "#!/bin/sh\nprintf reopened > \"$UPDATE_REOPEN_RESULT\"\n"
}

#[test]
fn failed_ownership_transfer_stops_the_helper_and_removes_its_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let work = tempfile::tempdir_in(fixture.path()).unwrap();
    fs::create_dir(work.path().join(crate::staging::OWNER)).unwrap();
    let stage = work.path().to_owned();
    let prepared = Prepared {
        installation: installation(&root),
        candidate: work.path().join("unused"),
        work,
    };
    assert!(matches!(
        prepared.launch(&fixture.path().join("error"), false, &[]),
        Err(Error::Io(_))
    ));
    #[cfg(target_os = "linux")]
    assert_no_detached_child();
    assert!(!stage.exists());
    assert!(root.exists());
    assert!(!fixture.path().join("error").exists());
}

/// Fails while this process has a child leading its own process group, running or not yet reaped: only `launch`
/// detaches its children, and its other callers in this binary fail before spawning or run in a child process.
#[cfg(target_os = "linux")]
fn assert_no_detached_child() {
    let this = std::process::id().to_string();
    for entry in fs::read_dir("/proc").unwrap().flatten() {
        // A process that exits during the scan has no stat left.
        let Ok(stat) = fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((head, rest)) = stat.rsplit_once(") ") else {
            continue;
        };
        // After the command name: state, parent and process group.
        let mut fields = rest.split(' ').skip(1);
        let (parent_pid, group) = (fields.next(), fields.next());
        let pid = head.split_once(" (").map(|(pid, _)| pid);
        assert!(
            parent_pid != Some(this.as_str()) || group != pid,
            "the helper was left behind: {stat}"
        );
    }
}

#[test]
fn mac_archive_extraction_checks_identity_at_the_archive_root() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::create_dir_all(source.join("share/ngterm")).unwrap();
    fs::write(source.join("share/ngterm/app-BUILD"), "0.1.0\ncommit\n").unwrap();
    let asset = fixture.path().join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(fs::File::create(&asset).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all(".", &source).unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut install = installation(fixture.path());
    install.format = Format::MacArchive;
    assert_eq!(
        extract(&install, &asset, work.path(), &Version::parse("0.2.0").unwrap())
            .unwrap_err()
            .to_string(),
        "Downloaded archive identity does not match the release."
    );
    assert_eq!(
        fs::read_to_string(work.path().join("unpacked/share/ngterm/app-BUILD")).unwrap(),
        "0.1.0\ncommit\n"
    );
}

#[test]
fn bundle_validation_requires_the_contents_identity_and_resource_marker() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm.app");
    let prefix = root.join("Contents");
    fs::create_dir_all(prefix.join("share/ngterm")).unwrap();
    fs::write(prefix.join("share/ngterm/app-BUILD"), "0.2.0\ncommit\n").unwrap();
    let version = Version::parse("0.2.0").unwrap();
    assert!(matches!(
        validate(&root, Format::MacBundle, &version),
        Err(Error::Io(_))
    ));
    fs::create_dir(prefix.join("Resources")).unwrap();
    fs::write(prefix.join("Resources/update-format"), "incompatible").unwrap();
    assert_eq!(
        validate(&root, Format::MacBundle, &version).unwrap_err().to_string(),
        "Downloaded archive has an incompatible installation format."
    );
}
