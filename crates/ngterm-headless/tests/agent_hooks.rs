//! The server's explicit hooks install works without starting an instance, even while its state directory is locked.

use std::fs;
use std::path::Path;
use std::process::Command;

use ngterm_core::state_lock::StateDirectoryLock;

fn installer(home: &Path, state: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ngterm-headless"));
    command
        .env_clear()
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("APPDATA", home.join(".config"))
        .env("LOCALAPPDATA", home.join(".local"))
        .env("NGTERM_STATE_DIR", state)
        .env("SHELL", home.join("missing-shell"))
        .env("PATH", "")
        .arg("--install-agent-hooks");
    // cmd.exe needs the Windows installation directory; everything user-specific stays isolated.
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
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
    assert!(String::from_utf8_lossy(&output.stdout).contains("Agent Status Hooks Installed"));
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
