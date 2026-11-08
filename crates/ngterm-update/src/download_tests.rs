use super::*;
#[cfg(unix)]
use crate::tests::metadata;
use crate::tests::{HttpFixture, TEST_KEY, installation, keyed};

struct Broken;
impl std::io::Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("metadata read failed"))
    }
}

#[test]
fn metadata_reads_are_bounded_and_propagate_io_errors() {
    assert_eq!(bounded_read(&b"abc"[..], 3).unwrap(), b"abc");
    assert!(bounded_read(&b""[..], 0).unwrap().is_empty());
    assert_eq!(
        bounded_read(&b"abcd"[..], 3).unwrap_err().to_string(),
        "Release metadata is too large."
    );
    assert_eq!(
        bounded_read(Broken, 10).unwrap_err().to_string(),
        "metadata read failed"
    );
}

#[test]
#[cfg(unix)]
fn selection_rejects_invalid_versions_and_asset_sizes() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installation(fixture.path());
    assert_eq!(
        select(&metadata("0.2.0"), DOWNLOADS, &install, "bad", &Trust::default())
            .unwrap_err()
            .to_string(),
        Version::parse("bad").unwrap_err().to_string()
    );
    for tag in ["0.2.0", "vbad"] {
        let mut release = metadata("0.2.0");
        release.tag_name = tag.into();
        assert_eq!(
            select(&release, DOWNLOADS, &install, "0.1.0", &Trust::default())
                .unwrap_err()
                .to_string(),
            "The release has an invalid version tag."
        );
    }
    assert!(
        select(&metadata("0.2.0-rc.1"), DOWNLOADS, &install, "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    for (index, size) in [(0, 0), (0, ASSET_LIMIT + 1), (1, METADATA_LIMIT + 1)] {
        let mut release = metadata("0.2.0");
        release.assets[index].size = size;
        assert_eq!(
            select(&release, DOWNLOADS, &install, "0.1.0", &Trust::default())
                .unwrap_err()
                .to_string(),
            "Release asset size is outside the supported range."
        );
    }
    let mut release = metadata("0.2.0");
    release.assets[0].size = ASSET_LIMIT;
    release.assets[1].size = METADATA_LIMIT;
    let selected = select(&release, DOWNLOADS, &install, "0.1.0", &Trust::default())
        .unwrap()
        .unwrap();
    assert_eq!(selected.version, Version::parse("0.2.0").unwrap());
    assert_eq!(selected.asset, release.assets[0].name);
    assert_eq!(selected.url, release.assets[0].browser_download_url);
    assert_eq!(selected.checksum_url, release.assets[1].browser_download_url);
    assert_eq!(selected.size, ASSET_LIMIT);
}

#[test]
fn invalid_signature_and_manifest_encodings_are_refused() {
    let version = Version::parse("0.2.0").unwrap();
    assert_eq!(
        verified_manifest(&keyed(TEST_KEY), &version, vec![], Some(&[0xff]))
            .unwrap_err()
            .to_string(),
        "Release signature verification failed. The installation was not changed."
    );
    let invalid = vec![0xff];
    let expected = String::from_utf8(invalid.clone()).unwrap_err().to_string();
    assert_eq!(
        verified_manifest(&Trust::default(), &version, invalid, None)
            .unwrap_err()
            .to_string(),
        expected
    );
    assert_eq!(
        checksum(&format!("ignored\n{}  asset", "AB".repeat(32)), "asset").unwrap(),
        "ab".repeat(32)
    );
    assert_eq!(
        checksum(&format!("{}  asset", "zz".repeat(32)), "asset")
            .unwrap_err()
            .to_string(),
        "Release checksum is invalid."
    );
}

fn local_release() -> Release {
    Release {
        version: Version::parse("0.2.0").unwrap(),
        asset: "asset".into(),
        url: "http://127.0.0.1:1/asset".into(),
        checksum_url: "http://127.0.0.1:1/SHA256SUMS".into(),
        signature_url: None,
        size: 3,
    }
}

