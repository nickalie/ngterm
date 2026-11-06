//! The private machine of the e2e test: a staged `ngterm-headless` in a private state dir, the `ngtermctl` and
//! `ngterm-session` calls against it, and the `sshd` and proxy in front of it ([`World`]).

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_mobile::KeyPair;
use ngterm_mobile::api::generate_key;

use crate::phone::poll;
use crate::sshd::{Proxy, Sshd, SshdSetup};

/// The staged binaries (`scripts/stage.sh`): `<stage>/bin/{ngterm-headless,ngtermctl,ngterm-session}`.
pub(crate) const STAGE_VARIABLE: &str = "MOBILE_E2E_STAGE";
/// A scratch directory of the test's own, one per run: the private state dir, home, `sshd` files and logs go under it.
pub(crate) const WORK_VARIABLE: &str = "MOBILE_E2E_WORK";
/// The longest Unix socket path every Unix takes (macOS's `sun_path` holds 104 bytes with the NUL).
pub(crate) const SOCKET_PATH_MAX: usize = 103;

/// The private instance, its `sshd` and the proxy in front of it; torn down in reverse on drop.
pub(crate) struct World {
    pub(crate) proxy: Proxy,
    pub(crate) sshd: Sshd,
    pub(crate) headless: Headless,
    pub(crate) key: KeyPair,
    pub(crate) host_fingerprint: String,
    /// The run's scratch directory ([`WORK_VARIABLE`]).
    pub(crate) work: PathBuf,
}

impl World {
    pub(crate) fn start(binary: &Path) -> Self {
        let stage = PathBuf::from(std::env::var_os(STAGE_VARIABLE).expect("MOBILE_E2E_STAGE: run the script"));
        let work = PathBuf::from(std::env::var_os(WORK_VARIABLE).expect("MOBILE_E2E_WORK: run the script"));
        let user = account();
        let headless = Headless::start(&stage, &work, &user);
        let key = generate_key("ngterm-mobile e2e".to_owned()).expect("a client key");
        let host = generate_key("ngterm-mobile e2e host".to_owned()).expect("a host key");
        let force_command = headless.write_force_command(&work);
        let sshd_dir = work.join("sshd");
        fs::create_dir_all(&sshd_dir).expect("create the sshd directory");
        let sshd = Sshd::start(&SshdSetup {
            binary,
            directory: &sshd_dir,
            user: &user,
            host_key: &host.private_key,
            authorized_key: &key.authorized_key,
            force_command: &force_command,
        })
        .unwrap_or_else(|error| panic!("start sshd: {error}"));
        let proxy = Proxy::start(sshd.port()).expect("start the proxy");
        Self {
            proxy,
            sshd,
            headless,
            key,
            host_fingerprint: host.fingerprint,
            work,
        }
    }
}

/// The account the test runs as, which the `sshd` lets in.
pub(crate) fn account() -> String {
    let output = Command::new("id").arg("-un").output().expect("run id");
    String::from_utf8(output.stdout).expect("a user name").trim().to_owned()
}

/// The private environment every process of the instance runs with.
pub(crate) struct Private {
    pub(crate) stage: PathBuf,
    pub(crate) state: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) user: String,
}

impl Private {
    /// `program` with only the private environment: no `NGTERM_*` of the test's, no `XDG_*`.
    pub(crate) fn command(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .env("NGTERM_STATE_DIR", &self.state)
            .env("HOME", &self.home)
            .env("PATH", self.path())
            .env("SHELL", "/bin/bash")
            .env("USER", &self.user)
            .env("LOGNAME", &self.user)
            .env("LANG", "C.UTF-8");
        command
    }

    pub(crate) fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.stage.join("bin").display())
    }
}

/// The private `ngterm-headless` and the commands run against it.
pub(crate) struct Headless {
    pub(crate) child: Child,
    pub(crate) private: Private,
}

