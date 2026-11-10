use super::*;

fn sha256(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}

/// The arguments [`ROOT_SCRIPT`] takes after the package kind, for an unchanged installation in `install`.
fn identity_args(install: &PrivilegedInstall) -> [std::ffi::OsString; 2] {
    [
        install.identity.clone().into_os_string(),
        install.identity_sha256.clone().into(),
    ]
}

/// A stage holding `package.deb`, with an installed identity matching the recorded one.
fn install(dir: &Path, sha: &str) -> PrivilegedInstall {
    fs::write(dir.join("package.deb"), b"package").unwrap();
    fs::write(dir.join("expected-identity"), "0.1.0\n").unwrap();
    fs::write(dir.join("app-BUILD"), "0.1.0\n").unwrap();
    PrivilegedInstall {
        package: dir.join("package.deb"),
        sha256: sha.to_owned(),
        kind: "deb",
        work: dir.to_owned(),
        identity: dir.join("app-BUILD"),
        identity_sha256: sha256_hex(b"0.1.0\n"),
        executable: dir.join("ngterm"),
        prompter: None,
    }
}

#[cfg(unix)]
fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// `PATH` with `bin` first, so its fakes stand in for the system's installers.
#[cfg(unix)]
fn path_with(bin: &Path) -> String {
    format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
}

#[test]
fn a_prompt_ends_as_its_exit_and_message_say() {
    assert_eq!(classify(Some(0), "").unwrap(), PromptOutcome::Installed);
    assert_eq!(classify(Some(126), "").unwrap(), PromptOutcome::Cancelled);
    assert_eq!(
        classify(Some(1), "execution error: User canceled. (-128)").unwrap(),
        PromptOutcome::Cancelled
    );
    for message in [
        "Error executing command as another user: No authentication agent found.",
        "Error getting authority: Error initializing authority",
        "pkexec must be setuid root",
    ] {
        assert_eq!(classify(Some(127), message).unwrap(), PromptOutcome::Unavailable);
    }
    assert_eq!(
        classify(Some(100), "one\n\ntwo\nthree\nE: four\n")
            .unwrap_err()
            .to_string(),
        "The update could not be installed.\ntwo\nthree\nE: four"
    );
}

#[test]
fn quoting_survives_a_quote_in_a_path() {
    assert_eq!(shell_quote("/a b/it's"), r"'/a b/it'\''s'");
}

#[test]
fn a_changed_installation_refuses_the_stale_package() {
    let dir = tempfile::tempdir().unwrap();
    let install = install(dir.path(), "sum");
    install.check_unchanged().unwrap();
    fs::write(dir.path().join("app-BUILD"), "0.3.0\n").unwrap();
    assert_eq!(
        install.check_unchanged().unwrap_err().to_string(),
        "The installed version changed after the download started. Check for updates again."
    );
}

#[test]
fn the_prompt_is_the_platforms_and_hands_over_the_package() {
    let dir = tempfile::tempdir().unwrap();
    let mut install = install(dir.path(), "sum");
    let arguments = |command: &Command| -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    };
    let linux = install.prompt_command();
    assert_eq!(linux.get_program(), "pkexec");
    let package = install.package.to_string_lossy().into_owned();
    let identity = install.identity.to_string_lossy().into_owned();
    let hash = install.identity_sha256.clone();
    assert_eq!(
        arguments(&linux),
        [
            "--disable-internal-agent".to_owned(),
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            ROOT_SCRIPT.to_owned(),
            "ngterm-update".to_owned(),
            package.clone(),
            "sum".to_owned(),
            "deb".to_owned(),
            identity.clone(),
            hash.clone(),
        ]
    );
    install.kind = MAC_PACKAGE;
    let mac = install.prompt_command();
    assert_eq!(mac.get_program(), "/usr/bin/osascript");
    assert_eq!(
        arguments(&mac)[1..],
        [
            "on run argv",
            "-e",
            MAC_RUN,
            "-e",
            "end run",
            ROOT_SCRIPT,
            package.as_str(),
            "sum",
            "pkg",
            identity.as_str(),
            hash.as_str()
        ]
    );
}

#[test]
#[cfg(unix)]
fn a_missing_prompter_means_no_popup() {
    let dir = tempfile::tempdir().unwrap();
    let mut install = install(dir.path(), "sum");
    install.prompter = Some(dir.path().join("absent"));
    assert_eq!(install.prompt().unwrap(), PromptOutcome::Unavailable);
}

