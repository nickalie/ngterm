//! Process boundaries: every run has its own home, state and control socket. The session executable is an inert
//! program, so exercising launch and quit cannot start a live daemon.
#![cfg(unix)]

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Fixture {
    root: tempfile::TempDir,
    binary: PathBuf,
}

impl Fixture {
    fn new(staged: bool) -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let source = Path::new(env!("CARGO_BIN_EXE_ngterm-headless"));
        let binary = if staged {
            let bin = root.path().join("stage/bin");
            std::fs::create_dir_all(&bin)?;
            let target = bin.join("ngterm-headless");
            std::fs::copy(source, &target)?;
            let share = root.path().join("stage/share");
            std::fs::create_dir_all(&share)?;
            std::os::unix::fs::symlink(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources"),
                share.join("ngterm"),
            )?;
            target
        } else {
            source.to_path_buf()
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

    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        let home = self.root.path().join("home");
        command
            .env_clear()
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("NGTERM_STATE_DIR", self.root.path().join("state"))
            .env("NGTERM_CONTROL_SOCKET", self.socket())
            .env("NGTERM_SESSION_PATH", "/bin/false")
            .env("GHOSTTY_RESOURCES_DIR", self.root.path().join("ghostty"))
            .env("PATH", "/usr/bin:/bin")
            .env("SHELL", "/bin/sh")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        // Child profiles belong to the same measurement as this test's profile.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    fn socket(&self) -> PathBuf {
        self.root.path().join("control.sock")
    }
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn wait_exit(child: &mut Child) -> io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        assert!(Instant::now() < deadline, "child did not exit");
        std::thread::yield_now();
    }
}

fn ready(fixture: &Fixture, child: &mut Child) -> io::Result<UnixStream> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(stream) = UnixStream::connect(fixture.socket()) {
            return Ok(stream);
        }
        assert!(child.try_wait()?.is_none(), "headless exited before serving");
        assert!(Instant::now() < deadline, "control socket did not open");
        std::thread::yield_now();
    }
}

fn control(fixture: &Fixture, request: &serde_json::Value) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(fixture.socket())?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut bytes = serde_json::to_vec(request)?;
    bytes.push(b'\n');
    stream.write_all(&bytes)?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

fn terminate(child: &Child) -> io::Result<()> {
    let pid = i32::try_from(child.id()).map_err(io::Error::other)?;
    // SAFETY: the signal targets only this test's private child.
    if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[test]
fn a_server_answers_its_private_socket_and_saves_on_termination() {
    let fixture = Fixture::new(false).unwrap();
    let mut child = Running(fixture.command().arg("--skip-agent-setup").spawn().unwrap());
    let mut stream = ready(&fixture, &mut child.0).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream.write_all(b"{\"cmd\":\"version\"}\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let response: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["ok"], true, "{response}");
    let created = control(
        &fixture,
        &serde_json::json!({"cmd":"session.new", "args":{"name":"quit-save-probe","command":"true","wait":true}}),
    )
    .unwrap();
    assert_eq!(created["ok"], true, "{created}");
    let closed = control(
        &fixture,
        &serde_json::json!({"cmd":"session.close", "target":created["result"]["id"], "args":{"undoable":true}}),
    )
    .unwrap();
    assert_eq!(closed["ok"], true, "{closed}");
    let second = fixture.command().arg("--skip-agent-setup").output().unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("another instance is already running on"));
    terminate(&child.0).unwrap();
    assert!(wait_exit(&mut child.0).unwrap().success());
    assert!(fixture.root.path().join("state/windows.json").is_file());
    for entry in std::fs::read_dir(fixture.root.path().join("state/windows")).unwrap() {
        let file = entry.unwrap().path();
        if file.extension().is_some_and(|extension| extension == "json") {
            assert!(!std::fs::read_to_string(file).unwrap().contains("quit-save-probe"));
        }
    }
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
        let mut child = Running(fixture.command().args(["--opencode-version", "v2"]).spawn().unwrap());
        drop(ready(&fixture, &mut child.0).unwrap());
        assert!(
            fixture
                .root
                .path()
                .join("home/.claude/skills/ngterm/SKILL.md")
                .is_file()
        );
        terminate(&child.0).unwrap();
        assert!(wait_exit(&mut child.0).unwrap().success());
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
    let mut child = Running(fixture.command().arg("--skip-agent-setup").spawn().unwrap());
    let log = fixture.root.path().join("state/ngterm.log");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if std::fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("the control server is not serving")
        {
            break;
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "server exited before the control refusal"
        );
        assert!(Instant::now() < deadline, "control refusal not logged");
        std::thread::yield_now();
    }
    terminate(&child.0).unwrap();
    assert!(!wait_exit(&mut child.0).unwrap().success());
    assert!(fixture.root.path().join("state/windows.json").is_file());
}

#[test]
fn unknown_opencode_reports_the_skipped_hooks_and_preserves_user_owned_skills() {
    let fixture = Fixture::new(true).unwrap();
    let config = fixture.root.path().join("home/.config/opencode");
    std::fs::create_dir_all(&config).unwrap();
    let skill = fixture.root.path().join("home/.claude/skills/ngterm");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "user owned skill").unwrap();
    let output = fixture
        .command()
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
    let mut child = Running(fixture.command().spawn().unwrap());
    drop(ready(&fixture, &mut child.0).unwrap());
    let log = std::fs::read_to_string(fixture.root.path().join("state/ngterm.log")).unwrap();
    assert!(
        log.contains("OpenCode version could not be detected; skipping its hooks."),
        "{log}"
    );
    terminate(&child.0).unwrap();
    assert!(wait_exit(&mut child.0).unwrap().success());
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
