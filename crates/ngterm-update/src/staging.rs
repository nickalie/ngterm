//! Process-owned stages survive abrupt app exits without leaving uncollectable installation-sized directories.
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Error, Installation};
use serde::{Deserialize, Serialize};

pub(crate) const OWNER: &str = "owner.json";
const RECOVERY: &str = "recovery-required";

#[derive(Serialize, Deserialize)]
struct Owner {
    installation: PathBuf,
    pid: u32,
}

pub(crate) fn create(installation: &Installation, parent: &Path) -> Result<tempfile::TempDir, Error> {
    let work = tempfile::Builder::new().prefix(".ngterm-update-").tempdir_in(parent)?;
    transfer(work.path(), installation, std::process::id())?;
    Ok(work)
}

pub(crate) fn transfer(work: &Path, installation: &Installation, pid: u32) -> Result<(), Error> {
    let owner = Owner {
        installation: dunce::canonicalize(&installation.root)?,
        pid,
    };
    fs::write(work.join(OWNER), serde_json::to_vec(&owner)?)?;
    Ok(())
}

pub(crate) fn cleanup(installation: &Installation) -> Result<usize, Error> {
    let parent = installation
        .root
        .parent()
        .ok_or_else(|| crate::error("Installation has no parent directory."))?;
    let root = dunce::canonicalize(&installation.root)?;
    cleanup_lock(&installation.root);
    let mut removed = 0;
    for entry in fs::read_dir(parent)?.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(".ngterm-update-")
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        let work = entry.path();
        let owner = fs::read(work.join(OWNER))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Owner>(&bytes).ok());
        if let Some(owner) = owner
            && owner.installation == root
            && !process_alive(owner.pid)
            && !work.join(RECOVERY).exists()
            && remove_stage(&work).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

// Keep ownership metadata until every other entry has gone. Windows can retain an executable in the old tree while
// a live-session server still uses it; a later pass must still recognize and clean up that stage.
fn remove_stage(work: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(work)? {
        let entry = entry?;
        if entry.file_name() == OWNER {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::remove_file(work.join(OWNER))?;
    fs::remove_dir(work)
}

fn cleanup_lock(root: &Path) {
    let mut name = root.as_os_str().to_os_string();
    name.push(".ngterm-update-lock");
    let lock = PathBuf::from(name);
    let owner = lock.join("owner.pid");
    if let Some(pid) = fs::read_to_string(&owner)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        && !process_alive(pid)
    {
        let _ = fs::remove_file(owner);
        let _ = fs::remove_dir(lock);
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    if pid <= 0 {
        return true;
    }
    // SAFETY: signal 0 only checks existence/permission and never signals or changes the process.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    if pid == 0 {
        return true;
    }
    // SAFETY: querying a PID without handle inheritance; a null handle is checked before use.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        // SAFETY: reads this thread's last error, immediately after the failed OpenProcess call.
        return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
    }
    // SAFETY: a live owned handle, timeout zero never waits.
    let running = unsafe { WaitForSingleObject(process, 0) } != WAIT_OBJECT_0;
    // SAFETY: release this handle exactly once after its last use.
    unsafe { CloseHandle(process) };
    running
}