#[test]
fn cancellation_happens_before_network_or_file_access() {
    let fixture = tempfile::tempdir().unwrap();
    let release = local_release();
    let mut events = Vec::new();
    let result = Source::github().prepare(
        &installation(&fixture.path().join("missing")),
        &release,
        &Trust::default(),
        |a, b| {
            events.push((a, b));
            false
        },
    );
    assert_eq!(result.unwrap_err().to_string(), "Update download was cancelled.");
    assert_eq!(events, [(0, 3)]);
    let target = fixture.path().join("asset");
    assert_eq!(
        download(&agent(), &release, &target, "", &mut |_, _| false)
            .unwrap_err()
            .to_string(),
        "Update download was cancelled."
    );
    assert!(!target.exists());
}

#[test]
#[cfg(unix)]
fn prepare_refuses_stale_or_invalid_installed_identity_and_cleans_its_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let release = local_release();
    for (identity, expected) in [
        ("not a version", "The installed version identity is invalid."),
        (
            "0.2.0",
            "The installation was updated while checking for releases. Check for updates again.",
        ),
        (
            "0.3.0",
            "The installation was updated while checking for releases. Check for updates again.",
        ),
    ] {
        let install = installed(fixture.path(), identity);
        assert_eq!(
            Source::github()
                .prepare(&install, &release, &Trust::default(), |_, _| true)
                .unwrap_err()
                .to_string(),
            expected
        );
        untouched(fixture.path(), identity);
    }
    let install = installed(fixture.path(), "0.1.0");
    assert!(matches!(
        Source::github().prepare(&install, &release, &Trust::default(), |_, _| true),
        Err(Error::Http(_))
    ));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    fs::remove_file(install.identity_file()).unwrap();
    assert!(matches!(
        Source::github().prepare(&install, &release, &Trust::default(), |_, _| true),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

#[test]
fn successful_download_writes_exact_bytes_and_reports_monotonic_progress() {
    let body: Vec<_> = (0_u8..=255).cycle().take(50_000).collect();
    let expected = "9d3550b2e0ae28ea766fd775454403cd4888c27509cb10d1be190f89b3f1decd";
    let server = HttpFixture::new("200 OK", &body, body.len());
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("download");
    let mut release = local_release();
    release.url.clone_from(&server.url);
    release.size = u64::try_from(body.len()).unwrap();
    let mut events = Vec::new();
    download(&http_agent(), &release, &target, expected, &mut |count, size| {
        events.push((count, size));
        true
    })
    .unwrap();
    assert_eq!(fs::read(target).unwrap(), body);
    assert_eq!(events[0], (0, release.size));
    assert_eq!(events.last(), Some(&(release.size, release.size)));
    assert!(events.windows(2).all(|pair| pair[0].0 < pair[1].0));
    assert!(
        events
            .iter()
            .all(|&(count, size)| size == release.size && count <= size)
    );
}

#[test]
fn transport_errors_preserve_their_exact_refusal_messages() {
    let fixture = tempfile::tempdir().unwrap();
    for (size, expected, cancel, message) in [
        (
            2,
            "00".repeat(32),
            false,
            "Downloaded release exceeds its declared size.",
        ),
        (
            4,
            "00".repeat(32),
            false,
            "Downloaded release failed SHA-256 verification. The installation was not changed.",
        ),
        (
            3,
            "00".repeat(32),
            false,
            "Downloaded release failed SHA-256 verification. The installation was not changed.",
        ),
        (3, "00".repeat(32), true, "Update download was cancelled."),
    ] {
        let server = HttpFixture::new("200 OK", b"abc", 3);
        let mut release = local_release();
        release.url.clone_from(&server.url);
        release.size = size;
        assert_eq!(
            download(
                &http_agent(),
                &release,
                &fixture.path().join("asset"),
                &expected,
                &mut |count, _| !cancel || count == 0
            )
            .unwrap_err()
            .to_string(),
            message
        );
    }
}

#[test]
fn http_status_truncation_and_destination_errors_are_propagated() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("asset");
    let mut release = local_release();
    {
        let server = HttpFixture::new("404 Not Found", &[], 0);
        release.url.clone_from(&server.url);
        assert!(matches!(
            download(&http_agent(), &release, &target, "", &mut |_, _| true),
            Err(Error::Http(ureq::Error::StatusCode(404)))
        ));
        assert!(!target.exists());
    }
    {
        let server = HttpFixture::new("200 OK", &[], 3);
        release.url.clone_from(&server.url);
        assert!(matches!(
            download(&http_agent(), &release, &target, "", &mut |_, _| true),
            Err(Error::Io(_))
        ));
    }
    {
        let server = HttpFixture::new("200 OK", b"abc", 3);
        release.url.clone_from(&server.url);
        assert!(matches!(
            download(&http_agent(), &release, fixture.path(), "", &mut |_, _| true),
            Err(Error::Io(_))
        ));
    }
}

#[test]
#[cfg(unix)]
fn prepare_requires_an_accessible_installation_and_a_parent_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let release = local_release();
    let missing = installation(&fixture.path().join("missing"));
    assert!(matches!(
        Source::github().prepare(&missing, &release, &Trust::default(), |_, _| true),
        Err(Error::Io(_))
    ));
    assert_eq!(
        Source::github()
            .prepare(&installation(Path::new("/")), &release, &Trust::default(), |_, _| true)
            .unwrap_err()
            .to_string(),
        "Installation has no parent directory."
    );
}

