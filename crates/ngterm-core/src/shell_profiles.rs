//! Shell profiles shared by the ordinary and persistent launch paths. A pane keeps a copy of its
//! profile, so changing the default or removing a catalog entry never changes an existing pane.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub mod direct;
#[cfg(windows)]
mod windows;

/// An executable and its arguments, never a shell command to parse or interpolate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellProfile {
    pub id: String,
    pub name: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_directory: Option<String>,
}

impl ShellProfile {
    #[must_use]
    pub fn new(id: &str, name: &str, executable: String, args: &[&str]) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            executable,
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: BTreeMap::new(),
            starting_directory: None,
        }
    }

    /// Validate structure separately from availability: a removed executable remains selected and
    /// produces a launch error, instead of silently changing the user's shell.
    #[must_use]
    pub fn valid(&self) -> bool {
        !self.id.trim().is_empty()
            && !self.name.trim().is_empty()
            && (Path::new(&self.executable).is_absolute() || windows_absolute(&self.executable))
            && !self.executable.contains(['\0', '\r', '\n'])
            && self.args.iter().all(|arg| !arg.contains('\0'))
            && self
                .env
                .iter()
                .all(|(key, value)| !key.is_empty() && !key.contains(['=', '\0']) && !value.contains('\0'))
    }

    /// Live integrates PowerShell/Git Bash and directly hosts CMD/WSL on Windows. Unsupported arguments
    /// are not silently discarded; the UI offers an ordinary session for that profile instead.
    #[must_use]
    pub fn live_shell(&self) -> Option<&str> {
        self.valid().then_some(())?;
        if self.direct_live().is_some() {
            return Some(&self.executable);
        }
        crate::live::support::profile_shell(&self.executable, &self.args)
    }
}

#[must_use]
pub fn windows_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with(r"\\")
        || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\'))
}

/// Forward-slash paths are accepted by Git Bash without assuming a particular MSYS mount table.
#[must_use]
pub fn bash_path(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        return format!("//{}", unc.replace('\\', "/"));
    }
    let path = path.strip_prefix(r"\\?\").unwrap_or(path);
    if windows_absolute(path) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

/// User entries replace detected profiles by id. Detection never executes a discovered shell.
#[must_use]
pub fn available(custom: &[ShellProfile]) -> Vec<ShellProfile> {
    #[cfg(windows)]
    let mut profiles = {
        // Sidebar snapshots are frequent; filesystem/registry discovery runs only once per app launch.
        static DETECTED: std::sync::OnceLock<Vec<ShellProfile>> = std::sync::OnceLock::new();
        DETECTED.get_or_init(windows::discover).clone()
    };
    #[cfg(not(windows))]
    let mut profiles: Vec<ShellProfile> = Vec::new();
    for profile in custom.iter().filter(|profile| profile.valid()) {
        if let Some(known) = profiles.iter_mut().find(|known| known.id == profile.id) {
            *known = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
    }
    profiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_round_trip_preserves_spaces_arguments_and_environment() {
        let mut profile = ShellProfile::new(
            "git-bash",
            "Git Bash",
            r"C:\Program Files\Git\bin\bash.exe".into(),
            &["--login", "-i"],
        );
        profile.env.insert("PROJECT".into(), "a b & c".into());
        assert!(profile.valid());
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(serde_json::from_str::<ShellProfile>(&json).unwrap(), profile);
        assert_eq!(profile.live_shell(), Some(profile.executable.as_str()));
    }

    #[test]
    fn unknown_shells_and_custom_startup_commands_are_not_live() {
        for (exe, args) in [
            ("C:/Windows/System32/cmd.exe", vec!["/C", "exit"]),
            ("C:/Git/bin/bash.exe", vec!["-c", "echo hi"]),
        ] {
            let profile = ShellProfile::new("custom", "Custom", exe.into(), &args);
            assert!(profile.valid());
            assert_eq!(profile.live_shell(), None);
        }
    }

    #[test]
    fn bash_paths_preserve_spaces_unc_and_non_windows_paths() {
        assert_eq!(
            bash_path(r"C:\Program Files\ngterm\ghostty.bash"),
            "C:/Program Files/ngterm/ghostty.bash"
        );
        assert_eq!(bash_path(r"\\server\share\file"), "//server/share/file");
        assert_eq!(bash_path(r"\\?\C:\Program Files\Git"), "C:/Program Files/Git");
        assert_eq!(bash_path(r"\\?\UNC\server\share"), "//server/share");
        assert_eq!(bash_path("/home/a\\b"), "/home/a\\b");
    }

    #[test]
    fn invalid_profiles_are_not_offered() {
        let mut profile = ShellProfile::new("bad", "Bad", "bash.exe".into(), &[]);
        assert!(!profile.valid());
        profile.executable = "C:/Git/bin/bash.exe".into();
        profile.env.insert("BAD=KEY".into(), "value".into());
        assert!(!profile.valid());
        assert!(!available(&[profile]).iter().any(|entry| entry.id == "bad"));
    }
}