#[test]
#[cfg(unix)]
fn the_root_script_installs_only_the_package_that_was_verified() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.path().join("log");
    executable(
        &bin.join("apt-get"),
        &format!(
            r#"for last; do :; done; cat "$last" > '{}'; echo "$last" >> '{}.path'"#,
            log.display(),
            log.display()
        ),
    );
    let run = |sha: &str| {
        let install = install(dir.path(), sha);
        Command::new("/bin/sh")
            .args(["-c", ROOT_SCRIPT, "ngterm-update"])
            .arg(&install.package)
            .arg(sha)
            .arg("deb")
            .args(identity_args(&install))
            .env("PATH", path_with(&bin))
            .output()
            .unwrap()
    };

    let refused = run(&"0".repeat(64));
    assert_eq!(refused.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr),
        "The downloaded package changed after it was verified.\n"
    );
    assert!(!log.exists(), "a changed package never reaches the installer");

    let installed = run(&sha256(b"package"));
    assert!(installed.status.success(), "{installed:?}");
    assert_eq!(fs::read(&log).unwrap(), b"package");
    let used = fs::read_to_string(dir.path().join("log.path")).unwrap();
    assert!(
        used.starts_with("/var/tmp/ngterm-update."),
        "installs its own copy: {used}"
    );
    assert!(!Path::new(used.trim()).exists(), "the copy is removed afterwards");
}

#[test]
#[cfg(unix)]
fn each_system_package_goes_to_its_installer() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let log = dir.path().join("called");
    for tool in ["dnf", "pacman"] {
        executable(&bin.join(tool), &format!(r"echo {tool} $1 >> '{}'", log.display()));
    }
    let install = install(dir.path(), "");
    for kind in ["rpm", "pacman"] {
        let output = Command::new("/bin/sh")
            .args(["-c", ROOT_SCRIPT, "ngterm-update"])
            .arg(&install.package)
            .arg(sha256(b"package"))
            .arg(kind)
            .args(identity_args(&install))
            .env("PATH", path_with(&bin))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }
    assert_eq!(fs::read_to_string(&log).unwrap(), "dnf install\npacman -U\n");
    let unknown = Command::new("/bin/sh")
        .args(["-c", ROOT_SCRIPT, "ngterm-update"])
        .arg(&install.package)
        .arg(sha256(b"package"))
        .arg("flatpak")
        .args(identity_args(&install))
        .output()
        .unwrap();
    assert_eq!(unknown.status.code(), Some(2));
}

#[cfg(unix)]
fn run_pane(terminal: &TerminalInstall, bin: &Path) -> std::process::Output {
    use std::io::Write as _;
    let mut child = Command::new("/bin/sh")
        .args(["-c", terminal.command()])
        .env("PATH", path_with(bin))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"\n").unwrap();
    child.wait_with_output().unwrap()
}

#[test]
#[cfg(unix)]
fn a_terminal_install_runs_sudo_and_reports_through_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let args = dir.path().join("args");
    let install = install(dir.path(), "the-sum");
    let terminal = install.terminal().unwrap();
    assert_eq!(terminal.status(), TerminalStatus::Running, "the pane has yet to start");

    executable(&bin.join("sudo"), &format!(r#"echo "$@" > '{}'"#, args.display()));
    let output = run_pane(&terminal, &bin);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(terminal.status(), TerminalStatus::Installed);
    assert_eq!(
        fs::read_to_string(&args).unwrap().trim(),
        format!(
            "/bin/sh {}/install-as-root.sh {} the-sum deb {} {}",
            dir.path().display(),
            install.package.display(),
            install.identity.display(),
            install.identity_sha256
        )
    );

    executable(&bin.join("sudo"), "exit 7");
    let terminal = install.terminal().unwrap();
    assert_eq!(
        terminal.status(),
        TerminalStatus::Running,
        "the last run's result is gone"
    );
    let output = run_pane(&terminal, &bin);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Press Enter"),
        "a failure holds the pane: {output:?}"
    );
    assert_eq!(terminal.status(), TerminalStatus::Failed("7".into()));
}

#[test]
#[cfg(unix)]
fn a_pane_that_goes_away_without_a_result_counts_as_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let terminal = install(dir.path(), "sum").terminal().unwrap();
    assert_eq!(terminal.status_after(Duration::ZERO), TerminalStatus::Running);
    assert_eq!(
        terminal.status_after(PANE_START_LIMIT),
        TerminalStatus::Cancelled,
        "it never started"
    );

    fs::write(dir.path().join(PANE_PID), std::process::id().to_string()).unwrap();
    assert_eq!(terminal.status(), TerminalStatus::Running);

    let mut gone = Command::new("/bin/sh").args(["-c", "exit 0"]).spawn().unwrap();
    let pid = gone.id();
    gone.wait().unwrap();
    fs::write(dir.path().join(PANE_PID), pid.to_string()).unwrap();
    assert_eq!(terminal.status(), TerminalStatus::Cancelled);
}

#[test]
#[cfg(unix)]
fn the_root_script_refuses_an_installation_that_changed_during_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let called = dir.path().join("called");
    executable(&bin.join("pacman"), &format!("touch '{}'", called.display()));
    let install = install(dir.path(), "");
    // the prompt was open while something else installed a newer version
    fs::write(&install.identity, "0.3.0\n").unwrap();
    let output = Command::new("/bin/sh")
        .args(["-c", ROOT_SCRIPT, "ngterm-update"])
        .arg(&install.package)
        .arg(sha256(b"package"))
        .arg("pacman")
        .args(identity_args(&install))
        .env("PATH", path_with(&bin))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "The installed version changed after the download started. Check for updates again.\n"
    );
    assert!(!called.exists(), "pacman would have downgraded it");
}