#[test]
#[cfg(unix)]
fn image_prepare_keeps_the_original_file_when_transport_is_refused() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm.AppImage");
    fs::write(&root, "original image").unwrap();
    let mut install = installation(&root);
    install.format = crate::Format::AppImage;
    install.image_identity = Some(b"device:inode:size:time\n".to_vec());
    assert!(matches!(
        Source::github().prepare(&install, &local_release(), &Trust::default(), |_, _| true),
        Err(Error::Http(_))
    ));
    assert_eq!(fs::read_to_string(&root).unwrap(), "original image");
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

// What follows runs `Source::check` and `Source::prepare` end to end over a release on 127.0.0.1 (`fakes::Server`),
// into an installation tree in a temp directory: a Linux archive's or a macOS bundle's, the tree its platform accepts.

#[cfg(unix)]
use crate::fakes;

/// An installation at `version` under `fixture`, found as the running one would be.
#[cfg(unix)]
fn installed(fixture: &Path, version: &str) -> Installation {
    let root = fixture.join(fakes::ROOT);
    fakes::tree(&root, version).unwrap();
    fakes::installation(&fakes::executable(&root)).unwrap()
}

/// Asserts the installation under `fixture` still holds `version` and no stage was left beside it.
#[cfg(unix)]
fn untouched(fixture: &Path, version: &str) {
    assert_eq!(
        fs::read_to_string(fakes::identity(&fixture.join(fakes::ROOT))).unwrap(),
        format!("{version}\ncommit\n")
    );
    assert_eq!(fs::read_dir(fixture).unwrap().count(), 1, "the stage is removed");
}

