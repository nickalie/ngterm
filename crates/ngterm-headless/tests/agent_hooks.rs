//! The server's explicit hooks install works without starting an instance, even while its state directory is locked,
//! and a staged build's start installs the skill and hooks by itself.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use ngterm_core::state_lock::StateDirectoryLock;

fn installer(home: &Path, state: &Path) -> Command {
    let mut command = headless(Path::new(env!("CARGO_BIN_EXE_ngterm-headless")), home, state);
    command.arg("--install-agent-hooks");
    command
}

/// `binary` over a private home and state directory.
fn headless(binary: &Path, home: &Path, state: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join(".config"))
        .env("LOCALAPPDATA", home.join(".local"))
        .env("NGTERM_STATE_DIR", state)
        .env("SHELL", home.join("missing-shell"))
        .env("PATH", "");
    // cmd.exe needs the Windows installation directory; everything user-specific stays isolated.
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    // Child profiles belong to the same measurement as this test's profile.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

#[test]
fn hooks_install_while_the_server_runs_and_a_reinstall_preserves_settings() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    fs::create_dir_all(home.join(".codex")).unwrap();
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".codex/config.toml"), "model = \"custom\"\n").unwrap();
    fs::write(home.join(".claude/settings.json"), "{\"model\":\"custom\"}\n").unwrap();
    fs::write(home.join(".bashrc"), "export USER_SETTING=kept\n").unwrap();
    let _lock = StateDirectoryLock::acquire(&state).claim(&state).unwrap();

    let output = installer(&home, &state).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_skill_and_hooks_reported(&output.stdout, &home);
    assert!(
        home.join(".config/ngterm/agent-status/ngterm-agent-status.sh")
            .is_file()
    );
    let configs = [".codex/config.toml", ".claude/settings.json", ".bashrc"];
    let installed: Vec<_> = configs
        .iter()
        .map(|name| fs::read_to_string(home.join(name)).unwrap())
        .collect();
    assert!(installed[0].contains("model = \"custom\""));
    assert!(installed[0].contains("ngterm-codex-status.sh"));
    assert!(installed[1].contains("custom") && installed[1].contains("ngterm-claude-status"));
    assert!(installed[2].contains("export USER_SETTING=kept"));
    assert!(installed[2].contains("shell/integration.sh"));

    let output = installer(&home, &state).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    for (name, previous) in configs.iter().zip(installed) {
        assert_eq!(fs::read_to_string(home.join(name)).unwrap(), previous, "{name}");
    }
    assert!(
        !state.join("windows.json").exists(),
        "installation must not start an instance"
    );
}

#[test]
fn an_unknown_opencode_version_is_reported_and_can_be_selected_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    fs::create_dir_all(home.join(".config/opencode")).unwrap();
    let output = installer(&home, &state).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("--opencode-version"));
    assert!(!home.join(".config/opencode/plugins/ngterm-status.js").exists());
    let output = installer(&home, &state)
        .args(["--opencode-version", "v1"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(home.join(".config/opencode/plugins/ngterm-status.js").is_file());
    assert!(
        !state.exists(),
        "the installer must not create or lock the state directory"
    );
}

/// Both installers reported on `stdout`, and the skill is in both agents' skills directories under `home`.
fn assert_skill_and_hooks_reported(stdout: &[u8], home: &Path) {
    let stdout = String::from_utf8_lossy(stdout);
    assert!(stdout.contains("Agent Status Hooks Installed"), "{stdout}");
    assert!(stdout.contains("Agent Skill Installed"), "{stdout}");
    for agent in [".claude", ".codex"] {
        assert!(home.join(agent).join("skills/ngterm/SKILL.md").is_file(), "{agent}");
    }
}

/// A staged layout (`scripts/stage.sh`'s, as far as the installers read it): this build's binary under `bin`, the
/// bundled hooks package and skill under `share/ngterm`, and no `ngterm-session`, so the start ends at the live probe.
fn stage(root: &Path) -> io::Result<PathBuf> {
    let bin = root.join("bin");
    fs::create_dir_all(&bin)?;
    let source = Path::new(env!("CARGO_BIN_EXE_ngterm-headless"));
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::other("the binary has no name"))?;
    let binary = bin.join(name);
    if fs::hard_link(source, &binary).is_err() {
        ngterm_host::fakes::copy_executable(source, &binary)?;
    }
    let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources");
    for relative in ["agent-status", "skills/ngterm"] {
        copy_tree(&resources.join(relative), &root.join("share/ngterm").join(relative))?;
    }
    Ok(binary)
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[test]
fn a_staged_start_installs_the_skill_and_hooks_unless_told_not_to() {
    let dir = tempfile::tempdir().unwrap();
    let binary = stage(&dir.path().join("stage")).unwrap();
    let (home, state) = (dir.path().join("home"), dir.path().join("state"));
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/settings.json"), "{\"model\":\"custom\"}\n").unwrap();

    let output = headless(&binary, &home, &state)
        .arg("--skip-agent-setup")
        .output()
        .unwrap();
    assert!(!output.status.success(), "no ngterm-session is staged: {output:?}");
    assert!(!home.join(".config/ngterm").exists());
    assert!(!home.join(".claude/skills").exists());
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
        "{\"model\":\"custom\"}\n"
    );

    let output = headless(&binary, &home, &state).output().unwrap();
    assert!(!output.status.success(), "no ngterm-session is staged: {output:?}");
    assert!(
        home.join(".config/ngterm/agent-status/ngterm-agent-status.sh")
            .is_file()
    );
    assert!(home.join(".claude/skills/ngterm/SKILL.md").is_file());
    let settings = fs::read_to_string(home.join(".claude/settings.json")).unwrap();
    assert!(
        settings.contains("custom") && settings.contains("ngterm-claude-status"),
        "{settings}"
    );

    // the next start brings a stale install back to the bundle's
    fs::write(
        home.join(".claude/skills/ngterm/SKILL.md"),
        "---\nname: ngterm\n---\n<!-- ngterm-skill -->\nold\n",
    )
    .unwrap();
    let output = headless(&binary, &home, &state).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/skills/ngterm/SKILL.md");
    assert_eq!(
        fs::read_to_string(home.join(".claude/skills/ngterm/SKILL.md")).unwrap(),
        fs::read_to_string(bundled).unwrap()
    );
    assert_eq!(
        fs::read_to_string(home.join(".claude/settings.json")).unwrap(),
        settings
    );
}

#[test]
fn a_hooks_failure_still_reports_the_skill_it_installed() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let state = dir.path().join("state");
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::create_dir_all(home.join(".config")).unwrap();
    // a file where the hooks package's directory goes: the package cannot be copied
    fs::write(home.join(".config/ngterm"), "not a directory\n").unwrap();

    let output = installer(&home, &state).output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Agent Skill Installed"), "{stdout}");
    assert!(!stdout.contains("Agent Status Hooks Installed"), "{stdout}");
    assert!(home.join(".claude/skills/ngterm/SKILL.md").is_file());
    assert!(!String::from_utf8_lossy(&output.stderr).is_empty(), "{output:?}");
}
