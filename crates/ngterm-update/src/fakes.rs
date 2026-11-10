//! A local release for tests of the updater and of the app around it: an installation tree in a temp directory (an
//! `ngterm.app` bundle on macOS, a Linux archive's tree elsewhere), its release archive, and a [`Server`] on 127.0.0.1
//! whose [`Server::source`] reads releases over plain HTTP. Behind the `fakes` feature, which shipped builds never
//! enable: they have only [`Source::github`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest as _, Sha256};

#[cfg(not(target_os = "macos"))]
use crate::MARKER;
use crate::{Error, Installation, Source};

/// The bundle name a release archive carries; an installed bundle only needs the `.app` extension.
#[cfg(target_os = "macos")]
pub const ROOT: &str = "ngterm.app";
/// What the updater requires an installation's root directory to be called on this platform.
#[cfg(not(target_os = "macos"))]
pub const ROOT: &str = "ngterm";

#[cfg(target_os = "macos")]
const EXECUTABLE: &str = "Contents/MacOS/ngterm";
#[cfg(not(target_os = "macos"))]
const EXECUTABLE: &str = "usr/bin/ngterm";

#[cfg(target_os = "macos")]
const IDENTITY: &str = "Contents/share/ngterm/app-BUILD";
#[cfg(not(target_os = "macos"))]
const IDENTITY: &str = "usr/share/ngterm/app-BUILD";

/// The main program of the installation [`tree`] at `root`.
#[must_use]
pub fn executable(root: &Path) -> PathBuf {
    root.join(EXECUTABLE)
}

/// The identity file of the installation [`tree`] at `root`.
#[must_use]
pub fn identity(root: &Path) -> PathBuf {
    root.join(IDENTITY)
}

/// Writes the unsigned tree of a macOS app bundle at `version` under `root`: its identity, its update marker and its
/// programs. The updater accepts it as an installation when `root` has the `.app` extension; [`seal`] signs it.
///
/// # Errors
/// Returns the filesystem's error.
#[cfg(target_os = "macos")]
pub fn tree(root: &Path, version: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let contents = root.join("Contents");
    let programs = contents.join("MacOS");
    fs::create_dir_all(&programs)?;
    fs::create_dir_all(contents.join("Resources"))?;
    fs::create_dir_all(contents.join("share/ngterm"))?;
    fs::write(contents.join("Info.plist"), INFO_PLIST)?;
    fs::write(contents.join("share/ngterm/app-BUILD"), format!("{version}\ncommit\n"))?;
    fs::write(contents.join("Resources/update-format"), "macos-bundle\n")?;
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"] {
        // codesign seals a bundle around real Mach-O programs, whose signatures survive the zip.
        let program = programs.join(name);
        fs::copy("/usr/bin/true", &program)?;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>ngterm</string>
<key>CFBundleIdentifier</key><string>com.ngterm.update-fixture</string>
</dict></plist>
"#;

/// Writes the installation tree of a Linux release archive at `version` under `root`: its identity, its update
/// marker and its programs. On Linux the updater accepts it as an installation when `root` is named `ngterm`.
///
/// # Errors
/// Returns the filesystem's error.
#[cfg(not(target_os = "macos"))]
pub fn tree(root: &Path, version: &str) -> std::io::Result<()> {
    let usr = root.join("usr");
    fs::create_dir_all(usr.join("bin"))?;
    fs::create_dir_all(usr.join("share/ngterm"))?;
    fs::write(usr.join("share/ngterm/app-BUILD"), format!("{version}\ncommit\n"))?;
    fs::write(usr.join(MARKER), "linux-archive\n")?;
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless", "ngterm-web"] {
        fs::write(usr.join("bin").join(name), format!("ngterm {version}"))?;
    }
    Ok(())
}

/// The release archive (`.app.zip`) of an ad-hoc signed macOS bundle [`tree`] at `version`.
///
/// # Errors
/// Returns the filesystem's error, or `codesign`'s or `ditto`'s.
#[cfg(target_os = "macos")]
pub fn archive(version: &str) -> std::io::Result<Vec<u8>> {
    let source = tempfile::tempdir()?;
    let root = source.path().join(ROOT);
    tree(&root, version)?;
    seal(&root)?;
    zip(&root)
}

/// Signs `bundle` ad hoc, as a release's signing does with its identity: `codesign --verify` then holds every file
/// under it to what it is now.
///
/// # Errors
/// Returns the failure of `codesign`, with its standard error.
#[cfg(target_os = "macos")]
pub fn seal(bundle: &Path) -> std::io::Result<()> {
    tool(
        Command::new("/usr/bin/codesign")
            .args(["--force", "--deep", "--sign", "-"])
            .arg(bundle),
    )
}

