//! The command line tool's Windows install: the bundled tool's folder onto the user's `PATH`, the value
//! `HKCU\Environment\Path` that Windows joins after the machine's for every new process of this user. Its type is
//! kept (`REG_EXPAND_SZ` when it holds `%VARIABLES%`, as Windows writes it), the folder is appended once, and Explorer
//! is told (`WM_SETTINGCHANGE` with `"Environment"`), so a terminal started from it afterwards sees the change.

use std::io;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ, REG_VALUE_TYPE, RegCloseKey, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};

use super::{CliPlan, CliResult, Environment};

/// How long the `WM_SETTINGCHANGE` broadcast waits for each window.
const BROADCAST_TIMEOUT_MS: u32 = 2000;

/// The plan: the bundled tool's folder, and whether this process's `PATH` already has it.
pub(super) fn plan(env: &Environment, source: &Path) -> CliPlan {
    let Some(directory) = source.parent().map(Path::to_path_buf) else {
        return CliPlan::Failed(format!("{} has no folder.", source.display()));
    };
    let process_path = env
        .vars
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("PATH"))
        .map(|(_, value)| value.as_str());
    let on_path = process_path.is_some_and(|path| has_entry(path, &directory));
    CliPlan::AddToPath { directory, on_path }
}

/// Whether `path` (`;`-separated) has `directory`, compared without case and trailing separators, as Windows looks
/// folders up.
#[must_use]
pub fn has_entry(path: &str, directory: &Path) -> bool {
    let wanted = normalized(&directory.to_string_lossy());
    path.split(';')
        .any(|entry| !entry.trim().is_empty() && normalized(entry) == wanted)
}

fn normalized(entry: &str) -> String {
    entry
        .trim()
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase()
}

/// `path` with `directory` appended, once.
#[must_use]
pub fn with_entry(path: &str, directory: &Path) -> String {
    if has_entry(path, directory) {
        return path.to_owned();
    }
    let directory = directory.to_string_lossy();
    let path = path.trim_end_matches(';');
    if path.is_empty() {
        directory.into_owned()
    } else {
        format!("{path};{directory}")
    }
}

/// Carries the plan out against `store`: a folder already on the process `PATH` or in the stored value is left; else
/// it is appended to the stored value.
pub(super) fn add_to_path(directory: &Path, on_path: bool, store: &UserPath) -> CliResult {
    let failed =
        |err: io::Error| CliResult::Failed(format!("Could not add {} to your PATH: {err}", directory.display()));
    if on_path {
        return CliResult::OnPath {
            directory: directory.to_path_buf(),
            added: false,
        };
    }
    let (current, kind) = match store.read() {
        Ok(Some((value, kind))) => (value, kind),
        Ok(None) => (String::new(), REG_EXPAND_SZ),
        Err(err) => return failed(err),
    };
    let updated = with_entry(&current, directory);
    let added = updated != current;
    if added && let Err(err) = store.write(&updated, kind) {
        return failed(err);
    }
    CliResult::OnPath {
        directory: directory.to_path_buf(),
        added,
    }
}

/// A `PATH` value under `HKEY_CURRENT_USER`: the user's environment's, or a test's own key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPath {
    key: PathBuf,
    value: String,
    /// Broadcast `WM_SETTINGCHANGE` after a write: only for the real environment.
    announce: bool,
}

impl UserPath {
    /// `HKCU\Environment\Path`.
    #[must_use]
    pub fn environment() -> Self {
        Self {
            key: PathBuf::from("Environment"),
            value: "Path".to_owned(),
            announce: true,
        }
    }

    /// The value `value` of `HKCU\<key>`, announced to no one: for tests over a key of their own.
    #[must_use]
    pub fn at(key: impl Into<PathBuf>, value: &str) -> Self {
        Self {
            key: key.into(),
            value: value.to_owned(),
            announce: false,
        }
    }

