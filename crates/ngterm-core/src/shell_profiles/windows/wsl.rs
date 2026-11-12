//! Read registered distributions without starting WSL, a console, or a distribution.

use std::path::Path;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};

use crate::shell_profiles::ShellProfile;

const LXSS: &str = r"Software\Microsoft\Windows\CurrentVersion\Lxss";

pub(super) fn discover(system: &Path) -> Vec<ShellProfile> {
    let executable = system.join("wsl.exe");
    if !executable.is_file() {
        return Vec::new();
    }
    profiles(&executable.to_string_lossy(), distributions())
}

fn profiles(executable: &str, mut names: Vec<String>) -> Vec<ShellProfile> {
    names.retain(|name| !name.is_empty() && !name.contains(['\0', '\r', '\n']));
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            ShellProfile::new(
                &format!("wsl:{name}"),
                &format!("WSL: {name}"),
                executable.to_owned(),
                &["--distribution", &name],
            )
        })
        .collect()
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

fn distributions() -> Vec<String> {
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: predefined hive, NUL-terminated name, read-only access, valid output pointer.
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide(LXSS).as_ptr(), 0, KEY_READ, &raw mut key) } != 0 {
        return Vec::new();
    }
    let mut names = Vec::new();
    let mut buffer = vec![0u16; 16384];
    for index in 0.. {
        let mut length = 16384u32;
        // SAFETY: live read-only key and a writable buffer; unused optional outputs are null.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                buffer.as_mut_ptr(),
                &raw mut length,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            break;
        }
        if let Ok(child) = String::from_utf16(&buffer[..length as usize])
            && let Some(name) = distribution_name(key, &child)
        {
            names.push(name);
        }
    }
    // SAFETY: exactly this function's successful RegOpenKeyExW result, closed once after enumeration.
    unsafe { RegCloseKey(key) };
    names
}

fn distribution_name(key: HKEY, child: &str) -> Option<String> {
    let mut value = vec![0u16; 32768];
    let mut bytes = u32::try_from(std::mem::size_of_val(value.as_slice())).ok()?;
    // SAFETY: live key, NUL-terminated names, writable buffer and its size in bytes.
    let status = unsafe {
        RegGetValueW(
            key,
            wide(child).as_ptr(),
            wide("DistributionName").as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &raw mut bytes,
        )
    };
    if status != 0 {
        return None;
    }
    let end = value.iter().position(|unit| *unit == 0)?;
    String::from_utf16(&value[..end]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_wsl_profiles_keep_distribution_names_as_single_arguments() {
        let names = ["Ubuntu Dev", "Debian", "Ubuntu Dev", "", "bad\nname"]
            .map(str::to_owned)
            .to_vec();
        let result = profiles("C:/Windows/System32/wsl.exe", names);
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].id, "wsl:Ubuntu Dev");
        assert_eq!(result[1].args, ["--distribution", "Ubuntu Dev"]);
        assert!(result.iter().all(|profile| profile.live_shell().is_some()));
    }
}
