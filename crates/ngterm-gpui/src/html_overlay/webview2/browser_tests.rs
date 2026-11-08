//! HTTPS associations in a disposable registry hive, with every launched process reaped.

use super::*;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_ALL_ACCESS, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteTreeW,
    RegOverridePredefKey, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::ASSOCSTR_PROGID;

const CHILD: &str = "html_overlay::webview2::browser::tests::association_child";
const HELPER: &str = "html_overlay::webview2::browser::tests::browser_process";
const CASE: &str = "W_GPUI_HTTPS_CASE";
const HANDSHAKE: &str = "W_GPUI_BROWSER_HANDSHAKE";

#[test]
fn https_associations_launch_the_browser_command_or_fall_back() {
    for scenario in ["command", "missing", "blank", "long", "launch"] {
        let directory = tempfile::tempdir().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") {
                child.env_remove(key);
            }
        }
        let output = child
            .args(["--exact", CHILD, "--ignored", "--nocapture", "--test-threads=1"])
            .env(CASE, scenario)
            .env(HANDSHAKE, directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Only this child sees the test hive as HKCU and HKCR; the user's associations are never written.
struct Association {
    path: Vec<u16>,
    root: HKEY,
    classes: HKEY,
}

fn create_key(parent: HKEY, path: &str) -> HKEY {
    let mut key = std::ptr::null_mut();
    // SAFETY: an open/predefined parent, a NUL-terminated path, and a live output handle closed by the caller.
    let status = unsafe {
        RegCreateKeyExW(
            parent,
            wide(path).as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_ALL_ACCESS,
            std::ptr::null(),
            &raw mut key,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(status, 0);
    key
}

fn string_value(key: HKEY, name: &str, value: &str) {
    let value = wide(value);
    // SAFETY: the open key, NUL-terminated name and UTF-16 value with its byte size.
    let status = unsafe {
        RegSetValueExW(
            key,
            wide(name).as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            u32::try_from(value.len() * 2).unwrap(),
        )
    };
    assert_eq!(status, 0);
}

impl Association {
    fn new(template: Option<&str>) -> Self {
        // load the shell's association factory before hiding its COM registration, and keep the selected ProgID
        let selected = selected_progid();
        let path = format!(r"Software\ngterm-test-https-{}", uuid::Uuid::new_v4());
        let root = create_key(HKEY_CURRENT_USER, &path);
        let classes = create_key(root, r"Software\Classes");
        if let Some(template) = template {
            for id in ["https", selected.as_deref().unwrap_or("https")] {
                let protocol = create_key(classes, id);
                string_value(protocol, "", "URL:ngterm test protocol");
                string_value(protocol, "URL Protocol", "");
                // SAFETY: the key just opened, no longer used.
                unsafe { RegCloseKey(protocol) };
                let command = create_key(classes, &format!(r"{id}\shell\open\command"));
                string_value(command, "", template);
                // SAFETY: the key just opened, no longer used.
                unsafe { RegCloseKey(command) };
            }
        }
        let association = Self {
            path: wide(&path),
            root,
            classes,
        };
        // SAFETY: process-local overrides to keys kept open until both overrides are undone in Drop.
        assert_eq!(unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, root) }, 0);
        // SAFETY: as above, for the classes view this process's shell queries.
        assert_eq!(unsafe { RegOverridePredefKey(HKEY_CLASSES_ROOT, classes) }, 0);
        association
    }
}

fn selected_progid() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut length = u32::try_from(buffer.len()).unwrap();
    // SAFETY: NUL-terminated protocol/verb, and a writable buffer with its capacity in UTF-16 units.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_NOTRUNCATE,
            ASSOCSTR_PROGID,
            wide("https").as_ptr(),
            wide("open").as_ptr(),
            buffer.as_mut_ptr(),
            &raw mut length,
        )
    };
    (result == 0).then(|| String::from_utf16_lossy(&buffer[..usize::try_from(length).unwrap() - 1]))
}

impl Drop for Association {
    fn drop(&mut self) {
        // SAFETY: restoring the process-local predefined views, before closing their backing keys.
        unsafe { RegOverridePredefKey(HKEY_CLASSES_ROOT, std::ptr::null_mut()) };
        // SAFETY: restoring HKCU before deleting the test key through the original HKCU.
        unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, std::ptr::null_mut()) };
        // SAFETY: owned handles, each closed once after undoing the overrides.
        unsafe { RegCloseKey(self.classes) };
        // SAFETY: as above, for the root handle.
        unsafe { RegCloseKey(self.root) };
        // SAFETY: only the UUID-named test subtree under the real HKCU is removed.
        assert_eq!(unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, self.path.as_ptr()) }, 0);
    }
}

