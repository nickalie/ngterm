use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use clap::Parser;
use ngterm_core::remote::server_install::{self, DOWNLOAD_LIMIT, INSTALLER_COMMAND};
use semver::Version;
use sha2::{Digest as _, Sha256};

use ngterm_update::Trust;
use ngterm_update::fakes::{Server, manifest};

use super::{Effects, Install, LiveEffects, NOT_A_RELEASE, ServerCommand, release_of};
use crate::terminfo::{Connection, Input, InstallFailure, Outcome, ssh_command};
use crate::{Failure, Ngtermctl};

const RELEASE: &[u8] = b"release";

fn parse(argv: &[&str]) -> Install {
    let command_line = ["ngtermctl", "server", "install"].iter().chain(argv).copied();
    let parsed = Ngtermctl::try_parse_from(command_line).unwrap();
    assert_eq!(parsed.command.validate(), Ok(()));
    let crate::Command::Server(group) = parsed.command else {
        panic!("not a server command")
    };
    let ServerCommand::Install(install) = group.command;
    install
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Call {
    phase: &'static str,
    argv: Vec<String>,
    detail: String,
}

/// Upstream's `RecordedEffects`: every phase in order, with what reached each.
struct Fake {
    release: Option<Version>,
    probe: (Outcome, &'static [u8]),
    checksum: Result<String, String>,
    body: &'static [u8],
    download_fails: bool,
    unpack: Outcome,
    installer: Outcome,
    spawn_fails: Option<&'static str>,
    before_unpack: Option<Box<dyn FnMut()>>,
    calls: Vec<Call>,
    printed: Vec<String>,
    downloaded: Option<PathBuf>,
}

impl Fake {
    fn new(probe: &'static [u8]) -> Self {
        Self {
            release: Some(Version::new(1, 2, 3)),
            probe: (Outcome::Exited(0), probe),
            checksum: Ok(digest(RELEASE)),
            body: RELEASE,
            download_fails: false,
            unpack: Outcome::Exited(0),
            installer: Outcome::Exited(0),
            spawn_fails: None,
            before_unpack: None,
            calls: Vec::new(),
            printed: Vec::new(),
            downloaded: None,
        }
    }

    fn phases(&self) -> Vec<&'static str> {
        self.calls.iter().map(|call| call.phase).collect()
    }

    fn answer(&mut self, argv: &[String], input: Input<'_>) -> (Outcome, Vec<u8>) {
        match input {
            Input::Capture => {
                self.calls.push(Call {
                    phase: "probe",
                    argv: argv.to_vec(),
                    detail: String::new(),
                });
                (self.probe.0, self.probe.1.to_vec())
            }
            Input::File(mut file) => {
                if let Some(before) = &mut self.before_unpack {
                    before();
                }
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).unwrap();
                let detail = if bytes == RELEASE { "archive" } else { "other" };
                self.calls.push(Call {
                    phase: "unpack",
                    argv: argv.to_vec(),
                    detail: detail.to_owned(),
                });
                (self.unpack, Vec::new())
            }
            Input::Null => {
                self.calls.push(Call {
                    phase: "installer",
                    argv: argv.to_vec(),
                    detail: String::new(),
                });
                (self.installer, Vec::new())
            }
            Input::Bytes(_) => panic!("server install sends no bytes from memory"),
        }
    }
}

impl Effects for Fake {
    fn release(&self) -> Option<Version> {
        self.release.clone()
    }

    fn ssh(&mut self, argv: &[String], input: Input<'_>) -> Result<(Outcome, Vec<u8>), InstallFailure> {
        let answer = self.answer(argv, input);
        let phase = self.calls.last().map(|call| call.phase);
        if phase.is_some() && phase == self.spawn_fails {
            return Err(InstallFailure::SpawnFailed {
                operation: "posix_spawnp".to_owned(),
                reason: "No such file or directory".to_owned(),
            });
        }
        Ok(answer)
    }

    fn checksum(&mut self, version: &Version, asset: &str) -> Result<String, String> {
        self.calls.push(Call {
            phase: "checksum",
            argv: Vec::new(),
            detail: format!("v{version} {asset}"),
        });
        self.checksum.clone()
    }

