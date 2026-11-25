//! Process boundaries: every run has its own home, state and control socket. The session executable is an inert
//! program, so exercising launch and quit cannot start a live daemon. On Windows a server runs on a pseudo console
//! ([`Running`]): a console's Ctrl events reach only the processes on it, so that is where its Ctrl+C is pressed.

use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use ngterm_server::endpoint::{self, Connection};

mod support;

struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
}

impl Fixture {
    fn new(staged: bool) -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let source = support::binary();
        // Windows' live probe wants the bundled PowerShell integration, which only a staged build finds
        let binary = if staged || cfg!(windows) {
            let bin = root.path().join("stage/bin");
            std::fs::create_dir_all(&bin)?;
            let target = bin.join(format!("ngterm-headless{}", std::env::consts::EXE_SUFFIX));
            ngterm_host::fakes::copy_executable(&source, &target)?;
            let share = root.path().join("stage/share");
            std::fs::create_dir_all(&share)?;
            let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
            #[cfg(unix)]
            std::os::unix::fs::symlink(resources, share.join("ngterm"))?;
            #[cfg(windows)]
            copy_tree(&resources, &share.join("ngterm"))?;
            target
        } else {
            source
        };
        for path in [
            "home/.claude",
            "home/.codex",
            "ghostty/shell-integration/bash",
            "ghostty/shell-integration/zsh",
            "ghostty/shell-integration/fish/vendor_conf.d",
        ] {
            std::fs::create_dir_all(root.path().join(path))?;
        }
        for file in [
            "bash/ghostty.bash",
            "zsh/.zshenv",
            "fish/vendor_conf.d/ghostty-shell-integration.fish",
        ] {
            std::fs::write(root.path().join("ghostty/shell-integration").join(file), "")?;
        }
        Ok(Self { root, binary })
    }

    /// The environment every run gets, nothing inherited but what the OS's own libraries need.
    fn environment(&self) -> Vec<(OsString, OsString)> {
        let home = self.root.path().join("home");
        let mut environment: Vec<(OsString, OsString)> = vec![
            ("HOME".into(), home.clone().into()),
            ("USERPROFILE".into(), home.clone().into()),
            ("XDG_CONFIG_HOME".into(), home.join(".config").into()),
            ("NGTERM_STATE_DIR".into(), self.root.path().join("state").into()),
            ("NGTERM_CONTROL_SOCKET".into(), self.socket().into()),
            ("NGTERM_SESSION_PATH".into(), inert_program().into()),
            ("GHOSTTY_RESOURCES_DIR".into(), self.root.path().join("ghostty").into()),
        ];
        if cfg!(windows) {
            for name in ["SystemRoot", "SystemDrive", "TEMP", "TMP", "windir", "PATH", "ComSpec"] {
                environment.extend(std::env::var_os(name).map(|value| (name.into(), value)));
            }
        } else {
            environment.push(("PATH".into(), "/usr/bin:/bin".into()));
            environment.push(("SHELL".into(), "/bin/sh".into()));
        }
        // Child profiles belong to the same measurement as this test's profile.
        environment.extend(std::env::var_os("LLVM_PROFILE_FILE").map(|profile| ("LLVM_PROFILE_FILE".into(), profile)));
        environment
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .env_clear()
            .envs(self.environment())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command
    }

    /// A server started with `args` and these variables added.
    fn start(&self, args: &[&str], extra: &[(&str, &Path)]) -> io::Result<Running> {
        Running::start(self, args, extra)
    }

    fn socket(&self) -> PathBuf {
        self.root.path().join("control.sock")
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.root.path().join("state/ngterm.log")).unwrap_or_default()
    }
}

/// A program that exits at once, unsuccessfully: `false` is `/bin/false` on Linux, `/usr/bin/false` alone on macOS.
#[cfg(unix)]
fn inert_program() -> &'static str {
    ["/bin/false", "/usr/bin/false"]
        .into_iter()
        .find(|program| Path::new(program).exists())
        .unwrap_or("/usr/bin/false")
}

/// A program that exits at once, unsuccessfully: `where` asked for nothing.
#[cfg(windows)]
fn inert_program() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_default()).join(r"System32\where.exe")
}

