//! The Windows known folders behind [`super::ProcessEnv::known_folder`]: `SHGetKnownFolderPath` for the current
//! user, the API Windows documents for a user's profile and application data directories.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;
use std::ptr::null_mut;

use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, FOLDERID_Profile, FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
};
use windows_sys::core::{GUID, PWSTR};

use super::KnownFolder;

/// The current user's `folder`, as the shell resolves it (redirections included); `None` when it does not resolve
/// (no profile loaded, a service account without one).
pub(super) fn known_folder(folder: KnownFolder) -> Option<PathBuf> {
    let id: GUID = match folder {
        KnownFolder::Profile => FOLDERID_Profile,
        KnownFolder::LocalAppData => FOLDERID_LocalAppData,
        KnownFolder::RoamingAppData => FOLDERID_RoamingAppData,
    };
    let mut raw: PWSTR = null_mut();
    // SAFETY: `id` and `raw` outlive the call; a null token asks for the calling user's folder.
    let status =
        unsafe { SHGetKnownFolderPath(&raw const id, KF_FLAG_DEFAULT.cast_unsigned(), null_mut(), &raw mut raw) };
    // SAFETY: the shell allocated `raw` (null on some failures) with CoTaskMemAlloc, it is ours to free on success and
    // on failure alike, and `take_string` reads it only on success.
    let path = unsafe { take_string(raw, status >= 0) };
    path.filter(|path| !path.is_empty()).map(PathBuf::from)
}

/// Copies out `text` when `read` is set, then frees it.
///
/// # Safety
///
/// `text` is null, or a string allocated with `CoTaskMemAlloc` that the caller owns, NUL-terminated when `read` is set.
unsafe fn take_string(text: PWSTR, read: bool) -> Option<OsString> {
    if text.is_null() {
        return None;
    }
    let string = read.then(|| {
        let mut len = 0;
        // SAFETY: the caller guarantees a NUL terminator, so every unit up to it is in bounds.
        while unsafe { *text.wrapping_add(len) } != 0 {
            len += 1;
        }
        // SAFETY: the `len` units before the terminator were just read.
        OsString::from_wide(unsafe { std::slice::from_raw_parts(text, len) })
    });
    // SAFETY: the caller hands the allocation over, and nothing reads it after this.
    unsafe { CoTaskMemFree(text.cast_const().cast()) };
    string
}