    fn asset_url(&self, version: &Version, asset: &str) -> String {
        format!("https://example.invalid/v{version}/{asset}")
    }

    fn download(&mut self, version: &Version, asset: &str, file: &Path) -> Result<(), String> {
        self.calls.push(Call {
            phase: "download",
            argv: Vec::new(),
            detail: format!("v{version} {asset}"),
        });
        self.downloaded = Some(file.to_owned());
        if self.download_fails {
            return Err("http status: 404".to_owned());
        }
        let mut output = ngterm_update::release_asset::create_private(file).unwrap();
        std::io::Write::write_all(&mut output, self.body).unwrap();
        Ok(())
    }

    fn print(&mut self, line: &str) {
        self.printed.push(line.to_owned());
    }
}

fn install(fake: &mut Fake, argv: &[&str]) -> Result<u8, String> {
    parse(argv).run(fake).map_err(|failure| match failure {
        Failure::Local(message) => message,
        other => panic!("not a local failure: {other:?}"),
    })
}

#[test]
fn install_maps_its_typed_options_onto_the_connection() {
    let install = parse(&[
        "me@box", "-p", "2222", "-i", "/k1", "-i", "/k2", "-J", "bastion", "-F", "/cfg",
    ]);
    assert_eq!(
        install.connection(),
        Connection {
            destination: "me@box".to_owned(),
            port: Some(2222),
            identities: vec!["/k1".to_owned(), "/k2".to_owned()],
            jump: Some("bastion".to_owned()),
            config: Some("/cfg".to_owned()),
        }
    );
}

#[test]
fn from_names_a_local_archive() {
    assert_eq!(
        parse(&["--from", "/tmp/t.tar.gz", "box"]).source.as_deref(),
        Some("/tmp/t.tar.gz")
    );
    assert_eq!(parse(&["box"]).source, None);
}

#[test]
fn a_matching_archive_is_downloaded_unpacked_then_installed() {
    let mut fake = Fake::new(b"x86_64\n");
    assert_eq!(install(&mut fake, &["box"]), Ok(0));
    assert_eq!(fake.phases(), ["probe", "checksum", "download", "unpack", "installer"]);
    let prefix = ssh_command(
        &Connection {
            destination: "box".to_owned(),
            ..Connection::default()
        },
        "ssh",
    )
    .unwrap();
    assert_eq!(
        fake.calls[0].argv,
        [prefix.clone(), vec!["uname -m".to_owned()]].concat()
    );
    assert_eq!(fake.calls[1].detail, "v1.2.3 ngterm-1.2.3-linux-x86_64.tar.gz");
    assert_eq!(fake.calls[2].detail, "v1.2.3 ngterm-1.2.3-linux-x86_64.tar.gz");
    assert_eq!(
        fake.calls[3].argv,
        [prefix.clone(), vec![server_install::unpack_command()]].concat()
    );
    assert_eq!(fake.calls[3].detail, "archive");
    assert_eq!(
        fake.calls[4].argv,
        [prefix, vec![INSTALLER_COMMAND.to_owned()]].concat()
    );
    assert!(prefix_has_no_batch_mode(&fake.calls[0].argv));
    assert_eq!(fake.printed, ["installed ngterm-headless 1.2.3 on box"]);
}

fn prefix_has_no_batch_mode(argv: &[String]) -> bool {
    argv.iter().any(|arg| arg == "StdinNull=no") && !argv.iter().any(|arg| arg.contains("BatchMode"))
}

#[test]
fn the_download_is_removed_whatever_happens() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.installer = Outcome::Exited(1);
    assert_eq!(
        install(&mut fake, &["box"]),
        Err("the installer on box failed: exited(1)".to_owned())
    );
    let file = fake.downloaded.unwrap();
    assert!(!file.parent().unwrap().exists());
}

#[test]
fn a_mismatch_never_unpacks() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.body = b"tampered";
    assert_eq!(
        install(&mut fake, &["box"]),
        Err(
            "ngterm-1.2.3-linux-x86_64.tar.gz does not match the sha256 release v1.2.3 was published with; nothing \
             was installed"
                .to_owned()
        )
    );
    assert_eq!(fake.phases(), ["probe", "checksum", "download"]);
}

