//! The scripts the process tests run, one per OS: `/bin/sh -c` on Unix, Windows PowerShell (in every Windows) on
//! Windows, its script passed as `-EncodedCommand` so no command-line quoting touches it.

use base64::Engine as _;

/// The argv running `unix` under `/bin/sh` on Unix, `windows` under PowerShell on Windows.
pub(crate) fn script_argv(unix: &str, windows: &str) -> Vec<String> {
    if cfg!(windows) {
        powershell_argv(windows)
    } else {
        vec!["/bin/sh".to_owned(), "-c".to_owned(), unix.to_owned()]
    }
}

/// PowerShell running `script`: no profile, no prompt, stopping at the first error.
pub(crate) fn powershell_argv(script: &str) -> Vec<String> {
    let script = format!("$ErrorActionPreference = 'Stop'\n{script}");
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    [
        "powershell.exe",
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &base64::engine::general_purpose::STANDARD.encode(utf16),
    ]
    .map(str::to_owned)
    .to_vec()
}
