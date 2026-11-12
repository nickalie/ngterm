//! Startup integration for the Windows shells hosted directly by the live server.

use std::collections::BTreeMap;

use super::direct::DirectShell;
use crate::live::support::Inputs;

const WSL_GHOSTTY: &str = "NGTERM_WSL_GHOSTTY_RESOURCES";
const WSL_OWN: &str = "NGTERM_WSL_OWN_RESOURCES";
const WSL_CWD: &str = "NGTERM_WSL_RESTORE_CWD";
const CMD_CWD: &str = "$E]7;kitty-shell-cwd://localhost/$P$E\\";
const WSL_START: &str = include_str!("../../../../resources/shell-integration/wsl/launch.sh");

pub fn configure(kind: DirectShell, inputs: &Inputs, environment: &mut BTreeMap<String, String>) {
    match kind {
        DirectShell::Cmd => {
            let prompt = environment
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("PROMPT"))
                .map(|(_, value)| value.as_str())
                .or(inputs.inherited.cmd_prompt.as_deref())
                .unwrap_or("$P$G");
            let prompt = if prompt.starts_with(CMD_CWD) {
                prompt.to_owned()
            } else {
                format!("{CMD_CWD}{prompt}")
            };
            environment.retain(|key, _| !key.eq_ignore_ascii_case("PROMPT"));
            environment.insert("PROMPT".into(), prompt);
        }
        DirectShell::Wsl => {
            if let (Some(ghostty), Some(own)) = (&inputs.resources_dir, &inputs.own_resources_dir)
                && own.join("shell-integration/wsl/bash-env").is_file()
                && own.join("shell-integration/wsl/zsh/.zshenv").is_file()
            {
                environment.insert(WSL_GHOSTTY.into(), ghostty.to_string_lossy().into_owned());
                environment.insert(WSL_OWN.into(), own.to_string_lossy().into_owned());
            }
        }
    }
}

/// A Linux cwd must be restored inside WSL, never passed as a Windows process directory.
pub fn working_directory(profile: &super::ShellProfile, cwd: &str, environment: &mut BTreeMap<String, String>) {
    environment.remove(WSL_CWD);
    if profile.direct_live() == Some(DirectShell::Wsl) && cwd.starts_with('/') && !cwd.starts_with("//") {
        environment.insert(WSL_CWD.into(), cwd.to_owned());
    }
}

/// Resource paths are argv values, translated by wslpath inside the selected distribution.
/// No Windows path, user name or command is interpolated into shell source.
pub fn wsl_arguments(environment: &BTreeMap<String, String>, payload: Option<(&str, bool)>) -> Option<Vec<String>> {
    let ghostty = environment.get(WSL_GHOSTTY)?;
    let own = environment.get(WSL_OWN)?;
    let cwd = environment.get(WSL_CWD).map_or(":", String::as_str);
    let (line, mode) = match payload {
        Some((line, true)) => (line, "return"),
        Some((line, false)) => (line, "end"),
        None => (":", "login"),
    };
    // wsl.exe rejects an empty argv element (E_INVALIDARG); ':' is the equivalent empty command.
    let line = if line.is_empty() { ":" } else { line };
    Some(
        [
            "--exec",
            "/bin/sh",
            "-c",
            WSL_START,
            "ngterm-wsl",
            ghostty,
            own,
            mode,
            line,
            cwd,
        ]
        .map(str::to_owned)
        .to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_preserves_the_custom_prompt_and_does_not_duplicate_integration() {
        let mut environment = BTreeMap::from([("Prompt".into(), "hello $P$G".into())]);
        configure(DirectShell::Cmd, &Inputs::default(), &mut environment);
        configure(DirectShell::Cmd, &Inputs::default(), &mut environment);
        assert_eq!(environment["PROMPT"], format!("{CMD_CWD}hello $P$G"));
        assert!(!environment.contains_key("Prompt"));
        let mut inputs = Inputs::default();
        inputs.inherited.cmd_prompt = Some("inherited $G".into());
        let mut environment = BTreeMap::new();
        configure(DirectShell::Cmd, &inputs, &mut environment);
        assert_eq!(environment["PROMPT"], format!("{CMD_CWD}inherited $G"));
    }

    #[test]
    fn wsl_keeps_paths_and_command_text_in_separate_arguments() {
        let environment = BTreeMap::from([
            (WSL_GHOSTTY.into(), "C:/a b/it's/ghostty".into()),
            (WSL_OWN.into(), "C:/a b/ngterm".into()),
        ]);
        let line = "printf '%s' 'a b; $(false)'";
        let argv = wsl_arguments(&environment, Some((line, true))).unwrap();
        assert_eq!(
            &argv[5..],
            ["C:/a b/it's/ghostty", "C:/a b/ngterm", "return", line, ":"]
        );
        assert_eq!(wsl_arguments(&BTreeMap::new(), None), None);
        assert!(!WSL_START.contains(line));
        assert!(
            wsl_arguments(&environment, None)
                .unwrap()
                .iter()
                .all(|word| !word.is_empty())
        );
        assert!(
            wsl_arguments(&environment, Some(("", false)))
                .unwrap()
                .iter()
                .all(|word| !word.is_empty())
        );
    }

    #[test]
    fn only_a_wsl_linux_directory_is_restored_inside_the_distribution() {
        let profile = super::super::ShellProfile::new("wsl", "WSL", "C:/Windows/System32/wsl.exe".into(), &[]);
        let mut environment = BTreeMap::new();
        working_directory(&profile, "/home/a b/quote'", &mut environment);
        assert_eq!(environment[WSL_CWD], "/home/a b/quote'");
        working_directory(&profile, "C:/Users/me", &mut environment);
        assert!(!environment.contains_key(WSL_CWD));
    }

    #[test]
    fn wsl_injection_requires_its_bundled_loaders_and_keeps_plain_startup_as_fallback() {
        let resources = tempfile::tempdir().unwrap();
        let inputs = Inputs {
            resources_dir: Some(resources.path().join("ghostty")),
            own_resources_dir: Some(resources.path().to_owned()),
            ..Inputs::default()
        };
        let mut environment = BTreeMap::new();
        configure(DirectShell::Wsl, &inputs, &mut environment);
        assert!(wsl_arguments(&environment, None).is_none());
        let loaders = resources.path().join("shell-integration/wsl");
        std::fs::create_dir_all(loaders.join("zsh")).unwrap();
        std::fs::write(loaders.join("bash-env"), "").unwrap();
        std::fs::write(loaders.join("zsh/.zshenv"), "").unwrap();
        configure(DirectShell::Wsl, &inputs, &mut environment);
        let profile = super::super::ShellProfile::new(
            "wsl",
            "WSL",
            "C:/Windows/System32/wsl.exe".into(),
            &["-d", "Ubuntu Dev", "-u", "alice"],
        );
        let args = super::super::direct::integrated_arguments(&profile, None, None, &["unused"], &environment);
        assert_eq!(
            &args[..5],
            [profile.executable.as_str(), "-d", "Ubuntu Dev", "-u", "alice"]
        );
        assert_eq!(&args[5..8], ["--exec", "/bin/sh", "-c"]);
        assert_eq!(
            &args[10..12],
            [
                inputs.resources_dir.unwrap().to_string_lossy(),
                resources.path().to_string_lossy(),
            ]
        );
    }
}
