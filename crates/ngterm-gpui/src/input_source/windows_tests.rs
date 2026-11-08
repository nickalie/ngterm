//! Real Windows layouts, including a layout whose KLID is found through the registry's layout id.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayoutNameW, KLF_NOTELLSHELL, LoadKeyboardLayoutW, UnloadKeyboardLayout,
};

use super::*;

const CHILD: &str = "input_source::windows::tests::layouts_child";

#[test]
fn installed_layouts_can_be_named_selected_and_restored() {
    crate::system_window::tests::run_child(CHILD);
}

struct Loaded {
    original: HKL,
    added: Vec<HKL>,
}

impl Loaded {
    fn new() -> Self {
        Self {
            // SAFETY: a query of this thread's layout.
            original: unsafe { GetKeyboardLayout(0) },
            added: Vec::new(),
        }
    }

    fn load(&mut self, id: &str) -> HKL {
        let before = handles();
        // SAFETY: a NUL-terminated system KLID; no activation or shell notification is requested.
        let handle = unsafe { LoadKeyboardLayoutW(wide(id).as_ptr(), KLF_NOTELLSHELL) };
        assert!(!handle.is_null(), "cannot load {id}");
        if !before.contains(&handle) {
            self.added.push(handle);
        }
        handle
    }
}

impl Drop for Loaded {
    fn drop(&mut self) {
        // SAFETY: the original layout is kept loaded until after restoration.
        unsafe { ActivateKeyboardLayout(self.original, KLF_SETFORPROCESS) };
        for handle in &self.added {
            // SAFETY: only layouts loaded by this child, after switching away from them.
            unsafe { UnloadKeyboardLayout(*handle) };
        }
    }
}

fn os_current() -> String {
    let mut name = [0u16; 9];
    // SAFETY: the API requires a buffer of KL_NAMELENGTH (9) UTF-16 units.
    assert_ne!(unsafe { GetKeyboardLayoutNameW(name.as_mut_ptr()) }, 0);
    String::from_utf16(&name[..8]).unwrap()
}

#[test]
#[ignore = "run by installed_layouts_can_be_named_selected_and_restored"]
fn layouts_child() {
    if !crate::system_window::tests::is_child(CHILD) {
        return;
    }
    let mut loaded = Loaded::new();
    let original = os_current();
    let mut backend = KeyboardLayouts;
    for (id, expected_name) in [
        ("00000409", "US"),
        ("00000407", "German"),
        ("00010409", "United States-Dvorak"),
    ] {
        let handle = loaded.load(id);
        assert!(handles().contains(&handle), "the OS must list the loaded layout");
        assert_eq!(klid(handle).as_deref(), Some(id));
        let layouts = backend.layouts().unwrap();
        let layout = layouts.iter().find(|layout| layout.id == id).unwrap();
        assert_eq!(layout.name, expected_name);
        backend.select(id).unwrap();
        assert_eq!(os_current(), id, "selection reaches Windows");
        assert_eq!(backend.current().unwrap(), id);
    }
    assert_eq!(
        backend.select("not-installed").unwrap_err().to_string(),
        "no installed layout not-installed"
    );
    assert_eq!(
        backend.current().unwrap(),
        "00010409",
        "a refused selection leaves the layout alone"
    );
    backend.select(&original).unwrap();
    assert_eq!(os_current(), original);
    assert_eq!(registry_string(LAYOUTS_KEY, "no such value"), None);
    assert_eq!(
        registry_string(&format!(r"{LAYOUTS_KEY}\not-a-layout"), "Layout Text"),
        None
    );
    assert_eq!(klid_of_layout_id(u16::MAX), None);
    assert_eq!(klid(0xE020_0804usize as HKL).as_deref(), Some("E0200804"));
    missing_catalog(&mut backend);
}

/// An unreadable layout catalog leaves ordinary layouts usable by id and reports an unresolved variant explicitly.
fn missing_catalog(backend: &mut KeyboardLayouts) {
    backend.select("00010409").unwrap();
    let _catalog = MissingCatalog::new();
    assert_eq!(
        backend.current().unwrap_err().to_string(),
        "no KLID for the current layout"
    );
    let layouts = backend.layouts().unwrap();
    assert!(layouts.iter().any(|layout| layout.id == "00000409"));
    assert!(layouts.iter().all(|layout| layout.name == layout.id));
    assert_eq!(
        backend.select("00010409").unwrap_err().to_string(),
        "no installed layout 00010409"
    );
    backend.select("00000409").unwrap();
    assert_eq!(os_current(), "00000409");
}

/// A process-local HKLM view backed by an empty disposable HKCU key, never a write to the real system catalog.
struct MissingCatalog {
    path: Vec<u16>,
    root: HKEY,
}

impl MissingCatalog {
    fn new() -> Self {
        use windows_sys::Win32::System::Registry::{
            HKEY_CURRENT_USER, KEY_ALL_ACCESS, RegCreateKeyExW, RegOverridePredefKey,
        };

        let path = wide(&format!(r"Software\ngterm-test-layouts-{}", uuid::Uuid::new_v4()));
        let mut root = std::ptr::null_mut();
        // SAFETY: a unique HKCU path, default security, and a live output handle closed in Drop.
        let created = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_ALL_ACCESS,
                std::ptr::null(),
                &raw mut root,
                std::ptr::null_mut(),
            )
        };
        assert_eq!(created, 0);
        let catalog = Self { path, root };
        // SAFETY: this child's HKLM view only, restored before the key is closed.
        assert_eq!(unsafe { RegOverridePredefKey(HKEY_LOCAL_MACHINE, root) }, 0);
        catalog
    }
}

impl Drop for MissingCatalog {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteTreeW, RegOverridePredefKey};

        // SAFETY: restoring this process's original system catalog view.
        unsafe { RegOverridePredefKey(HKEY_LOCAL_MACHINE, std::ptr::null_mut()) };
        // SAFETY: the owned key, after the override was removed.
        unsafe { RegCloseKey(self.root) };
        // SAFETY: only the UUID-named test subtree under HKCU is deleted.
        assert_eq!(unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, self.path.as_ptr()) }, 0);
    }
}
