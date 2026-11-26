//! `ngtermctl server install` (`agtermctlKit/ServerCommands.swift` `Server`): installs or upgrades `ngterm-headless`
//! on a Linux host over ssh. Local-only, as `terminfo install`: it never opens the control socket, so it has no
//! `--socket` or `--json`. It runs on a desktop of any OS, and installs the Linux release archive of this build's own
//! version, checked against the checksum that release was published with ([`ngterm_update::release_asset`]); the host
//! side is the archive's `server-install.sh` (`scripts/server-host-install.sh`). macOS and Windows hosts have no
//! systemd user manager to keep the server running, so they keep the manual install of docs/development.md.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ngterm_core::identity::InstalledSource;
use ngterm_core::remote::server_install::{self, ARCHITECTURES, DOWNLOAD_LIMIT, INSTALLER_COMMAND, PROBE_COMMAND};
use ngterm_update::release_asset::{self, create_private, file_sha256};
use ngterm_update::{MARKER, Source, Trust};
use semver::Version;

use crate::Failure;
use crate::terminfo::{Connection, Input, InstallFailure, Outcome, foreground_run, ssh_command};
use crate::version::client_path;

/// The refusal of a build that is not part of a release.
pub const NOT_A_RELEASE: &str =
    "this ngtermctl is not part of an ngterm release; a development build has no release to install";

/// `ngtermctl server <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "ngterm-headless on Linux hosts.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Server {
    #[command(subcommand)]
    command: ServerCommand,
}

/// The subcommands, in upstream's `Server.configuration.subcommands` order.
#[derive(Debug, clap::Subcommand)]
enum ServerCommand {
    Install(Install),
}

impl Server {
    /// Runs the command.
    pub fn execute(&self) -> Result<(), Failure> {
        match &self.command {
            ServerCommand::Install(command) => match command.run(&mut LiveEffects::github())? {
                0 => Ok(()),
                code => Err(Failure::Exit(ExitCode::from(code))),
            },
        }
    }
}

/// `ngtermctl server install [-p <port>] [-i <file> ...] [-J <host>] [-F <file>] [--from <file>] <destination>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Install or upgrade ngterm-headless on a Linux host over ssh.",
    long_about = "Installs the ngterm-headless release matching this ngtermctl's version: asks the host for its \
                  architecture (aarch64 or x86_64), downloads that release's Linux archive from GitHub, checks it \
                  against the sha256 the release was published with (its SHA256SUMS, signed with ngterm's release \
                  key), unpacks it into ~/.local/lib/ngterm and runs its installer, which sets up the systemd user \
                  service, enables linger and installs the agent hooks. Running it again upgrades in place and keeps \
                  every session. --from FILE uses an archive already on this machine instead of downloading one; it \
                  must match the same sha256 for the host's architecture.\n\nThe connection is interactive, as with \
                  terminfo install: a password or host-key prompt is answered on this terminal, and other settings \
                  belong in ~/.ssh/config. Exits 0 when installed, 3 when installed but the agent hooks are \
                  incomplete, and 1 on installation failure, with the reason on stderr. A development build has no \
                  release to install. A macOS or Windows host has no systemd user manager to keep the server running: \
                  install it there by hand."
)]
pub struct Install {
    #[arg(help = "The host, as ssh would take it: host, user@host, or an alias from ~/.ssh/config.")]
    destination: String,

    #[arg(short = 'p', value_name = "port", help = "Port to connect to on the remote host.")]
    port: Option<i64>,

    #[arg(
        short = 'i',
        value_name = "file",
        action = clap::ArgAction::Append,
        help = "Identity file for public key authentication. Repeat to offer several."
    )]
    identity: Vec<String>,

    #[arg(short = 'J', value_name = "host", help = "Jump host, as ssh -J takes it.")]
    jump: Option<String>,

    #[arg(short = 'F', value_name = "file", help = "Per-user ssh configuration file.")]
    config: Option<String>,

    #[arg(
        long = "from",
        value_name = "file",
        help = "Use a release archive already on this machine instead of downloading one; it must still match the release's sha256."
    )]
    source: Option<String>,
}

impl Install {
    fn connection(&self) -> Connection {
        Connection {
            destination: self.destination.clone(),
            port: self.port,
            identities: self.identity.clone(),
            jump: self.jump.clone(),
            config: self.config.clone(),
        }
    }

    /// Returns 0 or 3 once the host's installer ran; every refusal and failure is the error, with exit 1. A bad
    /// destination is one of them, so it is checked here, not as a usage error (upstream's `ServerInstaller.install`).
    fn run(&self, effects: &mut dyn Effects) -> Result<u8, Failure> {
        Installer {
            connection: self.connection(),
            source: self.source.as_deref(),
            effects,
        }
        .install()
        .map_err(Failure::Local)
    }
}

