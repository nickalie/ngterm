//! Windows shell discovery from registered installation roots, standard locations and PATH.
//! A bare bash.exe on PATH may be WSL's legacy launcher: only Git installations are detected as Git Bash.

mod wsl;

use super::ShellProfile;
use std::path::{Path, PathBuf};

pub(super) fn discover() -> Vec<ShellProfile> {
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    let system = std::env::var_os("SystemRoot").map(|dir| PathBuf::from(dir).join("System32"));
    let mut profiles = from_locations(&path, program_files.as_deref(), system.as_deref(), git_roots(&path));
    if let Some(system) = system.as_deref() {
        profiles.extend(wsl::discover(system));
    }
    profiles
}

fn from_locations(
    path: &[PathBuf],
    program_files: Option<&Path>,
    system: Option<&Path>,
    git_roots: Vec<PathBuf>,
) -> Vec<ShellProfile> {
    let mut profiles = Vec::new();
    let pwsh = path
        .iter()
        .map(|dir| dir.join("pwsh.exe"))
        .chain(program_files.iter().map(|dir| dir.join("PowerShell/7/pwsh.exe")))
        .find(|p| p.is_file());
    add(&mut profiles, "powershell", "PowerShell", pwsh, &["-NoLogo"]);
    add(
        &mut profiles,
        "windows-powershell",
        "Windows PowerShell",
        system
            .as_ref()
            .map(|dir| dir.join("WindowsPowerShell/v1.0/powershell.exe")),
        &["-NoLogo"],
    );
    let git = git_roots
        .into_iter()
        .map(|dir| dir.join("bin/bash.exe"))
        .find(|p| p.is_file());
    add(&mut profiles, "git-bash", "Git Bash", git, &["--login", "-i"]);
    if let Some(git) = profiles.iter_mut().find(|profile| profile.id == "git-bash") {
        git.env.insert("CHERE_INVOKING".into(), "1".into());
        git.env.insert("MSYSTEM".into(), "MINGW64".into());
    }
    add(
        &mut profiles,
        "cmd",
        "Command Prompt",
        system.map(|dir| dir.join("cmd.exe")),
        &["/D"],
    );
    profiles
}

fn add(profiles: &mut Vec<ShellProfile>, id: &str, name: &str, path: Option<PathBuf>, args: &[&str]) {
    if let Some(path) = path.filter(|path| path.is_absolute() && path.is_file()) {
        profiles.push(ShellProfile::new(id, name, path.to_string_lossy().into_owned(), args));
    }
}

fn git_roots(path: &[PathBuf]) -> Vec<PathBuf> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut roots = Vec::new();
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        if let Some(root) = git_registry_root(hive) {
            roots.push(root);
        }
    }
    for key in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(dir) = std::env::var_os(key) {
            roots.push(PathBuf::from(dir).join("Git"));
        }
    }
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(dir).join("Programs/Git"));
    }
    for dir in path.iter().filter(|dir| dir.join("git.exe").is_file()) {
        if let Some(root) = dir.parent() {
            roots.push(root.to_owned());
        }
    }
    roots
}

fn git_registry_root(hive: windows_sys::Win32::System::Registry::HKEY) -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<_>>();
    let (key, value) = (wide(r"Software\GitForWindows"), wide("InstallPath"));
    let mut buffer = vec![0u16; 32768];
    let mut bytes = u32::try_from(std::mem::size_of_val(buffer.as_slice())).ok()?;
    // SAFETY: predefined hive, NUL-terminated names, and a writable buffer with its size in bytes.
    let status = unsafe {
        RegGetValueW(
            hive,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let len = buffer.iter().position(|unit| *unit == 0)?;
    let value = String::from_utf16(&buffer[..len]).ok()?;
    (!value.is_empty() && Path::new(&value).is_absolute()).then(|| PathBuf::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_offers_only_existing_shells_and_prefers_pwsh_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let files = dir.path().join("Program Files");
        let system = dir.path().join("System32");
        let portable = dir.path().join("Portable PowerShell");
        let git = files.join("Git");
        for exe in [
            portable.join("pwsh.exe"),
            files.join("PowerShell/7/pwsh.exe"),
            system.join("WindowsPowerShell/v1.0/powershell.exe"),
            system.join("cmd.exe"),
            git.join("bin/bash.exe"),
        ] {
            std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
            std::fs::write(exe, "test executable; never run during detection").unwrap();
        }
        let profiles = from_locations(
            std::slice::from_ref(&portable),
            Some(&files),
            Some(&system),
            vec![git.clone()],
        );
        assert_eq!(
            profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["powershell", "windows-powershell", "git-bash", "cmd"]
        );
        assert_eq!(Path::new(&profiles[0].executable), portable.join("pwsh.exe"));
        assert_eq!(profiles[2].args, ["--login", "-i"]);
        assert_eq!(profiles[2].env.get("CHERE_INVOKING").map(String::as_str), Some("1"));
        assert_eq!(profiles[2].env.get("MSYSTEM").map(String::as_str), Some("MINGW64"));
        assert!(profiles[2].live_shell().is_some());
        assert!(profiles[3].live_shell().is_some());
        assert!(from_locations(&[], None, None, vec![dir.path().join("missing")]).is_empty());
        let fallback = from_locations(&[], Some(&files), None, vec![]);
        assert_eq!(Path::new(&fallback[0].executable), files.join("PowerShell/7/pwsh.exe"));
    }
}