    /// The value and its type; `None` when absent.
    ///
    /// # Errors
    ///
    /// The key could not be opened or the value read.
    pub fn read(&self) -> io::Result<Option<(String, REG_VALUE_TYPE)>> {
        let key = Key::open(&self.key, KEY_READ)?;
        let name = wide(&self.value);
        let (mut kind, mut size) = (0, 0u32);
        // SAFETY: `key` is open, `name` is NUL-terminated, and a null data pointer asks only for the size and type.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &raw mut kind,
                std::ptr::null_mut(),
                &raw mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status)?;
        let mut data = vec![0u16; usize::try_from(size).unwrap_or(0) / 2 + 1];
        let mut capacity = u32::try_from(data.len() * 2).unwrap_or(u32::MAX);
        // SAFETY: as above, with `data` a live buffer of `capacity` bytes.
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &raw mut kind,
                data.as_mut_ptr().cast(),
                &raw mut capacity,
            )
        };
        check(status)?;
        if kind != REG_SZ && kind != REG_EXPAND_SZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the PATH value is not a string",
            ));
        }
        let length = usize::try_from(capacity).unwrap_or(0) / 2;
        let text = String::from_utf16_lossy(&data[..length.min(data.len())]);
        Ok(Some((text.trim_end_matches('\0').to_owned(), kind)))
    }

    /// Writes `text` with type `kind`, then announces the change when this is the real environment.
    ///
    /// # Errors
    ///
    /// The key could not be opened or the value written.
    pub fn write(&self, text: &str, kind: REG_VALUE_TYPE) -> io::Result<()> {
        let key = Key::open(&self.key, KEY_WRITE)?;
        let name = wide(&self.value);
        let data = wide(text);
        let bytes = u32::try_from(data.len() * 2).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        // SAFETY: `key` is open, `name` is NUL-terminated, and `data` holds `bytes` bytes, its NUL included.
        check(unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, kind, data.as_ptr().cast(), bytes) })?;
        if self.announce {
            announce_environment();
        }
        Ok(())
    }
}

/// Tells every top-level window (Explorer above all) that the environment changed.
fn announce_environment() {
    let area = wide("Environment");
    let mut result = 0usize;
    // SAFETY: `area` is a NUL-terminated string that outlives the call; the broadcast waits at most the timeout per
    // window and skips hung ones.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            BROADCAST_TIMEOUT_MS,
            &raw mut result,
        )
    };
}

/// An open registry key, closed on drop.
struct Key(HKEY);

