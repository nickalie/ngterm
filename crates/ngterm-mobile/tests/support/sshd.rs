//! The e2e test's link to the machine: an unprivileged `sshd` of the test's own (its config, host key and
//! `authorized_keys` in the test's directory, listening on 127.0.0.1 only, on a free port), and a TCP proxy in front of
//! it whose connections the test can cut, to drop a client's link without touching `sshd`.
//!
//! Nothing here reads or writes the user's `~/.ssh` or the system's `sshd` configuration: `sshd -f` reads only the
//! config written here, which names absolute files for everything it would otherwise look up in the account
//! (`AuthorizedKeysFile`, no `~/.ssh/rc`, no user environment).

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// How long `sshd` gets to answer on its port.
const READY_WAIT: Duration = Duration::from_secs(15);

/// The `sshd` binary: the first on `PATH`, then the usual system directories (it must be run by an absolute path,
/// since it re-executes itself). `None` when the machine has none.
pub fn find_sshd() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(["/usr/sbin", "/usr/local/sbin", "/usr/bin", "/sbin"].map(PathBuf::from))
        .map(|directory| directory.join("sshd"))
        .find(|candidate| candidate.is_absolute() && is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

/// What the test's `sshd` is set up with.
pub struct SshdSetup<'a> {
    /// The `sshd` binary ([`find_sshd`]).
    pub binary: &'a Path,
    /// The directory its config, keys, pid file and log go to.
    pub directory: &'a Path,
    /// The account clients log in as: the test's own.
    pub user: &'a str,
    /// The OpenSSH private bytes of its host key.
    pub host_key: &'a [u8],
    /// The one `authorized_keys` line it accepts.
    pub authorized_key: &'a str,
    /// The command every login runs in place of the client's (the client's is in `SSH_ORIGINAL_COMMAND`).
    pub force_command: &'a Path,
}

/// A running `sshd`, stopped when dropped.
pub struct Sshd {
    child: Child,
    port: u16,
    log: PathBuf,
}

impl Sshd {
    /// Starts `sshd` on a free port of 127.0.0.1 and waits for its banner.
    ///
    /// # Errors
    ///
    /// Why it could not start, with its log.
    pub fn start(setup: &SshdSetup<'_>) -> Result<Self, String> {
        let host_key = setup.directory.join("host_ed25519");
        write_private(&host_key, setup.host_key)?;
        let authorized = setup.directory.join("authorized_keys");
        write_private(&authorized, format!("{}\n", setup.authorized_key).as_bytes())?;
        let mut last = String::new();
        // a free port is taken back from the kernel before sshd binds it, so another process may win it
        for _ in 0..3 {
            let port = free_port().map_err(|error| format!("no free port: {error}"))?;
            let config = setup.directory.join("sshd_config");
            let text = config_text(setup, port, &host_key, &authorized);
            fs::write(&config, text).map_err(|error| format!("writing {}: {error}", config.display()))?;
            match Self::spawn(setup, &config, port) {
                Ok(sshd) => return Ok(sshd),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    fn spawn(setup: &SshdSetup<'_>, config: &Path, port: u16) -> Result<Self, String> {
        let log = setup.directory.join("sshd.log");
        let child = Command::new(setup.binary)
            .args(["-D", "-f"])
            .arg(config)
            .arg("-E")
            .arg(&log)
            // sshd builds each login's environment itself; its own carries nothing of the test's anyway
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("starting {}: {error}", setup.binary.display()))?;
        let mut sshd = Self { child, port, log };
        sshd.wait_ready()?;
        Ok(sshd)
    }

    fn wait_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + READY_WAIT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Err(format!("sshd exited ({status}):\n{}", self.log_text()));
            }
            if banner(self.port) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        Err(format!(
            "sshd did not answer on port {}:\n{}",
            self.port,
            self.log_text()
        ))
    }

    /// The port it listens on, on 127.0.0.1.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Its log so far.
    pub fn log_text(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // our own child, by its handle; the per-connection processes end with their clients' connections
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn config_text(setup: &SshdSetup<'_>, port: u16, host_key: &Path, authorized: &Path) -> String {
    let directory = setup.directory.display();
    format!(
        "ListenAddress 127.0.0.1\n\
         Port {port}\n\
         HostKey {host_key}\n\
         PidFile {directory}/sshd.pid\n\
         AuthorizedKeysFile {authorized}\n\
         AuthenticationMethods publickey\n\
         PubkeyAuthentication yes\n\
         PasswordAuthentication no\n\
         KbdInteractiveAuthentication no\n\
         AllowUsers {user}\n\
         StrictModes no\n\
         PermitUserRC no\n\
         PermitUserEnvironment no\n\
         PermitTTY yes\n\
         AllowTcpForwarding no\n\
         AllowAgentForwarding no\n\
         X11Forwarding no\n\
         PrintMotd no\n\
         ForceCommand {force}\n\
         LogLevel VERBOSE\n",
        host_key = host_key.display(),
        authorized = authorized.display(),
        user = setup.user,
        force = setup.force_command.display(),
    )
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes)
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(0o600)))
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

fn free_port() -> io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Whether something on `port` speaks ssh.
fn banner(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut start = [0_u8; 4];
    stream.read_exact(&mut start).is_ok() && &start == b"SSH-"
}

/// A TCP proxy on 127.0.0.1 in front of `sshd`: [`Proxy::sever`] cuts every connection through it, as a lost network
/// would, while `sshd` and later connections carry on.
pub struct Proxy {
    port: u16,
    open: Arc<Mutex<Vec<TcpStream>>>,
}

impl Proxy {
    /// Forwards a free port of 127.0.0.1 to `target`'s.
    ///
    /// # Errors
    ///
    /// The bind's.
    pub fn start(target: u16) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let open = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&open);
        // the thread outlives the proxy blocked in accept; the test process ends it
        thread::spawn(move || {
            for client in listener.incoming().flatten() {
                let _ = forward(client, target, &shared);
            }
        });
        Ok(Self { port, open })
    }

    /// The port clients connect to.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Cuts every connection open now.
    pub fn sever(&self) {
        for stream in self.open.lock().unwrap_or_else(PoisonError::into_inner).drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.sever();
    }
}

fn forward(client: TcpStream, target: u16, open: &Mutex<Vec<TcpStream>>) -> io::Result<()> {
    let server = TcpStream::connect(("127.0.0.1", target))?;
    {
        let mut open = open.lock().unwrap_or_else(PoisonError::into_inner);
        open.push(client.try_clone()?);
        open.push(server.try_clone()?);
    }
    let (client_in, server_out) = (client.try_clone()?, server.try_clone()?);
    thread::spawn(move || pump(client_in, server_out));
    thread::spawn(move || pump(server, client));
    Ok(())
}

/// Copies `from` into `to` until either side ends, then ends both.
fn pump(mut from: TcpStream, mut to: TcpStream) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match from.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if to.write_all(&buffer[..count]).is_err() {
                    break;
                }
            }
        }
    }
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
}