#[test]
#[ignore = "run by https_associations_launch_the_browser_command_or_fall_back"]
fn association_child() {
    let Ok(scenario) = std::env::var(CASE) else { return };
    let directory = PathBuf::from(std::env::var_os(HANDSHAKE).unwrap());
    let missing = directory.join("no-such-program.exe").to_string_lossy().into_owned();
    if scenario == "launch" {
        launch_and_reap(&directory);
        return;
    }
    let template = match scenario.as_str() {
        "command" => Some(r#""C:\browser test\reader.exe" --single-argument "%L""#.to_owned()),
        "blank" => Some("   ".to_owned()),
        "long" => Some(format!(r#""C:\{}.exe" "%1""#, "x".repeat(2200))),
        "missing" => None,
        _ => panic!("unknown scenario {scenario}"),
    };
    let _association = Association::new(template.as_deref());
    if scenario == "command" {
        assert_eq!(https_command(), template);
        let uri = "file:///C:/日本語/page%20one.html";
        assert!(open_with(uri, |program, args| {
            assert_eq!(program, r"C:\browser test\reader.exe");
            assert_eq!(args, ["--single-argument", uri]);
            Ok(())
        }));
        assert!(
            !open_with(&missing, |program, args| {
                assert_eq!(program, r"C:\browser test\reader.exe");
                assert_eq!(args, ["--single-argument", &missing]);
                Err(std::io::Error::from_raw_os_error(2))
            }),
            "a failed spawn and failed shell hand-off report failure"
        );
    } else {
        assert_eq!(https_command(), None, "no usable https command for {scenario}");
        assert!(!open_with(&missing, |_, _| panic!(
            "an absent command must not be spawned"
        )));
    }
}

struct Process(HANDLE);

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: a test helper handle with termination rights; harmless if it has already exited.
        unsafe { TerminateProcess(self.0, 1) };
        // SAFETY: the owned process handle remains valid until closed below.
        unsafe { WaitForSingleObject(self.0, 5000) };
        // SAFETY: the handle opened by launch_and_reap, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

fn wait_for_file(path: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        assert!(Instant::now() < deadline, "{} was not written", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn launch_and_reap(directory: &Path) {
    let executable = std::env::current_exe().unwrap();
    let template = format!(
        r#""{}" --exact {HELPER} --ignored --nocapture --test-threads=1"#,
        executable.display()
    );
    let _association = Association::new(Some(&template));
    assert_eq!(https_command().as_deref(), Some(template.as_str()));
    let uri = directory.join("no-such-program.exe").to_string_lossy().into_owned();
    assert!(open(&uri));
    let pid: u32 = wait_for_file(&directory.join("pid")).parse().unwrap();
    // SAFETY: the child wrote its own pid and waits for release; waiting/querying/cleanup rights only.
    let handle = unsafe {
        OpenProcess(
            PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
            0,
            pid,
        )
    };
    assert!(!handle.is_null());
    let process = Process(handle);
    std::fs::write(directory.join("release"), "done").unwrap();
    // SAFETY: a live owned process handle, bounded wait for this helper alone.
    assert_eq!(unsafe { WaitForSingleObject(process.0, 15_000) }, WAIT_OBJECT_0);
    let mut code = u32::MAX;
    // SAFETY: the process handle and a live exit-code output.
    assert_ne!(unsafe { GetExitCodeProcess(process.0, &raw mut code) }, 0);
    assert_eq!(code, 0, "the associated program ran and exited successfully");
}

#[test]
#[ignore = "launched as the test https handler by launch_and_reap"]
fn browser_process() {
    let Some(directory) = std::env::var_os(HANDSHAKE).map(PathBuf::from) else {
        return;
    };
    std::fs::write(directory.join("pid"), std::process::id().to_string()).unwrap();
    assert_eq!(wait_for_file(&directory.join("release")), "done");
}

#[test]
fn placeholders_and_windows_argument_quoting_preserve_the_address() {
    for placeholder in ["%1", "%L", "%l"] {
        let template = format!(r#""C:\browser app\read.exe" --url "{placeholder}" {placeholder}"#);
        let uri = "https://example.com/a b?q=日本語";
        let line = command_line(&template, uri);
        assert_eq!(
            split(&line).unwrap(),
            [r"C:\browser app\read.exe", "--url", uri, placeholder]
        );
    }
    let uri = "file:///C:/a b/page.html";
    let line = command_line(r#""C:\browser app\read.exe" --new-window"#, uri);
    assert_eq!(split(&line).unwrap(), [r"C:\browser app\read.exe", "--new-window", uri]);
    assert_eq!(
        split(r#"reader.exe "" "a\"b" "c\\""#).unwrap(),
        ["reader.exe", "", "a\"b", "c\\"]
    );
}

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