#[cfg(windows)]
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
struct Running(std::process::Child);

#[cfg(unix)]
impl Running {
    fn start(fixture: &Fixture, args: &[&str], extra: &[(&str, &Path)]) -> io::Result<Self> {
        let mut command = fixture.command();
        command.args(args).envs(extra.iter().copied());
        Ok(Self(command.spawn()?))
    }

    fn has_exited(&mut self) -> io::Result<bool> {
        Ok(self.0.try_wait()?.is_some())
    }

    fn exit_succeeded(&mut self) -> io::Result<Option<bool>> {
        Ok(self.0.try_wait()?.map(|status| status.success()))
    }

    /// What ends a server the way the system does: SIGTERM.
    fn terminate(&self) -> io::Result<()> {
        let pid = i32::try_from(self.0.id()).map_err(io::Error::other)?;
        // SAFETY: the signal targets only this test's private child.
        if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(unix)]
impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

/// A server on a pseudo console of its own, with its output read and thrown away.
#[cfg(windows)]
struct Running {
    child: ngterm_pty::Child,
    actor: ngterm_pty::ActorHandle,
}

#[cfg(windows)]
impl Running {
    fn start(fixture: &Fixture, args: &[&str], extra: &[(&str, &Path)]) -> io::Result<Self> {
        let mut command = ngterm_pty::Command::new(&fixture.binary).env_clear().args(args);
        for (name, value) in fixture.environment() {
            command = command.env(name, value);
        }
        for (name, value) in extra {
            command = command.env(name, value);
        }
        let ngterm_pty::Spawned { master, child } = ngterm_pty::spawn(&command, ngterm_pty::Size::cells(24, 80))?;
        // a pseudo console holds its program back until the terminal answers its query of where the cursor is
        let actor = ngterm_pty::Actor::spawn(ngterm_pty::ActorConfig::new(
            "lifecycle",
            master,
            Box::new(|read| {
                if read.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                    vec![b"\x1b[1;1R".to_vec()]
                } else {
                    Vec::new()
                }
            }),
        ))?;
        Ok(Self { child, actor })
    }

    fn has_exited(&mut self) -> io::Result<bool> {
        Ok(self.child.try_wait()?.is_some())
    }

    fn exit_succeeded(&mut self) -> io::Result<Option<bool>> {
        Ok(self.child.try_wait()?.map(|status| status.success()))
    }

    /// What ends a server the way the system does: Ctrl+C on its console.
    fn terminate(&self) -> io::Result<()> {
        self.actor
            .write_input(b"\x03")
            .map_err(|_| io::Error::other("the console is gone"))
    }
}

#[cfg(windows)]
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.killer().kill_tree();
        self.actor.shutdown();
    }
}

/// Whether the server exited successfully, within a bound.
fn wait_exit(running: &mut Running) -> io::Result<bool> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(succeeded) = running.exit_succeeded()? {
            return Ok(succeeded);
        }
        assert!(Instant::now() < deadline, "child did not exit");
        std::thread::yield_now();
    }
}

fn ready(fixture: &Fixture, child: &mut Running) -> io::Result<Connection> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(stream) = endpoint::connect(&fixture.socket()) {
            first_pane_settled(fixture);
            return Ok(stream);
        }
        assert!(!child.has_exited()?, "headless exited before serving");
        assert!(
            Instant::now() < deadline,
            "control socket did not open: {}",
            fixture.log()
        );
        std::thread::yield_now();
    }
}

