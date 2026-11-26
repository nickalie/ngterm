use semver::Version;

use super::{create_private, file_sha256, linux_archive};
use crate::Trust;
use crate::fakes::{self, Server};
use crate::tests::{TEST_KEY, keyed};

const SUMS: &[u8] = include_bytes!("tests/minisign/SHA256SUMS");
const SIGNED: &[u8] = include_bytes!("tests/minisign/SHA256SUMS.minisig");
const SIGNED_BY_OTHER_KEY: &[u8] = include_bytes!("tests/minisign/wrong-key.minisig");
const SIGNED_FOR_ANOTHER_RELEASE: &[u8] = include_bytes!("tests/minisign/other-release.minisig");
const REFUSED: &str = "Release signature verification failed. The installation was not changed.";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn the_linux_archive_is_named_as_the_release_names_it() {
    assert_eq!(
        linux_archive(&v("1.2.3"), "x86_64").unwrap(),
        "ngterm-1.2.3-linux-x86_64.tar.gz"
    );
    assert_eq!(
        linux_archive(&v("1.2.3"), "aarch64").unwrap(),
        "ngterm-1.2.3-linux-aarch64.tar.gz"
    );
    assert!(linux_archive(&v("1.2.3"), "riscv64").is_err());
}

#[test]
fn a_signed_manifest_vouches_for_the_asset_of_its_own_release() {
    let server = Server::start().unwrap();
    server.route("/download/v0.2.0/SHA256SUMS", "200 OK", SUMS);
    server.route("/download/v0.2.0/SHA256SUMS.minisig", "200 OK", SIGNED);
    let checksum = server
        .source()
        .published_checksum(&v("0.2.0"), "ngterm-0.2.0-linux-x86_64.tar.gz", &keyed(TEST_KEY))
        .unwrap();
    assert_eq!(
        checksum,
        "2b0e6d1b5d1b3c4e0f9a8d7c6b5a49382716051f2e3d4c5b6a79881726354453"
    );
}

#[test]
fn a_manifest_the_trusted_key_does_not_vouch_for_is_refused() {
    let server = Server::start().unwrap();
    server.route("/download/v0.2.0/SHA256SUMS", "200 OK", SUMS);
    let source = server.source();
    let checksum = || {
        source
            .published_checksum(&v("0.2.0"), "ngterm-0.2.0-linux-x86_64.tar.gz", &keyed(TEST_KEY))
            .unwrap_err()
            .to_string()
    };
    assert_eq!(checksum(), "http status: 404");
    for signature in [SIGNED_BY_OTHER_KEY, SIGNED_FOR_ANOTHER_RELEASE, b"not a signature"] {
        server.route("/download/v0.2.0/SHA256SUMS.minisig", "200 OK", signature);
        assert_eq!(checksum(), REFUSED);
    }
}

#[test]
fn the_asset_must_have_exactly_one_line() {
    let server = Server::start().unwrap();
    let archive = b"archive".as_slice();
    let one = fakes::manifest(&[("a.tar.gz", archive)]);
    let twice = fakes::manifest(&[("a.tar.gz", archive), ("a.tar.gz", archive)]);
    let source = server.source();
    for (manifest, asset) in [(one.clone(), "b.tar.gz"), (twice, "a.tar.gz")] {
        server.route("/download/v1.0.0/SHA256SUMS", "200 OK", manifest);
        let expected = Err("Release checksum is missing or ambiguous.".to_owned());
        let result = source
            .published_checksum(&v("1.0.0"), asset, &Trust::default())
            .map_err(|error| error.to_string());
        assert_eq!(result, expected);
    }
    server.route("/download/v1.0.0/SHA256SUMS", "200 OK", one);
    let checksum = source
        .published_checksum(&v("1.0.0"), "a.tar.gz", &Trust::default())
        .unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), archive).unwrap();
    assert_eq!(checksum, file_sha256(file.path()).unwrap());
}

#[test]
fn a_download_writes_a_private_file_up_to_its_limit() {
    let server = Server::start().unwrap();
    server.route("/download/v1.0.0/a.tar.gz", "200 OK", b"0123456789".as_slice());
    let source = server.source();
    let dir = tempfile::tempdir().unwrap();
    let exact = dir.path().join("exact");
    source.fetch_asset(&v("1.0.0"), "a.tar.gz", &exact, 10).unwrap();
    assert_eq!(std::fs::read(&exact).unwrap(), b"0123456789");
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&exact).unwrap().permissions()) & 0o777,
        0o600
    );
    let over = dir.path().join("over");
    let refused = source.fetch_asset(&v("1.0.0"), "a.tar.gz", &over, 9).unwrap_err();
    assert_eq!(refused.to_string(), "larger than 9 bytes");
    let missing = source
        .fetch_asset(&v("1.0.0"), "b.tar.gz", &dir.path().join("missing"), 10)
        .unwrap_err();
    assert_eq!(missing.to_string(), "http status: 404");
    assert!(!dir.path().join("missing").exists());
    assert_eq!(
        source.asset_url(&v("1.0.0"), "a.tar.gz"),
        server.url("/download/v1.0.0/a.tar.gz")
    );
}

#[test]
fn a_private_file_is_never_one_that_exists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("taken");
    std::fs::write(&path, b"x").unwrap();
    assert_eq!(
        create_private(&path).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
}
