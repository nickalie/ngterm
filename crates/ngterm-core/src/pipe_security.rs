//! Who may use a local named pipe on Windows, for the pipes ngterm serves on (ngterm-session's namespace pipe, the
//! control socket's pipe, the app's instance pipe): the descriptor a server creates its pipe with, and the check a
//! client makes on the process serving the pipe it reached ([`check_server_user`], its refusal and message one for
//! every client).
//!
//! herdr: `src/platform/windows.rs` (`bind_private_local_listener`, `local_server_security_descriptor`) at
//! `HERDR_REV` (`build/pins.env`), licensed under the Apache License 2.0, modified: the descriptor's SDDL alone (the
//! caller builds the descriptor in the type its pipe API takes), and the server-user check, ngterm's addition, moved
//! here from ngterm-session's endpoint so the control socket's and the instance pipe's clients share it.

use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use windows_sys::Win32::Security::{EqualSid, TOKEN_ELEVATION, TOKEN_USER, TokenElevation, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use crate::owner_only::{open_token, token_information};

/// The descriptor of a pipe only its owner may use, in SDDL: full access for SYSTEM and the pipe's owner, no other
/// grant (herdr's private listener DACL), and a mandatory label that keeps lower-integrity processes from reading or
/// writing it: medium, or high for an elevated process, so an unelevated client of the same account cannot reach an
/// elevated server.
///
/// # Errors
///
/// The current process's elevation cannot be read.
pub fn owner_only_pipe_sddl() -> io::Result<String> {
    let integrity = if current_process_is_elevated()? { "HI" } else { "ME" };
    Ok(format!("D:P(A;;GA;;;SY)(A;;GA;;;OW)S:(ML;;NRNW;;;{integrity})"))
}

/// Whether the process `pid` runs as the current process's user: a client's check on the server of a pipe whose name
/// is derived, so anyone could create it first.
///
/// # Errors
///
/// The process cannot be opened (it is gone, or protected) or its user cannot be read.
pub fn runs_as_current_user(pid: u32) -> io::Result<bool> {
    let peer = open_process(pid)?;
    let peer_user = token_information(&open_token(peer.as_raw_handle())?, TokenUser)?;
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing and is always valid.
    let current = unsafe { GetCurrentProcess() };
    let own_user = token_information(&open_token(current)?, TokenUser)?;
    // SAFETY: GetTokenInformation(TokenUser) filled the aligned buffer with a TOKEN_USER at its start.
    let peer_sid = unsafe { &*peer_user.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
    // SAFETY: as above.
    let own_sid = unsafe { &*own_user.as_ptr().cast::<TOKEN_USER>() }.User.Sid;
    // SAFETY: both SIDs point into their buffers, which live until the end of this function.
    Ok(unsafe { EqualSid(peer_sid, own_sid) } != 0)
}

/// Refuses the pipe `pipe` (as the error names it) when the process `server_pid` serving it runs as another user: a
/// client's check on a pipe whose name is derived, so anyone could create it first ([`runs_as_current_user`]).
///
/// # Errors
///
/// `PermissionDenied`, naming the pipe and the server, when the server runs as another user; the check's own error
/// when the server cannot be opened or its user cannot be read.
pub fn check_server_user(pipe: &dyn std::fmt::Display, server_pid: u32) -> io::Result<()> {
    if runs_as_current_user(server_pid)? {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("the pipe at {pipe} belongs to a process of another user (pid {server_pid})"),
    ))
}

fn current_process_is_elevated() -> io::Result<bool> {
    // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing and is always valid.
    let current = unsafe { GetCurrentProcess() };
    let buffer = token_information(&open_token(current)?, TokenElevation)?;
    // SAFETY: GetTokenInformation(TokenElevation) filled the aligned buffer with a TOKEN_ELEVATION.
    let elevation = unsafe { &*buffer.as_ptr().cast::<TOKEN_ELEVATION>() };
    Ok(elevation.TokenIsElevated != 0)
}

fn open_process(pid: u32) -> io::Result<OwnedHandle> {
    // SAFETY: OpenProcess takes no pointers; a null result is checked before the handle is owned.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcess returned a fresh handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipe_descriptor_grants_system_and_the_owner_alone() {
        let sddl = owner_only_pipe_sddl().unwrap();
        assert!(sddl.starts_with("D:P(A;;GA;;;SY)(A;;GA;;;OW)S:(ML;;NRNW;;;"), "{sddl}");
        assert!(sddl.ends_with(";ME)") || sddl.ends_with(";HI)"), "{sddl}");
    }

    #[test]
    fn this_process_runs_as_its_own_user_and_a_missing_one_is_an_error() {
        assert!(runs_as_current_user(std::process::id()).unwrap());
        // the System process runs as SYSTEM, which no test runs as
        if let Ok(same) = runs_as_current_user(4) {
            assert!(!same, "the System process reads as the current user");
        }
        assert!(runs_as_current_user(u32::MAX - 2).is_err());
    }

    #[test]
    fn a_pipe_served_by_another_user_is_refused_by_name() {
        let pipe = r"\\.\pipe\ngterm-test";
        assert!(check_server_user(&pipe, std::process::id()).is_ok());
        // the System process runs as SYSTEM, which no test runs as; it may not be readable at all
        if runs_as_current_user(4).is_ok() {
            let error = check_server_user(&pipe, 4).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(
                error.to_string(),
                r"the pipe at \\.\pipe\ngterm-test belongs to a process of another user (pid 4)"
            );
        }
        let gone = check_server_user(&pipe, u32::MAX - 2).unwrap_err();
        assert_ne!(gone.kind(), io::ErrorKind::PermissionDenied, "{gone}");
    }
}