#[test]
fn incomplete_hooks_exit_3() {
    let mut fake = Fake::new(b"aarch64\n");
    fake.installer = Outcome::Exited(3);
    assert_eq!(install(&mut fake, &["box"]), Ok(3));
    assert_eq!(fake.calls[1].detail, "v1.2.3 ngterm-1.2.3-linux-aarch64.tar.gz");
    assert_eq!(
        fake.printed,
        ["installed ngterm-headless 1.2.3 on box; the agent hooks are incomplete"]
    );
}

#[test]
fn a_failed_installer_is_a_refusal() {
    for (outcome, shown) in [
        (Outcome::Exited(1), "exited(1)"),
        (Outcome::Exited(255), "exited(255)"),
        (Outcome::Signaled(9), "signaled(9)"),
    ] {
        let mut fake = Fake::new(b"x86_64\n");
        fake.installer = outcome;
        assert_eq!(
            install(&mut fake, &["box"]),
            Err(format!("the installer on box failed: {shown}"))
        );
        assert!(fake.printed.is_empty());
    }
}

#[test]
fn a_failed_unpack_never_runs_the_installer() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.unpack = Outcome::Exited(2);
    assert_eq!(
        install(&mut fake, &["box"]),
        Err("unpacking on box failed: exited(2)".to_owned())
    );
    assert_eq!(fake.phases(), ["probe", "checksum", "download", "unpack"]);
}

#[test]
fn ssh_that_cannot_start_stops_the_install_at_that_phase() {
    let phases: [&[&str]; 3] = [
        &["probe"],
        &["probe", "checksum", "download", "unpack"],
        &["probe", "checksum", "download", "unpack", "installer"],
    ];
    for expected in phases {
        let mut fake = Fake::new(b"x86_64\n");
        fake.spawn_fails = expected.last().copied();
        assert_eq!(
            install(&mut fake, &["box"]),
            Err("could not start ssh: posix_spawnp failed with No such file or directory".to_owned())
        );
        assert_eq!(fake.phases(), expected);
        assert!(fake.printed.is_empty());
    }
}

#[test]
fn an_unsupported_architecture_stops_before_the_checksum() {
    let mut fake = Fake::new(b"riscv64\n");
    assert_eq!(
        install(&mut fake, &["box"]),
        Err("unsupported host architecture 'riscv64'; ngterm-headless is built for aarch64 and x86_64".to_owned())
    );
    assert_eq!(fake.phases(), ["probe"]);
}

#[test]
fn a_failed_probe_stops_there() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.probe.0 = Outcome::Exited(255);
    assert_eq!(
        install(&mut fake, &["box"]),
        Err("ssh exited with exited(255) asking the host for its architecture".to_owned())
    );
    assert_eq!(fake.phases(), ["probe"]);
}

#[test]
fn a_failed_download_never_unpacks() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.download_fails = true;
    assert_eq!(
        install(&mut fake, &["box"]),
        Err(
            "could not download https://example.invalid/v1.2.3/ngterm-1.2.3-linux-x86_64.tar.gz: http status: 404"
                .to_owned()
        )
    );
    assert_eq!(fake.phases(), ["probe", "checksum", "download"]);
}

#[test]
fn a_release_without_a_usable_checksum_is_refused_before_the_download() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.checksum = Err("Release checksum is missing or ambiguous.".to_owned());
    assert_eq!(
        install(&mut fake, &["box"]),
        Err(
            "release v1.2.3 has no usable checksum for ngterm-1.2.3-linux-x86_64.tar.gz: Release checksum is \
             missing or ambiguous."
                .to_owned()
        )
    );
    assert_eq!(fake.phases(), ["probe", "checksum"]);
}

#[test]
fn a_development_build_is_refused_before_ssh_runs() {
    let mut fake = Fake::new(b"x86_64\n");
    fake.release = None;
    assert_eq!(install(&mut fake, &["box"]), Err(NOT_A_RELEASE.to_owned()));
    assert!(fake.calls.is_empty());
}