/// On Linux, where the headless e2e coverage is measured (scripts/test-mobile-e2e.sh --coverage): waits until the
/// first pane of a launch that runs live has failed its attach over the inert session host (the log says both), so
/// every run of a test quits after that failure, never before it, and an instrumented build's coverage does not vary
/// with how soon a test ends. Nothing for a launch that does not run live.
fn first_pane_settled(fixture: &Fixture) {
    if !cfg!(target_os = "linux") {
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let log = fixture.log();
        if !log.contains("this launch runs live") || log.contains("a live pane's attach failed") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the first live pane never failed its attach: {log}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn control(fixture: &Fixture, request: &serde_json::Value) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut stream = endpoint::connect(&fixture.socket())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

/// A new session named `name` running the login shell (a headless origin refuses `--wait`): its id.
fn new_session(fixture: &Fixture, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let created = control(fixture, &serde_json::json!({"cmd":"session.new", "args":{"name":name}}))?;
    assert_eq!(created["ok"], true, "{created}");
    Ok(created["result"]["id"].as_str().ok_or("no session id")?.to_owned())
}

/// The live session namespace of the fixture's server, as the server derives it from its state directory and home.
fn live_namespace(fixture: &Fixture) -> PathBuf {
    let state = fixture.root.path().join("state");
    #[cfg(unix)]
    let directory = ngterm_core::live::socket_directory(&state);
    #[cfg(windows)]
    let directory = ngterm_core::live::windows_socket_directory(
        &ngterm_core::paths::MapEnv::new().with("USERPROFILE", fixture.root.path().join("home")),
        &ngterm_core::live::state_dir_hash(&state),
    );
    PathBuf::from(directory)
}

/// Holds the server's live session endpoint with a listener that accepts and never answers, so each later live pane
/// waits in its attach and keeps its session; with the inert session host, an attach fails, and its session closes, at
/// once.
struct LiveHold {
    attaches: std::sync::mpsc::Receiver<ngterm_session::endpoint::Stream>,
    held: Vec<ngterm_session::endpoint::Stream>,
    #[cfg(unix)]
    namespace: PathBuf,
}

fn hold_live_attaches(fixture: &Fixture) -> io::Result<LiveHold> {
    let namespace = live_namespace(fixture);
    let listener = ngterm_session::endpoint::Endpoint::new(&namespace).bind()?;
    let (sender, attaches) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(stream) = listener.accept() {
            if sender.send(stream).is_err() {
                return;
            }
        }
    });
    Ok(LiveHold {
        attaches,
        held: Vec::new(),
        #[cfg(unix)]
        namespace,
    })
}

impl LiveHold {
    /// Waits until `count` attaches came to the hold, so a test knows they wait on it, whatever the runner's speed.
    fn wait_for(&mut self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.held.len() < count {
            let left = deadline.saturating_duration_since(Instant::now());
            let stream = self.attaches.recv_timeout(left);
            assert!(
                stream.is_ok(),
                "{} of {count} attaches came to the hold",
                self.held.len()
            );
            self.held.extend(stream);
        }
    }
}

impl Drop for LiveHold {
    fn drop(&mut self) {
        self.held.clear();
        #[cfg(unix)]
        let _ = std::fs::remove_dir_all(&self.namespace);
    }
}

/// The windows the state directory holds, as saved.
fn saved_windows(fixture: &Fixture) -> Result<Vec<serde_json::Value>, Box<dyn std::error::Error>> {
    let mut windows = Vec::new();
    for entry in std::fs::read_dir(fixture.root.path().join("state/windows"))? {
        let file = entry?.path();
        if file.extension().is_some_and(|extension| extension == "json") {
            windows.push(serde_json::from_str(&std::fs::read_to_string(file)?)?);
        }
    }
    Ok(windows)
}

