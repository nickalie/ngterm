use super::*;
use crate::tests::{TEST_KEY, installation, keyed};
#[cfg(target_os = "linux")]
use crate::tests::{metadata, tree};

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
#[cfg(target_os = "linux")]
fn selection_rejects_invalid_versions_and_asset_sizes() {
    let fixture = tempfile::tempdir().unwrap();
    let install = installation(fixture.path());
    assert_eq!(
        select(&metadata("0.2.0"), &install, "bad", &Trust::default())
            .unwrap_err()
            .to_string(),
        Version::parse("bad").unwrap_err().to_string()
    );
    for tag in ["0.2.0", "vbad"] {
        let mut release = metadata("0.2.0");
        release.tag_name = tag.into();
        assert_eq!(
            select(&release, &install, "0.1.0", &Trust::default())
                .unwrap_err()
                .to_string(),
            "The release has an invalid version tag."
        );
    }
    assert!(
        select(&metadata("0.2.0-rc.1"), &install, "0.1.0", &Trust::default())
            .unwrap()
            .is_none()
    );
    for (index, size) in [(0, 0), (0, ASSET_LIMIT + 1), (1, METADATA_LIMIT + 1)] {
        let mut release = metadata("0.2.0");
        release.assets[index].size = size;
        assert_eq!(
            select(&release, &install, "0.1.0", &Trust::default())
                .unwrap_err()
                .to_string(),
            "Release asset size is outside the supported range."
        );
    }
    let mut release = metadata("0.2.0");
    release.assets[0].size = ASSET_LIMIT;
    release.assets[1].size = METADATA_LIMIT;
    let selected = select(&release, &install, "0.1.0", &Trust::default()).unwrap().unwrap();
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
    let result = prepare(
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
#[cfg(target_os = "linux")]
fn prepare_refuses_stale_or_invalid_installed_identity_and_cleans_its_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm");
    let install = installation(&root);
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
        tree(&root, identity);
        assert_eq!(
            prepare(&install, &release, &Trust::default(), |_, _| true)
                .unwrap_err()
                .to_string(),
            expected
        );
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        assert_eq!(fs::read_to_string(root.join("usr/bin/ngterm")).unwrap(), "binary");
    }
    tree(&root, "0.1.0");
    assert!(matches!(
        prepare(&install, &release, &Trust::default(), |_, _| true),
        Err(Error::Http(_))
    ));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
    fs::remove_file(install.identity_file()).unwrap();
    assert!(matches!(
        prepare(&install, &release, &Trust::default(), |_, _| true),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

struct HttpFixture {
    url: String,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl HttpFixture {
    fn new(status: &str, body: Vec<u8>, declared_length: usize) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let headers = format!("HTTP/1.1 {status}\r\nContent-Length: {declared_length}\r\nConnection: close\r\n\r\n");
        let thread = std::thread::spawn(move || {
            use std::io::BufRead as _;
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "no download request arrived");
                        std::thread::yield_now();
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut reader = std::io::BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let _ = stream
                .write_all(headers.as_bytes())
                .and_then(|()| stream.write_all(&body));
        });
        Self {
            url,
            thread: Some(thread),
        }
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.thread.take().unwrap().join().unwrap();
    }
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

#[test]
fn successful_download_writes_exact_bytes_and_reports_monotonic_progress() {
    let body: Vec<_> = (0_u8..=255).cycle().take(50_000).collect();
    let expected = "9d3550b2e0ae28ea766fd775454403cd4888c27509cb10d1be190f89b3f1decd";
    let server = HttpFixture::new("200 OK", body.clone(), body.len());
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
        let server = HttpFixture::new("200 OK", b"abc".to_vec(), 3);
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
        let server = HttpFixture::new("404 Not Found", vec![], 0);
        release.url.clone_from(&server.url);
        assert!(matches!(
            download(&http_agent(), &release, &target, "", &mut |_, _| true),
            Err(Error::Http(ureq::Error::StatusCode(404)))
        ));
        assert!(!target.exists());
    }
    {
        let server = HttpFixture::new("200 OK", vec![], 3);
        release.url.clone_from(&server.url);
        assert!(matches!(
            download(&http_agent(), &release, &target, "", &mut |_, _| true),
            Err(Error::Io(_))
        ));
    }
    {
        let server = HttpFixture::new("200 OK", b"abc".to_vec(), 3);
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
        prepare(&missing, &release, &Trust::default(), |_, _| true),
        Err(Error::Io(_))
    ));
    assert_eq!(
        prepare(&installation(Path::new("/")), &release, &Trust::default(), |_, _| true)
            .unwrap_err()
            .to_string(),
        "Installation has no parent directory."
    );
}

#[test]
#[cfg(target_os = "linux")]
fn image_prepare_keeps_the_original_file_when_transport_is_refused() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("ngterm.AppImage");
    fs::write(&root, "original image").unwrap();
    let mut install = installation(&root);
    install.format = crate::Format::AppImage;
    install.image_identity = Some(crate::install::image_stamp(&root).unwrap());
    assert!(matches!(
        prepare(&install, &local_release(), &Trust::default(), |_, _| true),
        Err(Error::Http(_))
    ));
    assert_eq!(fs::read_to_string(&root).unwrap(), "original image");
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}
