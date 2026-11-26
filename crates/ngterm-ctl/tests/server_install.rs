//! `ngtermctl server install` against what it runs on: the terminal ssh prompts on, and the host side of the install
//! (the unpack under each login shell at hand, then the release archive's `server-install.sh`) on a home of its own,
//! with `systemctl` and `loginctl` stood in by scripts that log their calls. Linux: the host is Linux, and the
//! terminal check runs util-linux's `script`.
#![cfg(target_os = "linux")]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use ngterm_core::remote::server_install::{INSTALLER_COMMAND, unpack_command};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const HOST_SCRIPT: &str = include_str!("../../../scripts/server-host-install.sh");

fn executable(path: &Path, text: &str) -> TestResult {
    fs::write(path, text)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn wait(mut child: Child, limit: Duration) -> TestResult<Option<Output>> {
    let start = Instant::now();
    while child.try_wait()?.is_none() {
        if start.elapsed() > limit {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(Some(child.wait_with_output()?))
}

/// ssh reads a password or host-key answer from /dev/tty, which stops a background process group with SIGTTIN: the
/// probe must run in the terminal's foreground group (upstream's `theProbeCanReadTheTerminalItRunsIn`).
#[test]
fn the_probe_can_read_the_terminal_it_runs_in() -> TestResult {
    let root = tempfile::tempdir()?;
    let install = root.path().join("ngterm");
    fs::create_dir_all(install.join("bin"))?;
    fs::create_dir_all(install.join("share/ngterm"))?;
    fs::write(install.join("share/ngterm/app-BUILD"), "1.2.3\nabc\n")?;
    fs::write(install.join("share/ngterm/update-format"), "linux-archive\n")?;
    let client = install.join("bin/ngtermctl");
    fs::copy(env!("CARGO_BIN_EXE_ngtermctl"), &client)?;
    let bin = root.path().join("fakebin");
    fs::create_dir(&bin)?;
    let answer = root.path().join("tty.answer");
    executable(
        &bin.join("ssh"),
        &format!(
            "#!/bin/sh\nIFS= read -r answer < /dev/tty\nprintf '%s\\n' \"$answer\" > '{}'\necho riscv64\n",
            answer.display()
        ),
    )?;
    let mut child = Command::new("script")
        .args([
            "-q",
            "-e",
            "-c",
            &format!("'{}' server install box", client.display()),
            "/dev/null",
        ])
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("HOME", root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child.stdin.take().ok_or("no stdin")?.write_all(b"yes\n")?;
    let output = wait(child, Duration::from_secs(20))?.ok_or("ngtermctl did not finish within 20 seconds")?;
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("unsupported host architecture 'riscv64'"), "{text}");
    assert_eq!(fs::read_to_string(answer)?, "yes\n");
    Ok(())
}

/// A home with a release archive's stand-in to install: `usr/bin` stubs that log to `LOG` and the real
/// `server-install.sh`.
struct Host {
    root: tempfile::TempDir,
}

impl Host {
    fn new() -> TestResult<Self> {
        let host = Self {
            root: tempfile::tempdir()?,
        };
        fs::create_dir_all(host.home())?;
        fs::create_dir_all(host.stubs())?;
        executable(
            &host.stubs().join("systemctl"),
            "#!/bin/sh\necho \"systemctl $*\" >> \"$LOG\"\ncase \"$*\" in\n  *is-active*) [ -e \"$ACTIVE\" ] ;;\n  \
             *show*) cat \"$SERVER_PID\" ;;\n  *start*) touch \"$ACTIVE\" ;;\nesac\n",
        )?;
        executable(
            &host.stubs().join("loginctl"),
            "#!/bin/sh\necho \"loginctl $*\" >> \"$LOG\"\ncase \"$1\" in\n  show-user) cat \"$LINGER\" ;;\n  \
             enable-linger) [ \"$LINGER_ALLOWED\" = yes ] && echo yes > \"$LINGER\" ;;\nesac\n",
        )?;
        Ok(host)
    }

    fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    fn stubs(&self) -> PathBuf {
        self.root.path().join("stubs")
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("log")
    }

    fn lib(&self) -> PathBuf {
        self.home().join(".local/lib/ngterm")
    }

    /// The archive of a release whose binaries say `build`, as the release packages it: everything under `ngterm/`.
    fn archive(&self, build: &str) -> TestResult<PathBuf> {
        let stage = self.root.path().join(format!("stage-{build}"));
        let usr = stage.join("ngterm/usr");
        fs::create_dir_all(usr.join("bin"))?;
        fs::create_dir_all(usr.join("share/ngterm"))?;
        executable(
            &usr.join("bin/ngtermctl"),
            &format!("#!/bin/sh\necho \"ngtermctl {build} $* SERVER_MARK=$SERVER_MARK pwd=$(pwd -P)\" >> \"$LOG\"\n"),
        )?;
        executable(
            &usr.join("bin/ngterm-headless"),
            &format!("#!/bin/sh\necho \"ngterm-headless {build} $*\" >> \"$LOG\"\nexit \"${{HOOKS_EXIT:-0}}\"\n"),
        )?;
        executable(&usr.join("share/ngterm/server-install.sh"), HOST_SCRIPT)?;
        let archive = self.root.path().join(format!("{build}.tar.gz"));
        let status = Command::new("tar")
            .arg("-C")
            .arg(&stage)
            .arg("-czf")
            .arg(&archive)
            .arg("ngterm")
            .status()?;
        assert!(status.success());
        Ok(archive)
    }

    /// The remote unpack as `shell` runs the command sshd hands it.
    fn unpack(&self, shell: &str, archive: &Path) -> TestResult<Output> {
        Ok(Command::new(shell)
            .args(["-c", &unpack_command()])
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", "/usr/bin:/bin")
            .stdin(fs::File::open(archive)?)
            .output()?)
    }

    /// The remote installer, beside a stand-in server process whose environment and working directory the script
    /// must hand `ngtermctl version`.
    fn install(&self, linger: &str, linger_allowed: bool, hooks_exit: i32) -> TestResult<Output> {
        let linger_file = self.root.path().join("linger");
        fs::write(&linger_file, format!("{linger}\n"))?;
        let server_cwd = self.root.path().join("server-cwd");
        fs::create_dir_all(&server_cwd)?;
        let mut server = Command::new("sleep")
            .arg("30")
            .env_clear()
            .env("LOG", self.log())
            .env("SERVER_MARK", "from-the-server")
            .current_dir(&server_cwd)
            .spawn()?;
        let pid = self.root.path().join("server-pid");
        fs::write(&pid, format!("{}\n", server.id()))?;
        let output = Command::new("bash")
            .args(["-c", INSTALLER_COMMAND])
            .env_clear()
            .env("HOME", self.home())
            .env("PATH", format!("{}:/usr/bin:/bin", self.stubs().display()))
            .env("SHELL", "/bin/sh")
            .env("LOG", self.log())
            .env("ACTIVE", self.root.path().join("active"))
            .env("SERVER_PID", &pid)
            .env("LINGER", &linger_file)
            .env("LINGER_ALLOWED", if linger_allowed { "yes" } else { "no" })
            .env("HOOKS_EXIT", hooks_exit.to_string())
            .output();
        server.kill()?;
        server.wait()?;
        Ok(output?)
    }

    fn take_log(&self) -> Vec<String> {
        let text = fs::read_to_string(self.log()).unwrap_or_default();
        fs::remove_file(self.log()).ok();
        text.lines().map(str::to_owned).collect()
    }
}

fn shells() -> Vec<&'static str> {
    [
        "/bin/sh",
        "/bin/bash",
        "/bin/dash",
        "/usr/bin/fish",
        "/bin/zsh",
        "/bin/tcsh",
    ]
    .into_iter()
    .filter(|shell| Path::new(shell).exists())
    .collect()
}

#[test]
fn the_unpack_swaps_the_archive_in_under_every_login_shell() -> TestResult {
    for shell in shells() {
        let host = Host::new()?;
        for build in ["first", "second"] {
            let archive = host.archive(build)?;
            let output = host.unpack(shell, &archive)?;
            assert!(output.status.success(), "{shell}: {output:?}");
        }
        let lib = host.home().join(".local/lib");
        let mut entries: Vec<_> = fs::read_dir(&lib)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<Result<_, _>>()?;
        entries.sort();
        assert_eq!(entries, ["ngterm"], "{shell}");
        assert!(
            fs::read_to_string(host.lib().join("usr/bin/ngtermctl"))?.contains("ngtermctl second"),
            "{shell}"
        );
    }
    Ok(())
}

#[test]
fn an_archive_without_the_installer_never_replaces_the_live_one() -> TestResult {
    let host = Host::new()?;
    assert!(host.unpack("/bin/sh", &host.archive("first")?)?.status.success());
    let stage = host.root.path().join("bare");
    fs::create_dir_all(stage.join("ngterm/usr/bin"))?;
    let bare = host.root.path().join("bare.tar.gz");
    assert!(
        Command::new("tar")
            .arg("-C")
            .arg(&stage)
            .arg("-czf")
            .arg(&bare)
            .arg("ngterm")
            .status()?
            .success()
    );
    assert!(!host.unpack("/bin/sh", &bare)?.status.success());
    assert!(fs::read_to_string(host.lib().join("usr/bin/ngtermctl"))?.contains("ngtermctl first"));
    Ok(())
}

#[test]
fn a_first_install_writes_the_unit_enables_linger_starts_and_installs_the_hooks() -> TestResult {
    let host = Host::new()?;
    assert!(host.unpack("/bin/sh", &host.archive("first")?)?.status.success());
    let output = host.install("no", true, 0)?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let unit = fs::read_to_string(host.home().join(".config/systemd/user/ngterm-headless.service"))?;
    assert!(
        unit.contains("ExecStart=%h/.local/lib/ngterm/usr/bin/ngterm-headless\n"),
        "{unit}"
    );
    assert!(unit.contains("KillMode=process\n"), "{unit}");
    for name in ["ngtermctl", "ngterm-headless"] {
        assert_eq!(
            fs::read_link(host.home().join(".local/bin").join(name))?,
            host.lib().join("usr/bin").join(name)
        );
    }
    let user = String::from_utf8(Command::new("id").arg("-un").output()?.stdout)?;
    let server_cwd = fs::canonicalize(host.root.path().join("server-cwd"))?;
    let version = format!(
        "ngtermctl first version SERVER_MARK=from-the-server pwd={}",
        server_cwd.display()
    );
    assert_eq!(
        host.take_log(),
        [
            format!("loginctl show-user {} -p Linger --value", user.trim()),
            format!("loginctl enable-linger {}", user.trim()),
            "systemctl --user daemon-reload".to_owned(),
            "systemctl --user enable ngterm-headless".to_owned(),
            "systemctl --user is-active --quiet ngterm-headless".to_owned(),
            "systemctl --user start ngterm-headless".to_owned(),
            "systemctl --user show -p MainPID --value ngterm-headless".to_owned(),
            version.clone(),
            version,
            "ngterm-headless first --install-agent-hooks".to_owned(),
        ]
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("enabled lingering for"), "{stdout}");
    assert!(
        stdout.contains("a plain ssh command does not find ngtermctl"),
        "{stdout}"
    );
    Ok(())
}

#[test]
fn an_upgrade_keeps_the_unit_and_restarts_the_running_server() -> TestResult {
    let host = Host::new()?;
    assert!(host.unpack("/bin/sh", &host.archive("first")?)?.status.success());
    assert_eq!(host.install("no", true, 0)?.status.code(), Some(0));
    let unit = host.home().join(".config/systemd/user/ngterm-headless.service");
    fs::write(&unit, "[Service]\nExecStart=/edited\n")?;
    host.take_log();
    assert!(host.unpack("/bin/sh", &host.archive("second")?)?.status.success());
    let output = host.install("yes", false, 0)?;
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(fs::read_to_string(&unit)?, "[Service]\nExecStart=/edited\n");
    let log = host.take_log();
    assert!(
        log.contains(&"systemctl --user restart ngterm-headless".to_owned()),
        "{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.starts_with("loginctl enable-linger")),
        "{log:?}"
    );
    assert!(
        log.contains(&"ngterm-headless second --install-agent-hooks".to_owned()),
        "{log:?}"
    );
    Ok(())
}

#[test]
fn a_refused_linger_stops_before_anything_starts() -> TestResult {
    let host = Host::new()?;
    assert!(host.unpack("/bin/sh", &host.archive("first")?)?.status.success());
    let output = host.install("no", false, 0)?;
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr)?;
    let user = String::from_utf8(Command::new("id").arg("-un").output()?.stdout)?;
    assert!(
        stderr.contains(&format!(
            "Ask an administrator to run: loginctl enable-linger {}",
            user.trim()
        )),
        "{stderr}"
    );
    assert!(!host.take_log().iter().any(|line| line.starts_with("systemctl")));
    Ok(())
}

#[test]
fn incomplete_hooks_exit_3() -> TestResult {
    let host = Host::new()?;
    assert!(host.unpack("/bin/sh", &host.archive("first")?)?.status.success());
    let output = host.install("yes", true, 1)?;
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(String::from_utf8(output.stderr)?.contains("the agent hooks install is incomplete"));
    Ok(())
}