/// What an install reaches outside this process, so tests drive every refusal and the order of the ssh phases without
/// ssh or the network (upstream's `ServerInstaller.Effects`).
trait Effects {
    /// This build's release version; `None` for a build that is not part of a release.
    fn release(&self) -> Option<Version>;
    /// Runs ssh with `input` on stdin.
    fn ssh(&mut self, argv: &[String], input: Input<'_>) -> Result<(Outcome, Vec<u8>), InstallFailure>;
    /// The checksum release `version` was published with for `asset`.
    fn checksum(&mut self, version: &Version, asset: &str) -> Result<String, String>;
    /// Where `asset` of release `version` downloads from.
    fn asset_url(&self, version: &Version, asset: &str) -> String;
    /// Downloads `asset` of release `version` into the new private `file`, at most [`DOWNLOAD_LIMIT`] bytes.
    fn download(&mut self, version: &Version, asset: &str, file: &Path) -> Result<(), String>;
    fn print(&mut self, line: &str);
}

/// The process's effects: ssh on the terminal, the releases of `source` checked against `trust`.
struct LiveEffects {
    source: Source,
    trust: Result<Trust, String>,
}

impl LiveEffects {
    fn github() -> Self {
        Self {
            source: Source::github(),
            trust: Trust::release().map_err(|error| error.to_string()),
        }
    }
}

impl Effects for LiveEffects {
    fn release(&self) -> Option<Version> {
        release_of(&client_path()?, appimage_dir(|name| std::env::var_os(name)).as_deref())
    }

    fn ssh(&mut self, argv: &[String], input: Input<'_>) -> Result<(Outcome, Vec<u8>), InstallFailure> {
        let environment: HashMap<OsString, OsString> = std::env::vars_os().collect();
        foreground_run(&argv[0], &argv[1..], input, &environment)
    }

    fn checksum(&mut self, version: &Version, asset: &str) -> Result<String, String> {
        let trust = self.trust.as_ref().map_err(Clone::clone)?;
        self.source
            .published_checksum(version, asset, trust)
            .map_err(|error| error.to_string())
    }

    fn asset_url(&self, version: &Version, asset: &str) -> String {
        self.source.asset_url(version, asset)
    }

    fn download(&mut self, version: &Version, asset: &str, file: &Path) -> Result<(), String> {
        self.source
            .fetch_asset(version, asset, file, DOWNLOAD_LIMIT)
            .map_err(|error| error.to_string())
    }

    fn print(&mut self, line: &str) {
        println!("{line}");
    }
}

/// The release version of the CLI at `client`: its installed identity's, when release packaging marked the install
/// (`share/ngterm/update-format`, or `Contents/Resources/update-format` in a bundle), which a development tree never
/// is. An application image carries no marker, so a client inside the mounted `image` directory is a release too, as
/// `ngterm_update::Installation::detect` takes it.
fn release_of(client: &Path, image: Option<&Path>) -> Option<Version> {
    let source = InstalledSource::of_executable(client)?;
    if !image.is_some_and(|appdir| client.starts_with(appdir)) {
        let marker = match &source {
            InstalledSource::Staged(file) => file.parent()?.parent()?.parent()?.join(MARKER),
            InstalledSource::Bundle(plist) => plist.parent()?.join("Resources/update-format"),
        };
        fs::read_to_string(marker).ok().filter(|text| !text.trim().is_empty())?;
    }
    Version::parse(&source.read()?.version).ok()
}

/// The mounted directory of the application image this process runs from, when `var` names one (`APPDIR`, with `APPIMAGE`
/// the image file); only Linux has them.
fn appimage_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if !cfg!(target_os = "linux") || !Path::new(&var("APPIMAGE")?).is_file() {
        return None;
    }
    fs::canonicalize(var("APPDIR")?).ok()
}

/// One run of `server install` (upstream's `ServerInstaller`).
struct Installer<'a> {
    connection: Connection,
    source: Option<&'a str>,
    effects: &'a mut dyn Effects,
}