#[test]
fn a_server_answers_its_private_socket_and_saves_on_termination() {
    let fixture = Fixture::new(false).unwrap();
    let mut child = fixture.start(&["--skip-agent-setup"], &[]).unwrap();
    let mut stream = ready(&fixture, &mut child).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(b"{\"cmd\":\"version\"}\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    let mut hold = hold_live_attaches(&fixture).unwrap();
    let probe = new_session(&fixture, "quit-save-probe").unwrap();
    let closed = control(
        &fixture,
        &serde_json::json!({"cmd":"session.close", "target":probe, "args":{"undoable":true}}),
    )
    .unwrap();
    assert_eq!(closed["ok"], true, "{closed}");
    let second = fixture.command().arg("--skip-agent-setup").output().unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another instance is already running on"));
    // each new session is written at once, and the last write never had `first` selected; a selection is written
    // after a debounce, which the signal beats, so only the quit save writes it
    let first = new_session(&fixture, "selection-probe-first").unwrap();
    new_session(&fixture, "selection-probe-second").unwrap();
    hold.wait_for(2);
    // longer than an unheld attach takes to fail, so the select shows that the hold keeps `first`
    std::thread::sleep(Duration::from_millis(200));
    let selected = control(&fixture, &serde_json::json!({"cmd":"session.select", "target":first})).unwrap();
    assert_eq!(selected["ok"], true, "{selected}");
    child.terminate().unwrap();
    assert!(wait_exit(&mut child).unwrap());
    drop(hold);
    assert!(fixture.log().contains("quit: the state is saved"));
    assert!(fixture.root.path().join("state/windows.json").is_file());
    let windows = saved_windows(&fixture).unwrap();
    let saved = serde_json::to_string(&windows).unwrap();
    assert!(
        !saved.contains("quit-save-probe"),
        "the pending close was not made final: {saved}"
    );
    assert!(
        windows.iter().any(|window| window["selectedSessionID"]
            .as_str()
            .is_some_and(|id| id.eq_ignore_ascii_case(&first))),
        "the last selection was not saved: {saved}"
    );
}

fn install(fixture: &Fixture) -> io::Result<Output> {
    fixture
        .command()
        .args(["--install-agent-hooks", "--opencode-version", "v2"])
        .stdout(Stdio::piped())
        .output()
}

#[test]
fn installers_report_success_and_errors_in_a_private_home() {
    let fixture = Fixture::new(false).unwrap();
    let output = install(&fixture).unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Agent Skill Installed"));
    assert!(fixture.root.path().join("home/.codex/skills/ngterm/SKILL.md").is_file());
    let hooks = fixture.root.path().join("home/.config/ngterm");
    std::fs::remove_dir_all(&hooks).unwrap();
    std::fs::write(hooks, "blocked").unwrap();
    let output = install(&fixture).unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Agent Skill Installed"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("ngterm-headless:"));
}

#[test]
fn a_staged_launch_installs_agents_and_keeps_serving_after_an_install_failure() {
    for blocked in [false, true] {
        let fixture = Fixture::new(true).unwrap();
        if blocked {
            let config = fixture.root.path().join("home/.config");
            std::fs::create_dir_all(&config).unwrap();
            std::fs::write(config.join("ngterm"), "blocked").unwrap();
        }
        let mut child = fixture.start(&["--opencode-version", "v2"], &[]).unwrap();
        drop(ready(&fixture, &mut child).unwrap());
        assert!(
            fixture
                .root
                .path()
                .join("home/.claude/skills/ngterm/SKILL.md")
                .is_file()
        );
        // the agents are set up before the server starts, so the outcome is in the log by now
        let log = fixture.log();
        assert_eq!(
            log.contains("the agent status hooks were not installed"),
            blocked,
            "{log}"
        );
        child.terminate().unwrap();
        assert!(wait_exit(&mut child).unwrap());
    }
}

#[test]
fn a_refused_control_socket_saves_on_quit_and_exits_unsuccessfully() {
    let fixture = Fixture::new(false).unwrap();
    let lock_path = ngterm_core::paths::ownership_lock_path(&fixture.socket());
    let lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
        .unwrap();
    lock.try_lock().unwrap();
    let mut child = fixture.start(&["--skip-agent-setup"], &[]).unwrap();
    let log = fixture.root.path().join("state/ngterm.log");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("the control server is not serving")
        {
            break;
        }
        assert!(!child.has_exited().unwrap(), "server exited before the control refusal");
        assert!(
            Instant::now() < deadline,
            "control refusal not logged: {}",
            fixture.log()
        );
        std::thread::yield_now();
    }
    first_pane_settled(&fixture);
    // the launch wrote the index; nothing changes after it, so only the quit save writes it again
    let index = fixture.root.path().join("state/windows.json");
    std::fs::remove_file(&index).unwrap();
    child.terminate().unwrap();
    assert!(!wait_exit(&mut child).unwrap());
    assert!(fixture.log().contains("quit: the state is saved"));
    assert!(index.is_file());
}