/// `bundle` as a release ships it: a `ditto` zip with the bundle itself at its top.
///
/// # Errors
/// Returns the filesystem's error, or the failure of `ditto` with its standard error.
#[cfg(target_os = "macos")]
pub fn zip(bundle: &Path) -> std::io::Result<Vec<u8>> {
    let dir = tempfile::tempdir()?;
    let zip = dir.path().join("release.zip");
    tool(
        Command::new("/usr/bin/ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(bundle)
            .arg(&zip),
    )?;
    fs::read(zip)
}

#[cfg(target_os = "macos")]
fn tool(command: &mut Command) -> std::io::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(std::io::Error::other(
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

/// The release archive (`.tar.gz`) of a Linux installation [`tree`] at `version`.
///
/// # Errors
/// Returns the filesystem's error.
#[cfg(not(target_os = "macos"))]
pub fn archive(version: &str) -> std::io::Result<Vec<u8>> {
    let source = tempfile::tempdir()?;
    let root = source.path().join("ngterm");
    tree(&root, version)?;
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    archive.append_dir_all("ngterm", &root)?;
    archive.into_inner()?.finish()
}

/// The installation whose executable is `executable`, refused as [`Installation::detect`] refuses the running one.
///
/// # Errors
/// Returns the refusal: no release marker, an invalid layout or an inaccessible installation.
pub fn installation(executable: &Path) -> Result<Installation, Error> {
    Installation::at(executable)
}

/// `installation` with `program` run in place of the system's password popup.
#[must_use]
pub fn prompting(mut installation: Installation, program: &Path) -> Installation {
    installation.prompter = Some(program.to_owned());
    installation
}

/// The asset a `.deb` installation on this machine downloads at `version`.
#[must_use]
pub fn deb_asset(version: &str) -> String {
    let arch = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "amd64"
    };
    format!("ngterm_{version}-1_{arch}.deb")
}

/// The asset a macOS bundle installation on this machine downloads at `version`.
#[cfg(target_os = "macos")]
#[must_use]
pub fn asset(version: &str) -> String {
    format!(
        "ngterm-{version}-macos-{}.app.zip",
        std::env::consts::ARCH.replace("aarch64", "arm64")
    )
}

/// The asset a Linux archive installation on this machine downloads at `version`.
#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn asset(version: &str) -> String {
    format!("ngterm-{version}-linux-{}.tar.gz", std::env::consts::ARCH)
}

/// A `SHA256SUMS` manifest of `files`.
#[must_use]
pub fn manifest(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut manifest = String::new();
    for (name, bytes) in files {
        for byte in Sha256::digest(bytes) {
            let _ = write!(manifest, "{byte:02x}");
        }
        let _ = writeln!(manifest, "  {name}");
    }
    manifest.into_bytes()
}

/// A route's answer: its status line's status (`200 OK`) and its body.
type Answer = (String, Vec<u8>);

/// An HTTP/1.1 server on 127.0.0.1 that answers every request from its routes, and any other path with
/// `404 Not Found`, until it is dropped. It records the path of each request it answered.
pub struct Server {
    base: String,
    routes: Arc<Mutex<HashMap<String, Answer>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server with no routes on a free port.
    ///
    /// # Errors
    /// Returns the socket's error.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        // Nonblocking, so dropping the server stops it between requests.
        listener.set_nonblocking(true)?;
        let base = format!("http://{}", listener.local_addr()?);
        let routes = Arc::default();
        let requests = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let (routes, requests, stop) = (Arc::clone(&routes), Arc::clone(&requests), Arc::clone(&stop));
            move || serve(&listener, &routes, &requests, &stop)
        });
        Ok(Self {
            base,
            routes,
            requests,
            stop,
            thread: Some(thread),
        })
    }

    /// The URL of `path` (`/latest`) on this server.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Where releases come from on this server: the latest at `/latest`, downloads under `/download/`, read over
    /// plain HTTP with a short timeout and no environment proxy.
    #[must_use]
    pub fn source(&self) -> Source {
        Source {
            api: self.url("/latest"),
            downloads: self.url("/download/"),
            agent: ureq::Agent::config_builder()
                .proxy(None)
                .timeout_global(Some(Duration::from_secs(10)))
                .build()
                .into(),
        }
    }

    /// Answers `path` with `status` and `body` from now on.
    pub fn route(&self, path: &str, status: &str, body: impl Into<Vec<u8>>) {
        lock(&self.routes).insert(path.to_owned(), (status.to_owned(), body.into()));
    }

    /// Publishes the latest release at `version` with `files` as its assets, each under
    /// `/download/v<version>/<name>` with its exact size.
    pub fn publish(&self, version: &str, files: &[(&str, &[u8])]) {
        let assets: Vec<_> = files
            .iter()
            .map(|(name, bytes)| {
                let path = format!("/download/v{version}/{name}");
                self.route(&path, "200 OK", *bytes);
                serde_json::json!({ "name": name, "browser_download_url": self.url(&path), "size": bytes.len() })
            })
            .collect();
        let latest = serde_json::json!({
            "tag_name": format!("v{version}"),
            "draft": false,
            "prerelease": false,
            "assets": assets,
        });
        self.route("/latest", "200 OK", latest.to_string());
    }

    /// Publishes the latest release at `version` as GitHub carries it with no signing key: `archive` as this
    /// machine's [`asset`] and its `SHA256SUMS`.
    pub fn publish_archive(&self, version: &str, archive: &[u8]) {
        let asset = asset(version);
        let sums = manifest(&[(&asset, archive)]);
        self.publish(version, &[(&asset, archive), ("SHA256SUMS", &sums)]);
    }

    /// The path of each request answered so far, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        lock(&self.requests).clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A test that panicked while holding a lock must not take the server down with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(
    listener: &TcpListener,
    routes: &Mutex<HashMap<String, Answer>>,
    requests: &Mutex<Vec<String>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            // A client that hangs up mid-request only loses its own answer.
            Ok((stream, _)) => drop(answer(&stream, routes, requests)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return,
        }
    }
}

fn answer(
    mut stream: &TcpStream,
    routes: &Mutex<HashMap<String, Answer>>,
    requests: &Mutex<Vec<String>>,
) -> std::io::Result<()> {
    // macOS and Windows hand the listener's nonblocking mode on to the accepted socket.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = std::io::BufReader::new(stream);
    let mut line = String::new();
    request.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or_default().to_owned();
    loop {
        let mut header = String::new();
        if request.read_line(&mut header)? == 0 || header == "\r\n" {
            break;
        }
    }
    lock(requests).push(path.clone());
    let (status, body) = lock(routes)
        .get(&path)
        .cloned()
        .unwrap_or_else(|| ("404 Not Found".into(), Vec::new()));
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    stream.write_all(&response)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "fakes_tests.rs"]
mod tests;
