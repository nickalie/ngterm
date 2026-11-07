use super::*;
use crate::download::{Asset, Metadata};
use std::io::{BufRead as _, Write as _};

mod regressions;

#[test]
#[cfg(target_os = "linux")]
fn application_images_replace_the_original_file_and_reopen_without_the_old_mount_environment() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("ngterm.AppImage");
    fs::write(&target, "old image").unwrap();
    let work = fixture.path().join("stage");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("expected-identity"), install::image_stamp(&target).unwrap()).unwrap();
    let candidate = work.join("candidate.AppImage");
    fs::write(
        &candidate,
        "#!/bin/sh\n[ -z \"$APPIMAGE$APPDIR\" ] || exit 7\nprintf updated > \"$NGTERM_UPDATE_TEST_RESULT\"\n",
    )
    .unwrap();
    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o755)).unwrap();
    let reopened = fixture.path().join("reopened");
    let error_file = fixture.path().join("error");
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(install::UNIX_HELPER)
        .arg("test-updater")
        .arg("99999999")
        .arg(&target)
        .arg(&candidate)
        .arg(&work)
        .arg(&target)
        .arg(&error_file)
        .arg("tree")
        .arg("reopen")
        .arg(&target)
        .arg("image")
        .env("APPIMAGE", "/old/image")
        .env("APPDIR", "/old/mount")
        .env("NGTERM_UPDATE_TEST_RESULT", &reopened)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(reopened).unwrap(), "updated");
    assert!(!error_file.exists());
    assert!(!work.exists());
}

fn record_test_installation(root: &Path, work: &Path) -> PathBuf {
    let identity = root.join("identity");
    fs::write(&identity, "0.1.0\nold-commit\n").unwrap();
    fs::copy(&identity, work.join("expected-identity")).unwrap();
    identity
}

#[test]
#[cfg(target_os = "linux")]
fn an_install_replaced_while_the_app_was_running_is_never_downgraded() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    tree(&root, "0.3.0");
    assert!(
        download::select(&metadata("0.2.0"), &installation(&root), "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    assert!(
        download::select(&metadata("0.3.0"), &installation(&root), "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    assert!(
        download::select(&metadata("0.4.0"), &installation(&root), "0.1.0", &Trust::default())
            .unwrap()
            .is_some()
    );
}

#[test]
#[cfg(target_os = "linux")]
fn shared_prefixes_are_never_treated_as_replaceable_archive_roots() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join(".local");
    tree(&root, "0.1.0");
    assert!(Installation::at(&root.join("usr/bin/ngterm")).is_err());
}

#[test]
#[cfg(unix)]
fn archive_symlinks_cannot_write_outside_the_staging_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let asset = fixture.path().join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(fs::File::create(&asset).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_mode(0o777);
    header.set_size(0);
    archive.append_link(&mut header, "ngterm/escape", &outside).unwrap();
    let mut header = tar::Header::new_gnu();
    header.set_size(4);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, "ngterm/escape/unwanted", &b"evil"[..])
        .unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let work = tempfile::tempdir().unwrap();
    assert!(
        install::extract(
            &installation(&fixture.path().join("ngterm")),
            &asset,
            work.path(),
            &Version::parse("0.2.0").unwrap()
        )
        .is_err()
    );
    assert!(!outside.join("unwanted").exists());
}

#[test]
#[cfg(unix)]
fn a_quit_installs_without_reopening_and_a_stale_download_leaves_the_newer_install_alone() {
    for changed in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("install");
        let work = fixture.path().join("stage");
        let candidate = work.join("candidate");
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        let identity = record_test_installation(&root, &work);
        fs::write(candidate.join("new"), "new").unwrap();
        if changed {
            fs::write(&identity, "0.3.0\nnewer-commit\n").unwrap();
        }
        let error_file = fixture.path().join("error");
        let status = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(install::UNIX_HELPER)
            .arg("test-updater")
            .arg("99999999")
            .arg(&root)
            .arg(&candidate)
            .arg(&work)
            .arg(root.join("must-not-be-executed"))
            .arg(&error_file)
            .arg("tree")
            .arg("quit")
            .arg(&identity)
            .arg("identity")
            .status()
            .unwrap();
        assert_eq!(status.success(), !changed);
        assert_eq!(root.join("new").exists(), !changed);
        if changed {
            assert_eq!(fs::read_to_string(identity).unwrap(), "0.3.0\nnewer-commit\n");
            assert!(
                fs::read_to_string(error_file)
                    .unwrap()
                    .contains("installed version changed")
            );
        }
    }
}