impl Installer<'_> {
    fn install(&mut self) -> Result<u8, String> {
        let destination = self.connection.destination.clone();
        let prefix = ssh_command(&self.connection, "ssh")
            .map_err(|_| format!("destination must be a host, user@host or an ssh alias, not '{destination}'"))?;
        let version = self.effects.release().ok_or_else(|| NOT_A_RELEASE.to_owned())?;
        let architecture = self.probe(&prefix)?;
        let asset = release_asset::linux_archive(&version, architecture).map_err(|error| error.to_string())?;
        let expected = self
            .effects
            .checksum(&version, &asset)
            .map_err(|error| format!("release v{version} has no usable checksum for {asset}: {error}"))?;

        let directory = tempfile::tempdir().map_err(failed("could not make a private directory".to_owned()))?;
        let file = directory.path().join(&asset);
        match self.source {
            Some(source) => snapshot(source, &file)?,
            None => self.effects.download(&version, &asset, &file).map_err(|error| {
                format!(
                    "could not download {}: {error}",
                    self.effects.asset_url(&version, &asset)
                )
            })?,
        }
        let actual = file_sha256(&file).map_err(failed(format!("could not read {}", file.display())))?;
        if actual != expected {
            return Err(format!(
                "{asset} does not match the sha256 release v{version} was published with; nothing was installed"
            ));
        }

        let archive = File::open(&file).map_err(failed(format!("could not read {}", file.display())))?;
        let unpack = [prefix.clone(), vec![server_install::unpack_command()]].concat();
        let (unpacked, _) = self
            .effects
            .ssh(&unpack, Input::File(archive))
            .map_err(|failure| failure.message())?;
        if unpacked != Outcome::Exited(0) {
            return Err(format!("unpacking on {destination} failed: {unpacked}"));
        }
        let run_installer = [prefix, vec![INSTALLER_COMMAND.to_owned()]].concat();
        let (installed, _) = self
            .effects
            .ssh(&run_installer, Input::Null)
            .map_err(|failure| failure.message())?;
        let status = match installed {
            Outcome::Exited(status) => Some(status),
            Outcome::Signaled(_) => None,
        };
        let code = server_install::exit_code(status);
        match code {
            0 => self
                .effects
                .print(&format!("installed ngterm-headless {version} on {destination}")),
            3 => self.effects.print(&format!(
                "installed ngterm-headless {version} on {destination}; the agent hooks are incomplete"
            )),
            _ => return Err(format!("the installer on {destination} failed: {installed}")),
        }
        Ok(code)
    }

    fn probe(&mut self, prefix: &[String]) -> Result<&'static str, String> {
        let argv = [prefix, &[PROBE_COMMAND.to_owned()]].concat();
        let (outcome, output) = self
            .effects
            .ssh(&argv, Input::Capture)
            .map_err(|failure| failure.message())?;
        if outcome != Outcome::Exited(0) {
            return Err(format!(
                "ssh exited with {outcome} asking the host for its architecture"
            ));
        }
        server_install::architecture(&output).map_err(|name| {
            format!(
                "unsupported host architecture '{name}'; ngterm-headless is built for {}",
                ARCHITECTURES.join(" and ")
            )
        })
    }
}

/// The message of an I/O step that failed, `what` then the OS error.
fn failed(what: String) -> impl FnOnce(io::Error) -> String {
    move |error| format!("{what}: {error}")
}

/// Copies `source` through one checked descriptor into the private `file`, which alone is hashed and streamed: a copy
/// of the path would follow a symlink whose target can change between the hash and the unpack.
fn snapshot(source: &str, file: &Path) -> Result<(), String> {
    let unreadable = || copy_failure(CopyFailure::Read, source, file);
    let too_large = || copy_failure(CopyFailure::TooLarge, source, file);
    let mut input = File::open(source).map_err(|_| unreadable())?;
    let metadata = input.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(unreadable());
    }
    if metadata.len() > DOWNLOAD_LIMIT {
        return Err(too_large());
    }
    let mut output = create_private(file).map_err(failed(unwritable(file)))?;
    copy_bounded(&mut input, &mut output).map_err(|failure| copy_failure(failure, source, file))
}

fn unwritable(file: &Path) -> String {
    format!("could not write {}", file.display())
}

/// The message of a failed [`snapshot`] copy from `source` into `file`.
fn copy_failure(failure: CopyFailure, source: &str, file: &Path) -> String {
    match failure {
        CopyFailure::Read => format!("could not read {source} as a regular file"),
        CopyFailure::TooLarge => format!("{source} is larger than {DOWNLOAD_LIMIT} bytes"),
        CopyFailure::Write(error) => failed(unwritable(file))(error),
    }
}

/// Why [`copy_bounded`] stopped.
#[derive(Debug)]
enum CopyFailure {
    Read,
    TooLarge,
    Write(io::Error),
}

/// Copies at most [`DOWNLOAD_LIMIT`] bytes, telling a read failure from a write failure.
fn copy_bounded(input: &mut dyn io::Read, output: &mut dyn io::Write) -> Result<(), CopyFailure> {
    let mut buffer = vec![0; 64 * 1024];
    let mut copied = 0;
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => return output.flush().map_err(CopyFailure::Write),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(CopyFailure::Read),
        };
        copied += read as u64;
        if copied > DOWNLOAD_LIMIT {
            return Err(CopyFailure::TooLarge);
        }
        output.write_all(&buffer[..read]).map_err(CopyFailure::Write)?;
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