#[test]
#[cfg(unix)]
fn a_newer_release_is_checked_downloaded_verified_and_staged_beside_the_installation() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installed(fixture.path(), "0.1.0");
    let server = fakes::Server::start().unwrap();
    let archive = fakes::archive("0.2.0").unwrap();
    server.publish_archive("0.2.0", &archive);
    let source = server.source();
    assert!(
        source.check(&install, "0.2.0", &Trust::default()).unwrap().is_none(),
        "the running version is current"
    );
    let release = source.check(&install, "0.1.0", &Trust::default()).unwrap().unwrap();
    let asset = fakes::asset("0.2.0");
    assert_eq!(release.version, Version::parse("0.2.0").unwrap());
    assert_eq!(release.url, server.url(&format!("/download/v0.2.0/{asset}")));
    assert_eq!(release.signature_url, None, "no key, no signature");
    let mut events = Vec::new();
    let prepared = source
        .prepare(&install, &release, &Trust::default(), |count, size| {
            events.push((count, size));
            true
        })
        .unwrap();
    let size = u64::try_from(archive.len()).unwrap();
    assert_eq!(events.first(), Some(&(0, size)));
    assert_eq!(events.last(), Some(&(size, size)));
    assert_eq!(
        server.requests(),
        [
            "/latest".to_owned(),
            "/latest".to_owned(),
            "/download/v0.2.0/SHA256SUMS".to_owned(),
            format!("/download/v0.2.0/{asset}"),
        ]
    );
    let stage = prepared.work.path().to_owned();
    assert_eq!(
        stage.parent(),
        Some(dunce::canonicalize(fixture.path()).unwrap().as_path()),
        "staged beside the installation"
    );
    assert_eq!(prepared.candidate, stage.join("unpacked").join(fakes::ROOT));
    assert_eq!(
        fs::read_to_string(fakes::identity(&prepared.candidate)).unwrap(),
        "0.2.0\ncommit\n"
    );
    assert!(!stage.join(&asset).exists(), "the archive is removed once unpacked");
    assert_eq!(
        fs::read_to_string(stage.join("expected-identity")).unwrap(),
        "0.1.0\ncommit\n"
    );
    drop(prepared);
    untouched(fixture.path(), "0.1.0");
}

#[test]
#[cfg(unix)]
fn a_check_reports_transport_and_metadata_failures_with_their_messages() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installed(fixture.path(), "0.1.0");
    let server = fakes::Server::start().unwrap();
    let source = server.source();
    let check = |source: &Source| {
        source
            .check(&install, "0.1.0", &Trust::default())
            .unwrap_err()
            .to_string()
    };
    assert_eq!(check(&source), "http status: 404", "no release published");
    server.route("/latest", "500 Internal Server Error", "busy");
    assert_eq!(check(&source), "http status: 500");
    server.route("/latest", "200 OK", "not json");
    assert_eq!(
        check(&source),
        serde_json::from_str::<serde_json::Value>("not json")
            .unwrap_err()
            .to_string()
    );
    server.route("/latest", "200 OK", vec![b' '; 2 * 1024 * 1024 + 1]);
    assert_eq!(check(&source), "Release metadata is too large.");
    // a release whose assets live anywhere but under the source's downloads
    let elsewhere = fakes::Server::start().unwrap();
    elsewhere.publish_archive("0.2.0", b"archive");
    server.route(
        "/latest",
        "200 OK",
        http_agent()
            .get(&elsewhere.url("/latest"))
            .call()
            .unwrap()
            .body_mut()
            .read_to_vec()
            .unwrap(),
    );
    assert_eq!(check(&source), "Release asset has an unexpected download URL.");
    // a shipped build's agent never speaks plain HTTP, so it never even asks
    let shipped = Source {
        api: server.url("/latest"),
        ..Source::github()
    };
    let before = server.requests().len();
    assert!(check(&shipped).starts_with("configured for https only: "));
    assert_eq!(server.requests().len(), before);
    drop(server);
    assert!(matches!(
        source.check(&install, "0.1.0", &Trust::default()),
        Err(Error::Http(_))
    ));
}

/// The signed fixture manifest and its signature for v0.2.0 (tests.rs has how they were made).
#[cfg(unix)]
const SIGNED_SUMS: &[u8] = include_bytes!("tests/minisign/SHA256SUMS");
#[cfg(unix)]
const SIGNED_BY_TEST_KEY: &[u8] = include_bytes!("tests/minisign/SHA256SUMS.minisig");
#[cfg(unix)]
const SIGNED_BY_OTHER_KEY: &[u8] = include_bytes!("tests/minisign/wrong-key.minisig");