/// Stops a server only its pid is known of, when the test ends: on Unix as the system does, SIGTERM, so it quits and an
/// instrumented build writes its coverage profile; SIGKILL only if it is still there after [`DETACHED_QUIT`].
struct DetachedServer(u32);

/// How long a detached server is given to quit on SIGTERM.
#[cfg(unix)]
const DETACHED_QUIT: Duration = Duration::from_secs(10);

/// Whether the process `pid` still runs: there, and on Linux not a zombie its new parent has yet to reap.
#[cfg(unix)]
fn runs(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    let zombie = std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(')')
            .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z'))
    });
    exists && !zombie
}

impl Drop for DetachedServer {
    fn drop(&mut self) {
        #[cfg(windows)]
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &self.0.to_string()])
            .output();
        #[cfg(unix)]
        if let Ok(pid) = i32::try_from(self.0) {
            // SAFETY: the signal targets only this test's own server.
            unsafe { libc::kill(pid, libc::SIGTERM) };
            let deadline = Instant::now() + DETACHED_QUIT;
            while runs(pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if runs(pid) {
                // SAFETY: as above.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}

#[test]
fn a_detached_start_returns_and_the_server_keeps_serving() {
    let fixture = Fixture::new(false).unwrap();
    let mut command = fixture.command();
    let output = command
        .args(["--skip-agent-setup", "--detach"])
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    let pid = said
        .split("(pid ")
        .nth(1)
        .and_then(|rest| rest.trim_end().strip_suffix(')'))
        .and_then(|pid| pid.parse().ok())
        .unwrap_or_else(|| panic!("no pid in {said:?}"));
    let _server = DetachedServer(pid);
    // the command that ran is gone and the server answers on its own
    let deadline = Instant::now() + Duration::from_secs(20);
    let response = loop {
        if let Ok(response) = control(&fixture, &serde_json::json!({"cmd":"version"})) {
            break response;
        }
        assert!(
            Instant::now() < deadline,
            "control socket did not open: {}",
            fixture.log()
        );
        std::thread::yield_now();
    };
    assert_eq!(response["ok"], true, "{response}");
    first_pane_settled(&fixture);
}

#[test]
fn unknown_opencode_reports_the_skipped_hooks_and_preserves_user_owned_skills() {
    let fixture = Fixture::new(true).unwrap();
    let config = fixture.root.path().join("home/.config/opencode");
    std::fs::create_dir_all(&config).unwrap();
    let skill = fixture.root.path().join("home/.claude/skills/ngterm");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "user owned skill").unwrap();
    // no login shell to probe: the host's own OpenCode and startup files stay out of it
    let shell = fixture.root.path().join("missing-shell");
    let output = fixture
        .command()
        .env("SHELL", &shell)
        .arg("--install-agent-hooks")
        .stdout(Stdio::piped())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "OpenCode version could not be detected; skipping its hooks. Pass --opencode-version v1 or v2 to choose it."
    ));
    assert_eq!(
        std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
        "user owned skill"
    );
    let mut child = fixture.start(&[], &[("SHELL", shell.as_path())]).unwrap();
    drop(ready(&fixture, &mut child).unwrap());
    let log = fixture.log();
    assert!(
        log.contains("OpenCode version could not be detected; skipping its hooks."),
        "{log}"
    );
    child.terminate().unwrap();
    assert!(wait_exit(&mut child).unwrap());
}

#[test]
fn a_missing_session_host_refuses_launch_with_the_live_probe_reason() {
    let fixture = Fixture::new(false).unwrap();
    let output = fixture
        .command()
        .arg("--skip-agent-setup")
        .env("NGTERM_SESSION_PATH", fixture.root.path().join("absent-session"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(
            "ngterm-headless: live sessions are unavailable here: the live-session executable is unavailable"
        ),
        "{output:?}"
    );
    assert!(!fixture.socket().exists());
}

/// The catalog stream's hello and snapshot: the epoch and the catalog id.
fn catalog_hello(fixture: &Fixture) -> Result<(String, String), Box<dyn std::error::Error>> {
    let mut stream = endpoint::connect(&fixture.socket())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.write_all(b"{\"cmd\":\"catalog.subscribe\"}\n")?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut frame = || -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    };
    let reply = frame()?;
    assert_eq!(reply["ok"], true, "{reply}");
    stream.write_all(b"{\"kind\":\"hello\",\"hello\":{\"version\":1}}\n")?;
    let hello = frame()?;
    let snapshot = frame()?;
    assert_eq!(snapshot["kind"], "snapshot", "{snapshot}");
    let epoch = hello["hello"]["epoch"].as_str().ok_or("no epoch")?.to_owned();
    let catalog_id = snapshot["snapshot"]["catalogID"]
        .as_str()
        .ok_or("no catalog id")?
        .to_owned();
    Ok((epoch, catalog_id))
}

