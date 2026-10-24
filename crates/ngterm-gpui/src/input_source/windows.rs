//! Windows: the installed keyboard layouts (`GetKeyboardLayoutList`), the stored id the KLID (`00000409`, the name
//! `GetKeyboardLayoutNameW` gives the active one), named by the registry's `Layout Text`, selected for the process
//! with `ActivateKeyboardLayout` on the UI thread (the thread GPUI runs this on).
//!
//! A layout handle's high word names its layout: a plain one is the KLID's low word (`0x0409` → `00000409`), an
//! input method's whole handle is its KLID (`E0200804`), and a `0xFxxx` one holds a layout id the registry maps to its
//! KLID (`Keyboard Layouts\<KLID>\Layout Id`).

use std::ffi::c_void;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ, RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ActivateKeyboardLayout, GetKeyboardLayout, GetKeyboardLayoutList, HKL, KLF_SETFORPROCESS,
};

use super::{Backend, Failure, KeyboardLayout};

/// The registry key holding every layout by KLID.
const LAYOUTS_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Keyboard Layouts";

/// The installed keyboard layouts.
pub struct KeyboardLayouts;

/// `text` as a nul-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The installed layouts' handles.
fn handles() -> Vec<HKL> {
    // SAFETY: a null buffer of size 0 asks for the count only.
    let count = unsafe { GetKeyboardLayoutList(0, std::ptr::null_mut()) };
    let mut list: Vec<HKL> = vec![std::ptr::null_mut(); usize::try_from(count).unwrap_or(0)];
    // SAFETY: the buffer holds `count` handles.
    let filled = unsafe { GetKeyboardLayoutList(count, list.as_mut_ptr()) };
    list.truncate(usize::try_from(filled).unwrap_or(0));
    list
}

/// A registry string value, `value` of `key` under `HKEY_LOCAL_MACHINE`.
fn registry_string(key: &str, value: &str) -> Option<String> {
    let (key, value) = (wide(key), wide(value));
    let mut buffer = [0u16; 256];
    let mut size = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;
    // SAFETY: the key and value names are nul-terminated; the buffer and its size in bytes are live locals.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast::<c_void>(),
            &raw mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// The KLID whose `Layout Id` is `layout_id`.
fn klid_of_layout_id(layout_id: u16) -> Option<String> {
    let path = wide(LAYOUTS_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    // SAFETY: the path is nul-terminated; `key` receives the opened key, closed below.
    let opened = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, path.as_ptr(), 0, KEY_READ, &raw mut key) };
    if opened != ERROR_SUCCESS {
        return None;
    }
    let wanted = format!("{layout_id:04x}");
    let mut found = None;
    for index in 0.. {
        let mut name = [0u16; 16];
        let mut length = u32::try_from(name.len()).unwrap_or(0);
        // SAFETY: `key` is open; the name buffer and its length in characters are live locals.
        let status = unsafe {
            RegEnumKeyExW(
                key,
                index,
                name.as_mut_ptr(),
                &raw mut length,
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }
        let klid = String::from_utf16_lossy(&name[..usize::try_from(length).unwrap_or(0).min(name.len())]);
        let id = registry_string(&format!(r"{LAYOUTS_KEY}\{klid}"), "Layout Id");
        if id.is_some_and(|id| id.eq_ignore_ascii_case(&wanted)) {
            found = Some(klid);
            break;
        }
    }
    // SAFETY: `key` was opened above.
    unsafe { RegCloseKey(key) };
    found
}

/// The KLID of layout handle `hkl`.
fn klid(hkl: HKL) -> Option<String> {
    #[expect(clippy::cast_possible_truncation, reason = "a layout handle's value is 32 bits wide")]
    let handle = hkl as usize as u32;
    let high = u16::try_from(handle >> 16).ok()?;
    match high & 0xF000 {
        0xF000 => klid_of_layout_id(high & 0x0FFF),
        0xE000 => Some(format!("{handle:08X}")),
        _ => Some(format!("{high:08X}")),
    }
}

/// The installed layouts with their KLIDs.
fn installed() -> Vec<(HKL, String)> {
    handles()
        .into_iter()
        .filter_map(|hkl| Some((hkl, klid(hkl)?)))
        .collect()
}

impl Backend for KeyboardLayouts {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        Ok(installed()
            .into_iter()
            .map(|(_, id)| {
                let name =
                    registry_string(&format!(r"{LAYOUTS_KEY}\{id}"), "Layout Text").unwrap_or_else(|| id.clone());
                KeyboardLayout { id, name }
            })
            .collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        // SAFETY: thread 0 is the calling thread.
        let hkl = unsafe { GetKeyboardLayout(0) };
        klid(hkl).ok_or_else(|| "no KLID for the current layout".into())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let (hkl, _) = installed()
            .into_iter()
            .find(|(_, klid)| klid == id)
            .ok_or_else(|| format!("no installed layout {id}"))?;
        // SAFETY: `hkl` is an installed layout's handle.
        let previous = unsafe { ActivateKeyboardLayout(hkl, KLF_SETFORPROCESS) };
        if previous.is_null() {
            Err(format!("ActivateKeyboardLayout({id}) failed").into())
        } else {
            Ok(())
        }
    }
}