impl Key {
    fn open(subkey: &Path, access: u32) -> io::Result<Self> {
        let name = wide(&subkey.to_string_lossy());
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and `key` a live out-parameter.
        check(unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &raw mut key) })?;
        Ok(Self(key))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the key this value opened, closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn check(status: WIN32_ERROR) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(i32::try_from(status).unwrap_or(i32::MAX)))
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::process::Command;

    use super::*;

    fn env(bundled: &Path, path: &str) -> Environment {
        Environment {
            home: PathBuf::from(r"C:\Users\me"),
            agent_status: None,
            skill: None,
            bundled_cli: Some(bundled.to_path_buf()),
            vars: HashMap::from([("Path".to_owned(), path.to_owned())]),
        }
    }

    /// A key of the test's own under `HKCU\Software`, removed at the end.
    struct TestKey(String);

    impl TestKey {
        fn new() -> Self {
            let key = format!(r"Software\ngterm-test-{}", uuid::Uuid::new_v4());
            let made = Command::new("reg")
                .args(["add", &format!(r"HKCU\{key}"), "/f"])
                .output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");
            Self(key)
        }
    }

    impl Drop for TestKey {
        fn drop(&mut self) {
            let _ = Command::new("reg")
                .args(["delete", &format!(r"HKCU\{}", self.0), "/f"])
                .output();
        }
    }

    #[test]
    fn the_entries_compare_as_windows_looks_folders_up() {
        let directory = Path::new(r"C:\Program Files\ngterm\bin");
        assert!(has_entry(r"C:\Windows;c:\program files\NGTERM\bin\;D:\x", directory));
        assert!(has_entry("C:/Program Files/ngterm/bin", directory));
        assert!(!has_entry(r"C:\Windows;C:\Program Files\ngterm", directory));
        assert_eq!(with_entry("", directory), r"C:\Program Files\ngterm\bin");
        assert_eq!(with_entry(r"C:\a;", directory), r"C:\a;C:\Program Files\ngterm\bin");
        assert_eq!(
            with_entry(r"C:\a;C:\Program Files\ngterm\bin", directory),
            r"C:\a;C:\Program Files\ngterm\bin"
        );
    }

    #[test]
    fn the_plan_names_the_tools_folder_and_whether_the_process_path_has_it() {
        let tool = Path::new(r"C:\Program Files\ngterm\bin\ngtermctl.exe");
        let directory = PathBuf::from(r"C:\Program Files\ngterm\bin");
        assert_eq!(
            super::super::plan(&env(tool, r"C:\Windows")),
            CliPlan::AddToPath {
                directory: directory.clone(),
                on_path: false
            }
        );
        assert_eq!(
            super::super::plan(&env(tool, r"C:\Windows;C:\Program Files\ngterm\bin")),
            CliPlan::AddToPath {
                directory,
                on_path: true
            }
        );
    }

    #[test]
    fn the_folder_is_appended_once_keeping_the_value_type() {
        let key = TestKey::new();
        let store = UserPath::at(&key.0, "Path");
        store.write(r"%USERPROFILE%\bin", REG_EXPAND_SZ).unwrap();
        let directory = Path::new(r"C:\Users\me\AppData\Local\ngterm\bin");

        let first = add_to_path(directory, false, &store);
        assert_eq!(
            first,
            CliResult::OnPath {
                directory: directory.to_path_buf(),
                added: true
            }
        );
        assert_eq!(
            store.read().unwrap(),
            Some((format!(r"%USERPROFILE%\bin;{}", directory.display()), REG_EXPAND_SZ))
        );
        assert!(!first.is_warning());
        assert_eq!(first.title(), "Command Line Tool Installed");
        assert!(first.text().contains("was added to your PATH"), "{}", first.text());

        let second = add_to_path(directory, false, &store);
        assert!(matches!(second, CliResult::OnPath { added: false, .. }));
        assert!(second.text().contains("is already on your PATH"), "{}", second.text());
        assert_eq!(store.read().unwrap().unwrap().0.matches("ngterm").count(), 1);
    }

    #[test]
    fn an_absent_value_is_created_and_a_folder_on_the_process_path_writes_nothing() {
        let key = TestKey::new();
        let store = UserPath::at(&key.0, "Path");
        let directory = Path::new(r"C:\ngterm\bin");
        assert_eq!(store.read().unwrap(), None);
        assert!(matches!(
            add_to_path(directory, true, &store),
            CliResult::OnPath { added: false, .. }
        ));
        assert_eq!(store.read().unwrap(), None, "nothing written");
        assert!(matches!(
            add_to_path(directory, false, &store),
            CliResult::OnPath { added: true, .. }
        ));
        assert_eq!(
            store.read().unwrap(),
            Some((r"C:\ngterm\bin".to_owned(), REG_EXPAND_SZ))
        );
    }

    /// .NET's view of `HKCU\<key>\<value>`: its unexpanded data and type, `None` when absent. JSON over UTF-8 keeps
    /// Unicode and whitespace intact regardless of the console's code page.
    fn registry_query(key: &str, value: &str) -> Option<(String, REG_VALUE_TYPE)> {
        let script = r"
            $ErrorActionPreference = 'Stop'
            [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
            $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey($env:TEST_REGISTRY_KEY)
            try {
                $data = $key.GetValue($env:TEST_REGISTRY_VALUE, $null, 'DoNotExpandEnvironmentNames')
                if ($null -eq $data) { 'null' } else {
                    ConvertTo-Json -Compress -InputObject @(
                        [string]$data, [int]$key.GetValueKind($env:TEST_REGISTRY_VALUE)
                    )
                }
            } finally { $key.Dispose() }
        ";
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .env("TEST_REGISTRY_KEY", key)
            .env("TEST_REGISTRY_VALUE", value)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[test]
    fn the_environment_store_reads_the_users_own_path_value() {
        let read = UserPath::environment().read().unwrap();
        assert_eq!(read, registry_query("Environment", "Path"));
    }

    #[test]
    fn the_registry_comparison_keeps_unicode_spaces_and_unexpanded_variables() {
        let key = TestKey::new();
        let store = UserPath::at(&key.0, "Path");
        assert_eq!(registry_query(&key.0, "Path"), None);
        for kind in [REG_SZ, REG_EXPAND_SZ] {
            let text = "  C:\\Users\\René\\日本語;%USERPROFILE%\\bin  ";
            store.write(text, kind).unwrap();
            let expected = Some((text.to_owned(), kind));
            assert_eq!(registry_query(&key.0, "Path"), expected);
            assert_eq!(store.read().unwrap(), expected);
        }
    }

    #[test]
    fn a_path_value_that_is_not_a_string_is_refused_and_left_alone() {
        let key = TestKey::new();
        let made = Command::new("reg")
            .args([
                "add",
                &format!(r"HKCU\{}", key.0),
                "/v",
                "Path",
                "/t",
                "REG_DWORD",
                "/d",
                "1",
                "/f",
            ])
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let store = UserPath::at(&key.0, "Path");
        let error = store.read().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "the PATH value is not a string");
        assert_eq!(
            add_to_path(Path::new(r"C:\ngterm\bin"), false, &store),
            CliResult::Failed(r"Could not add C:\ngterm\bin to your PATH: the PATH value is not a string".to_owned())
        );
        assert_eq!(
            registry_query(&key.0, "Path"),
            Some(("1".to_owned(), windows_sys::Win32::System::Registry::REG_DWORD))
        );
    }

    #[test]
    fn a_value_that_cannot_be_written_fails_the_install() {
        let key = TestKey::new();
        // the key stays readable, and its owner can still delete it
        let deny = format!(
            "$key = 'HKCU:\\{}'; $acl = Get-Acl $key; $user = [Security.Principal.WindowsIdentity]::GetCurrent().User; \
             $acl.AddAccessRule([Security.AccessControl.RegistryAccessRule]::new($user, 'SetValue', 'Deny')); \
             Set-Acl $key $acl",
            key.0
        );
        let denied = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &deny])
            .output()
            .unwrap();
        assert!(denied.status.success(), "{denied:?}");
        let store = UserPath::at(&key.0, "Path");
        let directory = Path::new(r"C:\ngterm\bin");
        assert_eq!(
            add_to_path(directory, false, &store),
            CliResult::Failed(format!(
                r"Could not add C:\ngterm\bin to your PATH: {}",
                io::Error::from_raw_os_error(5)
            ))
        );
        assert_eq!(store.read().unwrap(), None, "nothing written");
    }

    #[test]
    fn a_tool_with_no_folder_cannot_be_put_on_the_path() {
        assert_eq!(
            super::super::plan(&env(Path::new(r"C:\"), r"C:\Windows")),
            CliPlan::Failed(r"C:\ has no folder.".to_owned())
        );
    }

    /// The `lParam` texts of the `WM_SETTINGCHANGE` messages [`settings_window`]'s window received.
    static SETTING_CHANGES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "system" fn record_setting_change(
        window: windows_sys::Win32::Foundation::HWND,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        use windows_sys::Win32::UI::WindowsAndMessaging::{DefWindowProcW, PostQuitMessage, WM_DESTROY};
        if message == WM_SETTINGCHANGE && lparam != 0 {
            let text = lparam as *const u16;
            let mut length = 0;
            // SAFETY: a `WM_SETTINGCHANGE` `lParam`, when not null, is a NUL-terminated string the sender keeps alive
            // for the call: read up to its NUL.
            while unsafe { *text.wrapping_add(length) } != 0 {
                length += 1;
            }
            // SAFETY: the `length` units before that NUL.
            let text = unsafe { std::slice::from_raw_parts(text, length) };
            SETTING_CHANGES.lock().unwrap().push(String::from_utf16_lossy(text));
        }
        if message == WM_DESTROY {
            // SAFETY: a plain call on the window's own thread.
            unsafe { PostQuitMessage(0) };
        }
        // SAFETY: the message as it came.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    /// The broadcast window and its thread, closed and joined even when an assertion unwinds.
    struct SettingsWindow {
        window: usize,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for SettingsWindow {
        fn drop(&mut self) {
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
            // SAFETY: this guard owns the window; its thread ends once it is destroyed.
            let posted = unsafe { PostMessageW(self.window as _, WM_CLOSE, 0, 0) };
            let joined = self.thread.take().unwrap().join();
            if !std::thread::panicking() {
                assert_ne!(posted, 0, "{}", io::Error::last_os_error());
                joined.unwrap();
            }
        }
    }

    /// A hidden top-level window, which a broadcast reaches, recording the setting changes on a thread of its own.
    fn settings_window() -> SettingsWindow {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DispatchMessageW, GWLP_WNDPROC, GetMessageW, MSG, SetWindowLongPtrW, WS_POPUP,
        };
        let (made, window) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let class = wide("STATIC");
            // SAFETY: a system class by NUL-terminated name, no parent, menu or creation data.
            let window = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!window.is_null(), "{}", io::Error::last_os_error());
            // SAFETY: the window this thread made, given a procedure with the window procedure's signature.
            unsafe { SetWindowLongPtrW(window, GWLP_WNDPROC, record_setting_change as *const () as isize as _) };
            made.send(window as usize).unwrap();
            // SAFETY: an all-zero `MSG` is valid.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            // SAFETY: `message` is a live out-parameter; sent messages are dispatched inside the call.
            while unsafe { GetMessageW(&raw mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                // SAFETY: the message GetMessageW filled.
                unsafe { DispatchMessageW(&raw const message) };
            }
        });
        SettingsWindow {
            window: window.recv().unwrap(),
            thread: Some(thread),
        }
    }

    #[test]
    fn a_write_to_the_environment_tells_every_window_and_a_tests_key_tells_none() {
        let key = TestKey::new();
        let _window = settings_window();
        let told = || SETTING_CHANGES.lock().unwrap().iter().any(|area| area == "Environment");

        UserPath::at(&key.0, "Path").write(r"C:\a", REG_SZ).unwrap();
        assert!(!told(), "a test's key announced its write");
        // the real environment's store, over the test's key
        let announcing = UserPath {
            announce: true,
            ..UserPath::at(&key.0, "Path")
        };
        announcing.write(r"C:\a;C:\b", REG_SZ).unwrap();
        // the broadcast is sent: it returned once every window handled it
        assert!(told(), "{:?}", SETTING_CHANGES.lock().unwrap());
        assert_eq!(announcing.read().unwrap(), Some((r"C:\a;C:\b".to_owned(), REG_SZ)));
    }

    #[test]
    fn the_broadcast_window_is_destroyed_when_an_assertion_unwinds() {
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        let window = settings_window();
        let handle = window.window;
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _window = window;
            panic!("an assertion failed while the window was alive");
        }));
        assert!(unwound.is_err());
        // SAFETY: querying a former window handle is valid, including after the window has been destroyed.
        assert_eq!(unsafe { IsWindow(handle as _) }, 0);
    }

    #[test]
    fn a_missing_key_fails_the_install() {
        let store = UserPath::at(format!(r"Software\ngterm-test-absent-{}", uuid::Uuid::new_v4()), "Path");
        let result = add_to_path(Path::new(r"C:\ngterm\bin"), false, &store);
        assert!(
            matches!(&result, CliResult::Failed(message) if message.starts_with("Could not add")),
            "{result:?}"
        );
        assert_eq!(result.title(), "Install Failed");
    }
}