/// Upstream's `ServerCommandsTests`: a destination ssh would read as an option, or one with a space, is named.
#[test]
fn a_bad_destination_is_refused_by_name_before_anything_runs() {
    for bad in ["-oProxyCommand=x", "a b", ""] {
        let mut fake = Fake::new(b"x86_64\n");
        assert_eq!(
            install(&mut fake, &["--", bad]),
            Err(format!(
                "destination must be a host, user@host or an ssh alias, not '{bad}'"
            )),
            "{bad:?}"
        );
        assert!(fake.calls.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn a_local_archive_is_installed_from_a_snapshot_the_original_cannot_change() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.tar.gz");
    let link = dir.path().join("link.tar.gz");
    std::fs::write(&original, RELEASE).unwrap();
    std::os::unix::fs::symlink(&original, &link).unwrap();
    let mut fake = Fake::new(b"x86_64\n");
    let tampered = original.clone();
    fake.before_unpack = Some(Box::new(move || std::fs::write(&tampered, b"tampered").unwrap()));
    assert_eq!(install(&mut fake, &["--from", link.to_str().unwrap(), "box"]), Ok(0));
    assert_eq!(fake.phases(), ["probe", "checksum", "unpack", "installer"]);
    assert_eq!(fake.calls[2].detail, "archive");
}

#[test]
fn a_mismatched_local_archive_never_unpacks() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("local.tar.gz");
    std::fs::write(&local, b"other").unwrap();
    let mut fake = Fake::new(b"x86_64\n");
    assert!(install(&mut fake, &["--from", local.to_str().unwrap(), "box"]).is_err());
    assert_eq!(fake.phases(), ["probe", "checksum"]);
}

#[test]
fn an_oversize_local_archive_is_refused_before_it_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("huge.tar.gz");
    std::fs::File::create(&local)
        .unwrap()
        .set_len(DOWNLOAD_LIMIT + 1)
        .unwrap();
    let path = local.to_str().unwrap();
    let mut fake = Fake::new(b"x86_64\n");
    assert_eq!(
        install(&mut fake, &["--from", path, "box"]),
        Err(format!("{path} is larger than {DOWNLOAD_LIMIT} bytes"))
    );
    assert_eq!(fake.phases(), ["probe", "checksum"]);
}

#[test]
fn an_unreadable_local_archive_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let absent = dir.path().join("absent.tar.gz");
    for path in [absent.to_str().unwrap(), "", dir.path().to_str().unwrap()] {
        let mut fake = Fake::new(b"x86_64\n");
        assert_eq!(
            install(&mut fake, &["--from", path, "box"]),
            Err(format!("could not read {path} as a regular file")),
            "{path:?}"
        );
        assert_eq!(fake.phases(), ["probe", "checksum"]);
    }
}

/// A staged or packaged tree: `bin/ngtermctl` beside `share/ngterm/app-BUILD`, and the marker release packaging adds.
fn tree(root: &Path, identity: &str, marker: Option<&str>) -> PathBuf {
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("share/ngterm")).unwrap();
    std::fs::write(root.join("share/ngterm/app-BUILD"), identity).unwrap();
    if let Some(marker) = marker {
        std::fs::write(root.join("share/ngterm/update-format"), marker).unwrap();
    }
    root.join("bin/ngtermctl")
}

#[test]
fn only_a_release_marked_install_has_a_release() {
    let dir = tempfile::tempdir().unwrap();
    let released = tree(&dir.path().join("released"), "1.2.3\nabc\n", Some("linux-deb\n"));
    assert_eq!(release_of(&released), Some(Version::new(1, 2, 3)));
    let staged = tree(&dir.path().join("staged"), "1.2.3\nabc\n", None);
    assert_eq!(release_of(&staged), None);
    let blank = tree(&dir.path().join("blank"), "1.2.3\n", Some("\n"));
    assert_eq!(release_of(&blank), None);
    let unversioned = tree(
        &dir.path().join("unversioned"),
        "not a version\n",
        Some("linux-archive\n"),
    );
    assert_eq!(release_of(&unversioned), None);
}

#[cfg(target_os = "macos")]
#[test]
fn a_release_bundle_has_a_release() {
    let dir = tempfile::tempdir().unwrap();
    let contents = dir.path().join("ngterm.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::create_dir_all(contents.join("Resources")).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>1.2.3</string></dict></plist>"#,
    )
    .unwrap();
    let client = contents.join("MacOS/ngtermctl");
    assert_eq!(release_of(&client), None);
    std::fs::write(contents.join("Resources/update-format"), "macos-bundle\n").unwrap();
    assert_eq!(release_of(&client), Some(Version::new(1, 2, 3)));
}