#[test]
#[cfg(windows)]
fn windows_helper_replaces_or_rolls_back_a_tree_and_respects_quit() {
    for (missing, changed) in [(false, false), (true, false), (false, true)] {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("install with spaces and 'quotes'");
        let work = fixture.path().join("stage");
        let candidate = work.join("candidate");
        fs::create_dir(&root).unwrap();
        fs::create_dir_all(&candidate).unwrap();
        let identity = record_test_installation(&root, &work);
        if changed {
            fs::write(&identity, "0.3.0\nexternal-update\n").unwrap();
        }
        fs::write(candidate.join("new"), "new").unwrap();
        let script = fixture.path().join("install.ps1");
        fs::write(&script, install::WINDOWS_HELPER).unwrap();
        let error_file = fixture.path().join("error");
        let candidate = if missing { work.join("missing") } else { candidate };
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .arg("99999999")
            .arg(&root)
            .arg(candidate)
            .arg(&work)
            .arg(root.join("must-not-be-executed.exe"))
            .arg(&error_file)
            .arg("tree")
            .arg("quit")
            .arg(identity)
            .arg("identity")
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(root.join("new").exists(), !(missing || changed));
        assert_eq!(error_file.exists(), missing || changed);
        if missing {
            assert!(
                fs::read_to_string(&error_file)
                    .unwrap()
                    .contains("previous installation was restored")
            );
        }
        assert_eq!(root.join("identity").exists(), missing || changed);
    }
}

fn installation(root: &Path) -> Installation {
    Installation {
        root: root.to_owned(),
        executable: root.join("usr/bin/ngterm"),
        format: Format::LinuxArchive,
        image_identity: None,
    }
}
fn metadata(version: &str) -> Metadata {
    let tag = format!("v{version}");
    let name = format!("ngterm-{version}-linux-{}.tar.gz", std::env::consts::ARCH);
    Metadata {
        tag_name: tag.clone(),
        draft: false,
        prerelease: false,
        assets: [name, "SHA256SUMS".into()]
            .into_iter()
            .map(|name| Asset {
                browser_download_url: format!("https://github.com/nickalie/ngterm/releases/download/{tag}/{name}"),
                name,
                size: 100,
            })
            .collect(),
    }
}