impl Headless {
    pub(crate) fn start(stage: &Path, work: &Path, user: &str) -> Self {
        let private = Private {
            stage: stage.to_owned(),
            state: work.join("s"),
            home: work.join("home"),
            user: user.to_owned(),
        };
        for directory in [&private.state, &private.home] {
            fs::create_dir_all(directory).expect("create a private directory");
        }
        let socket = private.state.join("ngterm.sock");
        assert!(
            socket.as_os_str().len() <= SOCKET_PATH_MAX,
            "the control socket path is too long for a Unix socket ({}); give MOBILE_E2E_WORK a shorter path",
            socket.display()
        );
        let log = fs::File::create(work.join("headless.log")).expect("create the headless log");
        let child = private
            .command(&stage.join("bin/ngterm-headless"))
            // the private home's shells stay as the test sets them up: no skill, no hooks, no rc line
            .arg("--skip-agent-setup")
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("the log"))
            .stderr(log)
            .spawn()
            .expect("start ngterm-headless");
        // for the script's exit trap, which reaps through the same namespace when the test dies without unwinding
        let namespace = ngterm_core::live::socket_directory(&private.state);
        fs::write(work.join("live-dir"), &namespace).expect("record the live sessions' namespace");
        fs::write(work.join("headless.pid"), child.id().to_string()).expect("record the instance's pid");
        let headless = Self { child, private };
        poll("the private instance's control socket", || {
            headless.ctl(&["live", "tree", "--json"]).ok()
        });
        headless
    }

    pub(crate) fn socket(&self) -> PathBuf {
        self.private.state.join("ngterm.sock")
    }

    /// `ngtermctl ARGUMENTS --socket <private socket>`: what it printed, or why it failed.
    pub(crate) fn ctl(&self, arguments: &[&str]) -> Result<String, String> {
        self.ctl_fed(arguments, None)
    }

    /// [`Headless::ctl`] with `input` on its standard input (a hook's payload), or none.
    pub(crate) fn ctl_fed(&self, arguments: &[&str], input: Option<&[u8]>) -> Result<String, String> {
        let mut child = self
            .private
            .command(&self.private.stage.join("bin/ngtermctl"))
            .args(arguments)
            .arg("--socket")
            .arg(self.socket())
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;
        if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
            // dropped at the end of the block: the command reads to its end
            stdin.write_all(input).map_err(|error| error.to_string())?;
        }
        let output = child.wait_with_output().map_err(|error| error.to_string())?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() {
            Ok(stdout)
        } else {
            Err(format!("{stdout}{}", String::from_utf8_lossy(&output.stderr)))
        }
    }

    /// The private instance's `live tree`, as (id, name) pairs.
    pub(crate) fn tree(&self) -> Vec<(String, String)> {
        let Ok(output) = self.ctl(&["live", "tree", "--json"]) else {
            return Vec::new();
        };
        let reply: serde_json::Value = serde_json::from_str(&output).expect("live tree's json");
        reply["result"]["remote"]["sessions"]
            .as_array()
            .map(|sessions| {
                sessions
                    .iter()
                    .map(|session| (text(&session["id"]), text(&session["name"])))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The `sshd`'s forced command: the private environment, then the client's command.
    pub(crate) fn write_force_command(&self, work: &Path) -> PathBuf {
        let path = work.join("force-command.sh");
        let script = format!(
            "#!/bin/sh\n\
             export NGTERM_STATE_DIR={state}\n\
             export HOME={home}\n\
             export PATH={path}\n\
             exec /bin/sh -c \"$SSH_ORIGINAL_COMMAND\"\n",
            state = quoted(&self.private.state.display().to_string()),
            home = quoted(&self.private.home.display().to_string()),
            path = quoted(&self.private.path()),
        );
        fs::write(&path, script).expect("write the forced command");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("make it executable");
        path
    }

    /// Stops the instance (its own child, by its handle) and ends every session left in its namespace through the
    /// namespace's own session server.
    pub(crate) fn reap(&mut self) {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reap_sessions();
    }

    /// Kills every session the private namespace's server holds, until the server, left empty, has gone (after its
    /// idle grace), so no process of the stage outlives the test; then removes the namespace's directory.
    fn reap_sessions(&self) {
        let namespace = ngterm_core::live::socket_directory(&self.private.state);
        let session = self.private.stage.join("bin/ngterm-session");
        let run = |arguments: &[&str]| {
            self.private
                .command(&session)
                .env("NGTERM_LIVE_DIR", &namespace)
                .args(arguments)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
        };
        // each call is a client of the server and restarts its idle grace, so the wait for it to go watches its
        // socket instead
        let socket = Path::new(&namespace).join("server.sock");
        for _ in 0..3 {
            // `list` never starts a server and lists nothing when none runs, so a failing `list` is a client error
            // (a timeout, a refused connect): try again in the next round
            let listed = match run(&["list", "--short"]) {
                Ok(listed) if listed.status.success() => String::from_utf8_lossy(&listed.stdout).into_owned(),
                _ => String::new(),
            };
            let names: Vec<&str> = listed.split_whitespace().collect();
            if !names.is_empty() {
                let _ = run(&[["kill", "--force"].as_slice(), &names].concat());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if !socket.exists() {
                    // the namespace is this run's alone (hashed from its private state dir): nothing else uses it
                    let _ = fs::remove_dir_all(&namespace);
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
        eprintln!("reap: the session server of {namespace} is still running; it and the directory are left behind");
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        self.reap();
    }
}

pub(crate) fn text(value: &serde_json::Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

pub(crate) fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