#[test]
#[cfg(unix)]
fn prepare_refuses_a_manifest_whose_signature_is_missing_unreachable_or_from_another_key() {
    const REFUSED: &str = "Release signature verification failed. The installation was not changed.";
    let fixture = tempfile::tempdir().unwrap();
    let install = installed(fixture.path(), "0.1.0");
    let trust = keyed(TEST_KEY);
    let server = fakes::Server::start().unwrap();
    let source = server.source();
    let asset = fakes::asset("0.2.0");
    let prepare = |release: &Release| {
        source
            .prepare(&install, release, &trust, |_, _| true)
            .unwrap_err()
            .to_string()
    };
    // a release checked with no key carries no signature, so a build that trusts one refuses it
    server.publish_archive("0.2.0", b"archive");
    let unsigned = source.check(&install, "0.1.0", &Trust::default()).unwrap().unwrap();
    assert_eq!(prepare(&unsigned), REFUSED);
    untouched(fixture.path(), "0.1.0");
    for (signature, expected) in [
        (SIGNED_BY_OTHER_KEY, REFUSED.to_owned()),
        (b"not a signature".as_slice(), REFUSED.to_owned()),
        // signed by the trusted key: the manifest checks out and names the asset with another archive's hash
        (
            SIGNED_BY_TEST_KEY,
            if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
                "Downloaded release failed SHA-256 verification. The installation was not changed."
            } else {
                "Release checksum is missing or ambiguous."
            }
            .to_owned(),
        ),
    ] {
        server.publish(
            "0.2.0",
            &[
                (&asset, b"archive"),
                ("SHA256SUMS", SIGNED_SUMS),
                ("SHA256SUMS.minisig", signature),
            ],
        );
        let release = source.check(&install, "0.1.0", &trust).unwrap().unwrap();
        assert_eq!(
            release.signature_url,
            Some(server.url("/download/v0.2.0/SHA256SUMS.minisig"))
        );
        assert_eq!(prepare(&release), expected);
        untouched(fixture.path(), "0.1.0");
    }
    server.route("/download/v0.2.0/SHA256SUMS.minisig", "404 Not Found", "");
    let release = source.check(&install, "0.1.0", &trust).unwrap().unwrap();
    assert_eq!(prepare(&release), "http status: 404");
    untouched(fixture.path(), "0.1.0");
}

#[test]
#[cfg(unix)]
fn prepare_refuses_an_archive_the_manifest_or_the_release_does_not_vouch_for() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installed(fixture.path(), "0.1.0");
    let server = fakes::Server::start().unwrap();
    let source = server.source();
    let asset = fakes::asset("0.2.0");
    let archive = fakes::archive("0.2.0").unwrap();
    let prepare = || {
        let release = source.check(&install, "0.1.0", &Trust::default()).unwrap().unwrap();
        let refused = source
            .prepare(&install, &release, &Trust::default(), |_, _| true)
            .unwrap_err()
            .to_string();
        untouched(fixture.path(), "0.1.0");
        refused
    };
    let other = fakes::manifest(&[("ngterm-0.2.0-other.tar.gz", &archive)]);
    server.publish("0.2.0", &[(&asset, &archive), ("SHA256SUMS", &other)]);
    assert_eq!(prepare(), "Release checksum is missing or ambiguous.");
    let stale = fakes::manifest(&[(&asset, b"another archive")]);
    server.publish("0.2.0", &[(&asset, &archive), ("SHA256SUMS", &stale)]);
    assert_eq!(
        prepare(),
        "Downloaded release failed SHA-256 verification. The installation was not changed."
    );
    // a verified archive of another version
    server.publish_archive("0.2.0", &fakes::archive("0.3.0").unwrap());
    assert_eq!(prepare(), "Downloaded archive identity does not match the release.");
    server.publish_archive("0.2.0", &archive);
    server.route(&format!("/download/v0.2.0/{asset}"), "404 Not Found", "");
    assert_eq!(prepare(), "http status: 404");
    server.route("/download/v0.2.0/SHA256SUMS", "503 Service Unavailable", "");
    assert_eq!(prepare(), "http status: 503");
}