#[test]
fn stable_selection_never_downgrades_or_uses_partial_releases() {
    let install = installation(Path::new("/tmp/ngterm"));
    assert!(
        download::select(&metadata("0.2.0"), &install, "0.1.0", &Trust::default())
            .unwrap()
            .is_some()
    );
    for version in ["0.1.0", "0.0.9"] {
        assert!(
            download::select(&metadata(version), &install, "0.1.0", &Trust::default())
                .unwrap()
                .is_none()
        );
    }
    let mut preview = metadata("0.2.0");
    preview.prerelease = true;
    assert!(
        download::select(&preview, &install, "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    let mut draft = metadata("0.2.0");
    draft.draft = true;
    assert!(
        download::select(&draft, &install, "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    let mut incomplete = metadata("0.2.0");
    incomplete.assets.pop();
    assert!(download::select(&incomplete, &install, "0.1.0", &Trust::default()).is_err());
}

#[test]
fn selection_rejects_external_assets_and_duplicate_manifests() {
    let install = installation(Path::new("/tmp/ngterm"));
    let mut release = metadata("0.2.0");
    release.assets[0].browser_download_url = "https://example.com/evil.tar.gz".into();
    assert!(download::select(&release, &install, "0.1.0", &Trust::default()).is_err());
    let mut release = metadata("0.2.0");
    release.assets.push(Asset {
        name: "SHA256SUMS".into(),
        browser_download_url: String::new(),
        size: 100,
    });
    assert!(download::select(&release, &install, "0.1.0", &Trust::default()).is_err());
}

// The minisign fixtures in tests/minisign/, made with the minisign CLI (0.12) and two test-only keys whose secret
// halves were thrown away:
//   minisign -G -W -p test.pub -s test.key; minisign -G -W -p other.pub -s other.key
//   minisign -S -s test.key -m SHA256SUMS -x SHA256SUMS.minisig -t "ngterm v0.2.0 SHA256SUMS"
//   minisign -S -s test.key -m SHA256SUMS -x other-release.minisig -t "ngterm v0.3.0 SHA256SUMS"
//   minisign -S -s other.key -m SHA256SUMS -x wrong-key.minisig -t "ngterm v0.2.0 SHA256SUMS"
const TEST_KEY: &str = include_str!("tests/minisign/test.pub");
const OTHER_KEY: &str = include_str!("tests/minisign/other.pub");
const SUMS: &[u8] = include_bytes!("tests/minisign/SHA256SUMS");
const SIGNATURE: &[u8] = include_bytes!("tests/minisign/SHA256SUMS.minisig");

fn keyed(key: &str) -> Trust {
    Trust::from_files(key).unwrap()
}

fn verified(trust: &Trust, version: &str, manifest: &[u8], signature: Option<&[u8]>) -> Result<String, Error> {
    download::verified_manifest(trust, &Version::parse(version).unwrap(), manifest.to_vec(), signature)
}

#[test]
fn a_signed_manifest_verifies_against_the_trusted_key() {
    let manifest = verified(&keyed(TEST_KEY), "0.2.0", SUMS, Some(SIGNATURE)).unwrap();
    assert_eq!(
        download::checksum(&manifest, "ngterm-0.2.0-linux-x86_64.tar.gz").unwrap(),
        "2b0e6d1b5d1b3c4e0f9a8d7c6b5a49382716051f2e3d4c5b6a79881726354453"
    );
}

#[test]
fn a_manifest_is_refused_under_another_key_tampered_or_signed_for_another_release() {
    let refused = |trust: &Trust, version: &str, manifest: &[u8], signature: Option<&[u8]>| {
        let error = verified(trust, version, manifest, signature).unwrap_err().to_string();
        assert!(error.contains("signature verification failed"), "{error}");
    };
    // the right signature, but the build trusts another key
    refused(&keyed(OTHER_KEY), "0.2.0", SUMS, Some(SIGNATURE));
    // signed by a key the build doesn't trust
    refused(
        &keyed(TEST_KEY),
        "0.2.0",
        SUMS,
        Some(include_bytes!("tests/minisign/wrong-key.minisig")),
    );
    // one byte of the manifest changed
    let mut tampered = SUMS.to_vec();
    tampered[0] ^= 1;
    refused(&keyed(TEST_KEY), "0.2.0", &tampered, Some(SIGNATURE));
    // a genuine signature replayed onto another release: the trusted comment names v0.3.0
    refused(
        &keyed(TEST_KEY),
        "0.2.0",
        SUMS,
        Some(include_bytes!("tests/minisign/other-release.minisig")),
    );
    // the signature for v0.2.0 offered for v0.3.0
    refused(&keyed(TEST_KEY), "0.3.0", SUMS, Some(SIGNATURE));
    // no signature, or garbage
    refused(&keyed(TEST_KEY), "0.2.0", SUMS, None);
    refused(&keyed(TEST_KEY), "0.2.0", SUMS, Some(b"not a signature"));
}

#[test]
fn with_no_key_the_manifest_needs_no_signature() {
    for empty in ["", " \n", "\n\n"] {
        let trust = keyed(empty);
        assert_eq!(verified(&trust, "0.2.0", SUMS, None).unwrap().as_bytes(), SUMS);
        // a signature, even a broken one, is not looked at
        assert!(verified(&trust, "0.2.0", SUMS, Some(b"garbage")).is_ok());
    }
    assert!(Trust::from_files("untrusted comment: x\nnot base64\n").is_err());
}

#[test]
fn the_committed_trust_parses() {
    Trust::release().unwrap();
}

#[test]
fn with_a_key_selection_requires_the_signature_asset() {
    let install = installation(Path::new("/tmp/ngterm"));
    let trust = keyed(TEST_KEY);
    // no SHA256SUMS.minisig: refused with a key, fine without
    assert!(download::select(&metadata("0.2.0"), &install, "0.1.0", &trust).is_err());
    let release = download::select(&metadata("0.2.0"), &install, "0.1.0", &Trust::default())
        .unwrap()
        .unwrap();
    assert!(release.signature_url.is_none());
    let mut signed = metadata("0.2.0");
    let signature = |url: String, size| Asset {
        name: "SHA256SUMS.minisig".into(),
        browser_download_url: url,
        size,
    };
    let url = "https://github.com/nickalie/ngterm/releases/download/v0.2.0/SHA256SUMS.minisig".to_string();
    signed.assets.push(signature(url.clone(), 300));
    let release = download::select(&signed, &install, "0.1.0", &trust).unwrap().unwrap();
    assert_eq!(release.signature_url.as_deref(), Some(url.as_str()));
    // from somewhere else, too large, or twice
    let mut external = metadata("0.2.0");
    external
        .assets
        .push(signature("https://example.com/SHA256SUMS.minisig".into(), 300));
    assert!(download::select(&external, &install, "0.1.0", &trust).is_err());
    let mut large = metadata("0.2.0");
    large.assets.push(signature(url.clone(), 1 << 20));
    assert!(download::select(&large, &install, "0.1.0", &trust).is_err());
    let mut twice = metadata("0.2.0");
    twice.assets.push(signature(url.clone(), 300));
    twice.assets.push(signature(url, 300));
    assert!(download::select(&twice, &install, "0.1.0", &trust).is_err());
}

#[test]
fn every_release_architecture_has_the_packaged_asset_name() {
    let version = Version::parse("1.2.3").unwrap();
    for (format, arch, suffix) in [
        (Format::LinuxArchive, "aarch64", "linux-aarch64.tar.gz"),
        (Format::AppImage, "x86_64", "linux-x86_64.AppImage"),
        (Format::MacBundle, "aarch64", "macos-arm64.app.zip"),
        (Format::MacArchive, "x86_64", "macos-x86_64.tar.gz"),
        (Format::WindowsArchive, "x86_64", "windows-x86_64.zip"),
        (Format::WindowsMsi, "x86_64", "windows-x86_64.msi"),
    ] {
        assert_eq!(format.asset(&version, arch).unwrap(), format!("ngterm-1.2.3-{suffix}"));
    }
    assert!(Format::WindowsMsi.asset(&version, "aarch64").is_err());
}

#[test]
fn checksum_requires_one_exact_filename_and_a_valid_hash() {
    let hash = "ab".repeat(32);
    assert_eq!(
        download::checksum(&format!("{hash}  app.tar.gz\n"), "app.tar.gz").unwrap(),
        hash
    );
    assert_eq!(
        download::checksum(&format!("{hash} *app.tar.gz\n"), "app.tar.gz").unwrap(),
        hash
    );
    assert!(download::checksum(&format!("{hash}  other-app.tar.gz\n"), "app.tar.gz").is_err());
    assert!(download::checksum("not-a-hash  app.tar.gz", "app.tar.gz").is_err());
    assert!(download::checksum(&format!("{hash}  app.tar.gz\n{hash}  app.tar.gz"), "app.tar.gz").is_err());
}

#[cfg(target_os = "linux")]
fn tree(root: &Path, version: &str) {
    fs::create_dir_all(root.join("usr/bin")).unwrap();
    fs::create_dir_all(root.join("usr/share/ngterm")).unwrap();
    fs::write(root.join("usr/share/ngterm/app-BUILD"), format!("{version}\ncommit\n")).unwrap();
    fs::write(root.join("usr").join(MARKER), "linux-archive\n").unwrap();
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"] {
        fs::write(root.join("usr/bin").join(name), "binary").unwrap();
    }
}

#[test]
#[cfg(target_os = "linux")]
fn detects_archive_root_and_keeps_package_manager_and_source_builds_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ngterm");
    tree(&root, "0.1.0");
    let install = Installation::at(&root.join("usr/bin/ngterm")).unwrap();
    assert_eq!(install.root, root);
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    let alias = dir.path().join("state-alias");
    std::os::unix::fs::symlink(&state, &alias).unwrap();
    assert!(install.contains_path(&state));
    assert!(install.contains_path(&alias));
    assert!(!install.contains_path(dir.path()));
    fs::remove_file(root.join("usr").join(MARKER)).unwrap();
    assert!(Installation::at(&root.join("usr/bin/ngterm")).is_err());
    assert!(Installation::at(Path::new("/src/target/debug/ngterm")).is_err());
}

#[test]
#[cfg(target_os = "linux")]
fn extracts_complete_archive_and_refuses_wrong_identity_or_missing_companions() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source/ngterm");
    tree(&source, "0.2.0");
    let asset = fixture.path().join("release.tar.gz");
    let encoder = flate2::write::GzEncoder::new(fs::File::create(&asset).unwrap(), flate2::Compression::fast());
    let mut archive = tar::Builder::new(encoder);
    archive.append_dir_all("ngterm", &source).unwrap();
    archive.into_inner().unwrap().finish().unwrap();
    let work = tempfile::tempdir().unwrap();
    let version = Version::parse("0.2.0").unwrap();
    let candidate = install::extract(
        &installation(&fixture.path().join("installed")),
        &asset,
        work.path(),
        &version,
    )
    .unwrap();
    assert!(candidate.join("usr/bin/ngterm-session").is_file());
    assert!(install::validate(&candidate, Format::LinuxArchive, &Version::parse("0.3.0").unwrap()).is_err());
    fs::remove_file(candidate.join("usr/bin/ngtermctl")).unwrap();
    assert!(install::validate(&candidate, Format::LinuxArchive, &version).is_err());
}

#[test]
#[cfg(unix)]
fn helper_replaces_complete_tree_reopens_original_path_and_cleans_up() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("install with spaces and 'quotes'");
    let work = fixture.path().join("stage with spaces");
    let candidate = work.join("candidate");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("old"), "old").unwrap();
    fs::create_dir_all(&candidate).unwrap();
    let identity = record_test_installation(&root, &work);
    let reopened = fixture.path().join("reopened");
    let executable = root.join("ngterm");
    let script = candidate.join("ngterm");
    fs::write(&script, "#!/bin/sh\nprintf updated > \"$NGTERM_UPDATE_TEST_RESULT\"\n").unwrap();
    fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    let error_file = fixture.path().join("error");
    fs::write(&error_file, "previous failure").unwrap();
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(install::UNIX_HELPER)
        .arg("test-updater")
        .arg("99999999")
        .arg(&root)
        .arg(&candidate)
        .arg(&work)
        .arg(executable)
        .arg(&error_file)
        .arg("tree")
        .arg("reopen")
        .arg(&identity)
        .arg("identity")
        .env("NGTERM_UPDATE_TEST_RESULT", &reopened)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(fs::read_to_string(reopened).unwrap(), "updated");
    assert!(!root.join("old").exists());
    assert!(!work.exists());
    assert!(!error_file.exists());
}

