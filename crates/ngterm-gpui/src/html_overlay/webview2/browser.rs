//! The hand-off to the default web browser (upstream's `SystemBrowser.open`, the WPE host's `gio` default for
//! `https`): the address opens in the program the `https` protocol is associated with whatever its type, so a file
//! page shows as a page rather than in the editor `.html` maps to. Where that association names no command (a packaged
//! browser registered through its app id), the shell opens the address by its own type.

use std::process::{Command, Stdio};

use windows_sys::Win32::UI::Shell::{
    ASSOCF_IS_PROTOCOL, ASSOCF_NOTRUNCATE, ASSOCSTR_COMMAND, AssocQueryStringW, CommandLineToArgvW, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Opens `uri` in the default web browser; false when there is none.
#[must_use]
pub fn open(uri: &str) -> bool {
    if let Some(argv) = https_command()
        .map(|template| command_line(&template, uri))
        .and_then(|line| split(&line))
        && let Some((program, args)) = argv.split_first()
    {
        let spawned = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(_) => return true,
            Err(err) => tracing::warn!(%err, program, "the web browser did not start"),
        }
    }
    shell_open(uri)
}

/// The command the `https` protocol opens with, as the registry spells it (`"…\msedge.exe" --single-argument %1`).
fn https_command() -> Option<String> {
    let protocol = wide("https");
    let verb = wide("open");
    let mut buffer = vec![0u16; 2048];
    let mut length = u32::try_from(buffer.len()).ok()?;
    // SAFETY: NUL-terminated strings and a buffer of `length` UTF-16 units.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_NOTRUNCATE,
            ASSOCSTR_COMMAND,
            protocol.as_ptr(),
            verb.as_ptr(),
            buffer.as_mut_ptr(),
            &raw mut length,
        )
    };
    if result != 0 {
        return None;
    }
    let text = String::from_utf16_lossy(&buffer[..usize::try_from(length).ok()?.saturating_sub(1)]);
    (!text.trim().is_empty()).then_some(text)
}

/// `template` with `uri` in place of its address placeholder (`%1`, `%L`), or after it when it has none.
#[must_use]
pub fn command_line(template: &str, uri: &str) -> String {
    for placeholder in ["%1", "%L", "%l"] {
        if template.contains(placeholder) {
            return template.replacen(placeholder, uri, 1);
        }
    }
    format!("{template} \"{uri}\"")
}

/// A command line split as programs read theirs (`CommandLineToArgvW`).
fn split(line: &str) -> Option<Vec<String>> {
    let line = wide(line);
    let mut count = 0;
    // SAFETY: a NUL-terminated string; the array the call allocates is read `count` long and freed with `LocalFree`.
    unsafe {
        let argv = CommandLineToArgvW(line.as_ptr(), &raw mut count);
        if argv.is_null() {
            return None;
        }
        let args = (0..usize::try_from(count).ok()?)
            .map(|index| {
                let arg = *argv.add(index);
                let length = (0..).take_while(|&offset| *arg.add(offset) != 0).count();
                String::from_utf16_lossy(std::slice::from_raw_parts(arg, length))
            })
            .collect();
        windows_sys::Win32::Foundation::LocalFree(argv.cast());
        Some(args)
    }
}

/// The shell's open of `uri` by its own type.
fn shell_open(uri: &str) -> bool {
    let verb = wide("open");
    let file = wide(uri);
    // SAFETY: NUL-terminated strings, no parent window, no parameters or directory.
    let instance = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // a value above 32 is success
    instance as usize > 32
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_takes_the_placeholder_or_goes_last() {
        let edge = r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --single-argument %1"#;
        let line = command_line(edge, "file:///C:/site/page.html");
        assert!(line.ends_with("--single-argument file:///C:/site/page.html"), "{line}");
        assert_eq!(
            split(&line).unwrap(),
            [
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                "--single-argument",
                "file:///C:/site/page.html"
            ]
        );
        assert_eq!(
            command_line(r"C:\b\browser.exe", "https://example.com/"),
            r#"C:\b\browser.exe "https://example.com/""#
        );
    }
}