fn session_ids(fixture: &Fixture) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let tree = control(fixture, &serde_json::json!({"cmd":"tree"}))?;
    let workspaces = tree["result"]["tree"]["workspaces"].as_array().ok_or("no workspaces")?;
    Ok(workspaces
        .iter()
        .flat_map(|workspace| workspace["sessions"].as_array().into_iter().flatten())
        .filter_map(|session| session["id"].as_str().map(str::to_owned))
        .collect())
}

#[test]
fn the_catalog_id_and_its_receipts_outlive_a_restart() {
    let fixture = Fixture::new(false).unwrap();
    let mut child = fixture.start(&["--skip-agent-setup"], &[]).unwrap();
    drop(ready(&fixture, &mut child).unwrap());
    let (epoch, catalog_id) = catalog_hello(&fixture).unwrap();
    let created = control(
        &fixture,
        &serde_json::json!({"cmd":"session.new", "args":{"opID":"op-restart", "epoch":epoch}}),
    )
    .unwrap();
    assert_eq!(created["result"]["receipt"]["outcome"], "applied", "{created}");
    child.terminate().unwrap();
    assert!(wait_exit(&mut child).unwrap());
    let saved = std::fs::read_to_string(fixture.root.path().join("state/catalog-id")).unwrap();
    assert!(
        saved.trim().eq_ignore_ascii_case(&catalog_id),
        "{saved} != {catalog_id}"
    );

    let mut child = fixture.start(&["--skip-agent-setup"], &[]).unwrap();
    drop(ready(&fixture, &mut child).unwrap());
    let (restarted, same_id) = catalog_hello(&fixture).unwrap();
    assert_ne!(restarted, epoch);
    assert_eq!(same_id, catalog_id);
    let before = session_ids(&fixture).unwrap();
    // an applied receipt moves to the new epoch at rev 0, so a repeat sent at either epoch runs nothing
    let applied = serde_json::json!({"receipt":{"opID":"op-restart", "epoch":restarted, "outcome":"applied", "rev":0}});
    for sent_at in [&epoch, &restarted] {
        let repeated = control(
            &fixture,
            &serde_json::json!({"cmd":"session.new", "args":{"opID":"op-restart", "epoch":sent_at}}),
        )
        .unwrap();
        assert_eq!(repeated, serde_json::json!({"ok":true, "result":applied}));
    }
    let looked = control(
        &fixture,
        &serde_json::json!({"cmd":"catalog.op", "target":"op-restart"}),
    )
    .unwrap();
    assert_eq!(looked, serde_json::json!({"ok":true, "result":applied}));
    let after = session_ids(&fixture).unwrap();
    assert!(after.iter().all(|id| before.contains(id)), "{before:?} -> {after:?}");
    child.terminate().unwrap();
    assert!(wait_exit(&mut child).unwrap());
}

#[test]
fn a_start_refuses_a_malformed_catalog_id_and_leaves_it_alone() {
    let fixture = Fixture::new(false).unwrap();
    let file = fixture.root.path().join("state").join("catalog-id");
    std::fs::create_dir_all(fixture.root.path().join("state")).unwrap();
    std::fs::write(&file, "not-a-uuid\n").unwrap();
    let output = fixture.command().arg("--skip-agent-setup").output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(&format!(
            "ngterm-headless: catalog id is not a UUID: {}",
            file.display()
        )),
        "{output:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "not-a-uuid\n");
    assert!(endpoint::connect(&fixture.socket()).is_err());
}