#[test]
#[cfg(unix)]
fn failed_install_rolls_back_and_reopens_the_previous_version() {
    use std::os::unix::fs::PermissionsExt as _;
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("install");
    let work = fixture.path().join("stage");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&work).unwrap();
    let identity = record_test_installation(&root, &work);
    let executable = root.join("ngterm");
    let reopened = fixture.path().join("reopened");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf restored > \"$NGTERM_UPDATE_TEST_RESULT\"\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let error_file = fixture.path().join("error");
    let status = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(install::UNIX_HELPER)
        .arg("test-updater")
        .arg("99999999")
        .arg(&root)
        .arg(work.join("missing-candidate"))
        .arg(&work)
        .arg(&executable)
        .arg(&error_file)
        .arg("tree")
        .arg("reopen")
        .arg(&identity)
        .arg("identity")
        .env("NGTERM_UPDATE_TEST_RESULT", &reopened)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(executable.exists());
    assert_eq!(fs::read_to_string(reopened).unwrap(), "restored");
    assert!(
        fs::read_to_string(error_file)
            .unwrap()
            .contains("previous installation was restored")
    );
}

#[test]
fn transport_rejects_wrong_hash_and_truncated_downloads_before_extracting() {
    const BODY: &[u8] = b"abc";
    // HTTP is confined to this fixture; production's agent enforces HTTPS, including redirects.
    fn response(body: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = std::io::BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                if request.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        });
        (url, thread)
    }
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let dir = tempfile::tempdir().unwrap();
    for (hash, size, success, cancel) in [
        (expected, 3, true, false),
        (expected, 4, false, false),
        (expected, 2, false, false),
        (&"00".repeat(32), 3, false, false),
        (expected, 3, false, true),
    ] {
        let (url, thread) = response(BODY);
        let release = Release {
            version: Version::parse("0.2.0").unwrap(),
            asset: "asset".into(),
            url,
            checksum_url: String::new(),
            signature_url: None,
            size,
        };
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(3)))
            .build()
            .into();
        assert_eq!(
            download::download(
                &agent,
                &release,
                &dir.path().join("asset"),
                hash,
                &mut |count, _| !cancel || count == 0
            )
            .is_ok(),
            success
        );
        thread.join().unwrap();
    }
}
