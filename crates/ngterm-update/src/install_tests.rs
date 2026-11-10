use super::*;
#[cfg(unix)]
use crate::fakes;
use crate::tests::installation;
#[cfg(target_os = "macos")]
use crate::tests::mac_archive_tree;
#[cfg(unix)]
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
        sha256: String::new(),
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
#[cfg(unix)]
fn launch_transfers_ownership_and_installs_after_the_parent_exits() {
    const CHILD_FIXTURE: &str = "UPDATE_LAUNCH_FIXTURE";
    if let Some(path) = std::env::var_os(CHILD_FIXTURE) {
        launch_child(Path::new(&path));
        return;
    }
    // Only Linux has application images, and only GNU's stat their stamp.
    let images: &[bool] = if cfg!(target_os = "linux") {
        &[false, true]
    } else {
        &[false]
    };
    for (image, reopen) in images.iter().flat_map(|image| [(*image, false), (*image, true)]) {
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
                fs::read_to_string(fakes::identity(&fixture.path().join(fakes::ROOT))).unwrap(),
                "0.2.0\ncommit\n"
            );
        }
        let stage = fs::read_to_string(fixture.path().join("stage-path")).unwrap();
        assert!(!Path::new(&stage).exists());
        assert!(!fixture.path().join("error").exists());
        assert_eq!(fixture.path().join("reopened").exists(), reopen);
    }
}

