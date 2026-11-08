//! A local release for tests of the updater and of the app around it: a Linux installation tree in a temp
//! directory, its release archive, and a [`Server`] on 127.0.0.1 whose [`Server::source`] reads releases over plain
//! HTTP. Behind the `fakes` feature, which shipped builds never enable: they have only [`Source::github`].

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{BufRead as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest as _, Sha256};

use crate::{Error, Installation, MARKER, Source};

/// Writes the installation tree of a Linux release archive at `version` under `root`: its identity, its update
/// marker and its programs. On Linux the updater accepts it as an installation when `root` is named `ngterm`.
///
/// # Errors
/// Returns the filesystem's error.
pub fn tree(root: &Path, version: &str) -> std::io::Result<()> {
    let usr = root.join("usr");
    fs::create_dir_all(usr.join("bin"))?;
    fs::create_dir_all(usr.join("share/ngterm"))?;
    fs::write(usr.join("share/ngterm/app-BUILD"), format!("{version}\ncommit\n"))?;
    fs::write(usr.join(MARKER), "linux-archive\n")?;
    for name in ["ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"] {
        fs::write(usr.join("bin").join(name), format!("ngterm {version}"))?;
    }
    Ok(())
}

/// The release archive (`.tar.gz`) of a Linux installation [`tree`] at `version`.
///
/// # Errors
/// Returns the filesystem's error.
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

/// The asset a Linux archive installation on this machine downloads at `version`.
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
    /// plain HTTP with a short timeout.
    #[must_use]
    pub fn source(&self) -> Source {
        Source {
            api: self.url("/latest"),
            downloads: self.url("/download/"),
            agent: ureq::Agent::config_builder()
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