const ASSET: &str = "ngterm-1.2.3-linux-x86_64.tar.gz";

fn live(server: &Server, trust: Result<Trust, String>) -> LiveEffects {
    LiveEffects {
        source: server.source(),
        trust,
    }
}

#[test]
fn the_live_effects_read_the_checksum_and_the_archive_the_release_published() {
    let server = Server::start().unwrap();
    let sums = manifest(&[(ASSET, RELEASE)]);
    server.publish("1.2.3", &[(ASSET, RELEASE), ("SHA256SUMS", &sums)]);
    let mut effects = live(&server, Ok(Trust::default()));
    let version = Version::new(1, 2, 3);

    assert_eq!(effects.checksum(&version, ASSET), Ok(digest(RELEASE)));
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(ASSET);
    effects.download(&version, ASSET, &file).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), RELEASE);
    assert_eq!(
        effects.asset_url(&version, ASSET),
        server.url(&format!("/download/v1.2.3/{ASSET}"))
    );
}

#[test]
fn the_live_effects_refuse_a_release_that_is_not_published() {
    let server = Server::start().unwrap();
    let mut effects = live(&server, Ok(Trust::default()));
    let version = Version::new(1, 2, 3);

    assert!(effects.checksum(&version, ASSET).is_err());
    let dir = tempfile::tempdir().unwrap();
    assert!(effects.download(&version, ASSET, &dir.path().join(ASSET)).is_err());
}

#[test]
fn the_live_effects_refuse_an_unsigned_manifest_under_the_release_key() {
    let server = Server::start().unwrap();
    let sums = manifest(&[(ASSET, RELEASE)]);
    server.publish("1.2.3", &[(ASSET, RELEASE), ("SHA256SUMS", &sums)]);
    let mut effects = live(&server, LiveEffects::github().trust);

    assert!(effects.checksum(&Version::new(1, 2, 3), ASSET).is_err());
}

#[test]
fn the_live_effects_report_an_unusable_key_without_a_request() {
    let server = Server::start().unwrap();
    let mut effects = live(&server, Err("bad key".to_owned()));

    assert_eq!(
        effects.checksum(&Version::new(1, 2, 3), ASSET),
        Err("bad key".to_owned())
    );
    assert_eq!(server.requests(), Vec::<String>::new());
}

#[test]
fn a_development_build_runs_no_install() {
    let parsed = Ngtermctl::try_parse_from(["ngtermctl", "server", "install", "host"]).unwrap();
    let crate::Command::Server(group) = parsed.command else {
        panic!("not a server command")
    };
    match group.execute() {
        Err(Failure::Local(message)) => assert_eq!(message, NOT_A_RELEASE),
        other => panic!("not refused as a development build: {other:?}"),
    }
}

#[cfg(unix)]
fn shell(script: &str) -> Vec<String> {
    vec!["sh".to_owned(), "-c".to_owned(), script.to_owned()]
}

#[cfg(unix)]
#[test]
fn the_live_ssh_captures_what_the_probe_prints() {
    let (outcome, output) = LiveEffects::github()
        .ssh(&shell("printf x86_64"), Input::Capture)
        .unwrap();
    assert_eq!((outcome, output.as_slice()), (Outcome::Exited(0), &b"x86_64"[..]));
}

#[cfg(unix)]
#[test]
fn the_live_ssh_streams_the_archive_on_stdin_and_reports_the_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(ASSET);
    std::fs::write(&file, RELEASE).unwrap();
    let mut effects = LiveEffects::github();

    let archive = std::fs::File::open(&file).unwrap();
    let (streamed, _) = effects
        .ssh(&shell(r#"test "$(cat)" = release"#), Input::File(archive))
        .unwrap();
    assert_eq!(streamed, Outcome::Exited(0));
    let (nothing, _) = effects.ssh(&shell(r#"test -z "$(cat)""#), Input::Null).unwrap();
    assert_eq!(nothing, Outcome::Exited(0));
    let (failed, _) = effects.ssh(&shell("exit 3"), Input::Null).unwrap();
    assert_eq!(failed, Outcome::Exited(3));
}