#[cfg(unix)]
fn launch_child(fixture: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let root = fixture.join(fakes::ROOT);
    let image = std::env::var("UPDATE_IMAGE").unwrap() == "yes";
    let install = if image {
        fs::write(&root, "old image").unwrap();
        let mut install = installation(&root);
        install.format = Format::AppImage;
        install.executable = root.clone();
        install.image_identity = Some(image_stamp(&root).unwrap());
        install
    } else {
        tree(&root, "0.1.0");
        fakes::installation(&fakes::executable(&root)).unwrap()
    };
    let work = tempfile::Builder::new().prefix("stage").tempdir_in(fixture).unwrap();
    record_installation(&install, work.path()).unwrap();
    let stage = work.path().to_owned();
    let candidate = work.path().join("candidate");
    let executable = if image {
        candidate.clone()
    } else {
        tree(&candidate, "0.2.0");
        fakes::executable(&candidate)
    };
    fs::write(&executable, reopen_script()).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let prepared = Prepared {
        sha256: String::new(),
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

#[cfg(target_os = "macos")]
fn wait_for_helper(pid: &str) {
    let helper = pid.parse().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    // SAFETY: signal 0 only asks whether the helper, a process this test's child started, still exists.
    while unsafe { libc::kill(helper, 0) } == 0 {
        assert!(std::time::Instant::now() < deadline, "helper {pid} did not exit");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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

#[cfg(unix)]
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
        sha256: String::new(),
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

/// The refusal text a failed `ditto` or `codesign` starts with.
#[cfg(target_os = "macos")]
const NOT_EXTRACTED: &str = "Release extraction or signature validation failed: ";

#[cfg(target_os = "macos")]
fn extract_bundle(fixture: &Path, archive: &[u8], version: &Version) -> Result<PathBuf, Error> {
    let asset = fixture.join("release.app.zip");
    fs::write(&asset, archive).unwrap();
    let mut install = installation(&fixture.join(fakes::ROOT));
    install.format = Format::MacBundle;
    let work = tempfile::Builder::new()
        .prefix("work")
        .tempdir_in(fixture)
        .unwrap()
        .keep();
    extract(&install, &asset, &work, version)
}

#[test]
#[cfg(target_os = "macos")]
fn a_signed_bundle_is_extracted_and_verified_as_the_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    let candidate = extract_bundle(fixture.path(), &fakes::archive("0.2.0").unwrap(), &version).unwrap();
    assert_eq!(
        candidate.parent().unwrap().file_name().unwrap(),
        "unpacked",
        "{candidate:?}"
    );
    assert_eq!(candidate.file_name().unwrap(), fakes::ROOT);
    assert_eq!(
        fs::read_to_string(fakes::identity(&candidate)).unwrap(),
        "0.2.0\ncommit\n"
    );
    assert!(candidate.join("Contents/MacOS/ngterm-session").is_file());
}

#[test]
#[cfg(target_os = "macos")]
fn unsigned_tampered_or_corrupt_bundle_archives_are_refused_before_validation() {
    let fixture = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    let refused = |archive: &[u8]| {
        extract_bundle(fixture.path(), archive, &version)
            .unwrap_err()
            .to_string()
    };
    let corrupt = refused(b"not a zip");
    assert!(corrupt.starts_with(NOT_EXTRACTED), "{corrupt}");

    let source = tempfile::tempdir().unwrap();
    let unsigned = source.path().join("unsigned").join(fakes::ROOT);
    tree(&unsigned, "0.2.0");
    let refusal = refused(&fakes::zip(&unsigned).unwrap());
    assert!(refusal.starts_with(NOT_EXTRACTED), "{refusal}");
    // /usr/bin/true's own signature doesn't cover the bundle; the tool's refusal names the bundle.
    assert!(refusal.contains("ngterm.app:"), "{refusal}");

    let tampered = source.path().join("tampered").join(fakes::ROOT);
    tree(&tampered, "0.2.0");
    fakes::seal(&tampered).unwrap();
    fs::write(fakes::identity(&tampered), "0.2.0\nsomething else\n").unwrap();
    let refusal = refused(&fakes::zip(&tampered).unwrap());
    assert!(refusal.starts_with(NOT_EXTRACTED), "{refusal}");
    assert!(refusal.contains("a sealed resource is missing or invalid"), "{refusal}");
}

#[test]
#[cfg(target_os = "macos")]
fn a_macos_archive_unpacks_as_its_own_candidate() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    mac_archive_tree(&source, "0.2.0");
    let asset = fixture.path().join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(File::create(&asset).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all(".", &source).unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut install = installation(fixture.path());
    install.format = Format::MacArchive;
    assert_eq!(
        extract(&install, &asset, work.path(), &Version::parse("0.2.0").unwrap()).unwrap(),
        work.path().join("unpacked")
    );
    assert!(work.path().join("unpacked/bin/ngterm-headless").is_file());
}

#[test]
#[cfg(target_os = "macos")]
fn a_linux_archive_is_refused_on_macos_after_unpacking() {
    let fixture = tempfile::tempdir().unwrap();
    let share = fixture.path().join("source/usr/share/ngterm");
    fs::create_dir_all(&share).unwrap();
    fs::write(share.join("app-BUILD"), "0.2.0\ncommit\n").unwrap();
    fs::write(share.join("update-format"), "linux-archive\n").unwrap();
    let asset = fixture.path().join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(File::create(&asset).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all("ngterm", fixture.path().join("source")).unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let work = tempfile::tempdir().unwrap();
    assert_eq!(
        extract(
            &installation(fixture.path()),
            &asset,
            work.path(),
            &Version::parse("0.2.0").unwrap()
        )
        .unwrap_err()
        .to_string(),
        "Downloaded archive has an incompatible installation format."
    );
}

#[test]
#[cfg(unix)]
fn a_missing_download_is_an_io_error_for_every_format_extraction_reads() {
    let fixture = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    for format in [Format::AppImage, Format::LinuxArchive, Format::MacArchive] {
        let work = tempfile::tempdir().unwrap();
        let mut install = installation(fixture.path());
        install.format = format;
        assert!(matches!(
            extract(&install, &fixture.path().join("missing"), work.path(), &version),
            Err(Error::Io(_))
        ));
    }
}

#[test]
#[cfg(unix)]
fn an_empty_error_file_path_refuses_the_launch_and_drops_the_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    fs::create_dir(&root).unwrap();
    let work = tempfile::tempdir_in(fixture.path()).unwrap();
    let stage = work.path().to_owned();
    let prepared = Prepared {
        sha256: String::new(),
        installation: installation(&root),
        candidate: work.path().join("unused"),
        work,
    };
    assert!(matches!(prepared.launch(Path::new(""), false, &[]), Err(Error::Io(_))));
    assert!(!stage.exists());
    assert!(root.exists());
}

#[test]
#[cfg(target_os = "macos")]
fn bundle_validation_reports_identity_format_and_each_missing_program() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join(fakes::ROOT);
    let version = Version::parse("0.2.0").unwrap();
    assert!(matches!(
        validate(&root, Format::MacBundle, &version),
        Err(Error::Io(_))
    ));
    tree(&root, "0.1.0");
    assert_eq!(
        validate(&root, Format::MacBundle, &version).unwrap_err().to_string(),
        "Downloaded archive identity does not match the release."
    );
    tree(&root, "0.2.0");
    validate(&root, Format::MacBundle, &version).unwrap();
    fs::write(root.join("Contents/Resources/update-format"), "windows-archive").unwrap();
    assert_eq!(
        validate(&root, Format::MacBundle, &version).unwrap_err().to_string(),
        "Downloaded archive has an incompatible installation format."
    );
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"] {
        tree(&root, "0.2.0");
        fs::remove_file(root.join("Contents/MacOS").join(name)).unwrap();
        assert_eq!(
            validate(&root, Format::MacBundle, &version).unwrap_err().to_string(),
            format!("Downloaded archive has no {name}.")
        );
    }
    fs::remove_file(root.join("Contents/Resources/update-format")).unwrap();
    assert!(matches!(
        validate(&root, Format::MacBundle, &version),
        Err(Error::Io(_))
    ));
}

#[test]
#[cfg(target_os = "macos")]
fn image_stamp_reports_a_failing_stat() {
    // BSD stat refuses -c for any path; a missing path also fails with GNU stat on PATH.
    let fixture = tempfile::tempdir().unwrap();
    assert_eq!(
        image_stamp(&fixture.path().join("missing")).unwrap_err().to_string(),
        "Could not identify the installed application image."
    );
}

#[test]
#[cfg(target_os = "linux")]
fn a_system_package_stays_as_downloaded_and_never_goes_to_the_tree_helper() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    tree(&root, "0.1.0");
    fs::write(root.join("usr").join(MARKER), "linux-deb\n").unwrap();
    let install = Installation::at(&root.join("usr/bin/ngterm")).unwrap();
    assert_eq!(install.format, Format::LinuxDeb);
    assert_eq!(install.root, root);
    assert_eq!(install.identity_file(), root.join("usr/share/ngterm/app-BUILD"));

    let asset = fixture.path().join("package.deb");
    fs::write(&asset, "package").unwrap();
    let work = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    assert_eq!(extract(&install, &asset, work.path(), &version).unwrap(), asset);
    fs::remove_dir(work.path().join("unpacked")).unwrap();

    let prepared = Prepared {
        sha256: "sum".into(),
        installation: install,
        candidate: asset.clone(),
        work,
    };
    let privileged = prepared.privileged().expect("a package goes to the system's installer");
    assert_eq!((privileged.kind, privileged.sha256.as_str()), ("deb", "sum"));
    assert_eq!(privileged.package, asset);
    assert_eq!(
        privileged.identity_sha256, "",
        "with no record, no installed identity can match"
    );
    assert_eq!(
        prepared
            .launch(&fixture.path().join("error"), true, &[])
            .unwrap_err()
            .to_string(),
        "The system's installer has to install this update."
    );
}

#[test]
fn an_installation_the_app_replaces_itself_has_no_privileged_install() {
    let fixture = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let prepared = Prepared {
        sha256: String::new(),
        installation: installation(fixture.path()),
        candidate: fixture.path().join("unused"),
        work,
    };
    assert!(prepared.privileged().is_none());
}
