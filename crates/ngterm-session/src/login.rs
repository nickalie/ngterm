//! The login session a server runs in, so a client tells a server left from an earlier login from its own (upstream
//! agterm's `HostIdentity.endpointKey`, whose audit session id part this is; `agtermCore/Sources/SessionHostRuntime/
//! HostIdentity.swift` at v0.42.3).
//!
//! On macOS a server started in one GUI login can outlive it (a forced logout ends the app, not the detached server),
//! and the sessions it creates after that fail in the dead login (upstream's #971: `No user exists for uid 501`). The
//! login is the audit session id (`getaudit_addr`'s `ai_asid`) of a GUI login, which every process of one login shares
//! and a new login does not. A process outside a GUI login (an ssh login, which on macOS 27 shares one audit session
//! without graphic access, measured) reports none, so an attach over ssh never takes the GUI login's server for an
//! earlier one. Linux and Windows have no such failure, and report no login: a server there is never from an earlier
//! one.

/// This process's login session: the audit session id of the GUI login it runs in on macOS; `None` outside a GUI login,
/// on another OS, or when it cannot be read.
#[must_use]
pub fn current() -> Option<i64> {
    sys::audit_session()
}

/// Whether a server of login `server` is from an earlier login than `own`: never when either is unknown.
#[must_use]
pub fn is_earlier(server: Option<i64>, own: Option<i64>) -> bool {
    matches!((server, own), (Some(server), Some(own)) if server != own)
}

#[cfg(target_os = "macos")]
mod sys {
    use std::ffi::c_int;
    use std::mem::{offset_of, size_of};

    /// `<bsm/audit.h>`'s `struct auditinfo_addr`.
    #[repr(C)]
    #[derive(Default)]
    struct AuditinfoAddr {
        ai_auid: u32,
        ai_mask: [u32; 2],
        at_port: i32,
        at_type: u32,
        at_addr: [u32; 4],
        ai_asid: i32,
        ai_flags: u64,
    }

    const _: () = assert!(size_of::<AuditinfoAddr>() == 48);
    const _: () = assert!(offset_of!(AuditinfoAddr, ai_asid) == 36);

    /// `<bsm/audit_session.h>`'s `AU_SESSION_FLAG_HAS_GRAPHIC_ACCESS`: the session of a GUI login.
    const HAS_GRAPHIC_ACCESS: u64 = 0x0010;

    unsafe extern "C" {
        fn getaudit_addr(info: *mut AuditinfoAddr, length: c_int) -> c_int;
    }

    pub(super) fn audit_session() -> Option<i64> {
        let mut info = AuditinfoAddr::default();
        let length = c_int::try_from(size_of::<AuditinfoAddr>()).ok()?;
        // SAFETY: `info` is a writable `auditinfo_addr` of `length` bytes.
        let read = unsafe { getaudit_addr(&raw mut info, length) };
        (read == 0)
            .then_some(info)
            .and_then(|info| graphic(info.ai_flags, info.ai_asid))
    }

    pub(super) fn graphic(flags: u64, asid: i32) -> Option<i64> {
        (flags & HAS_GRAPHIC_ACCESS != 0).then_some(i64::from(asid))
    }
}

#[cfg(not(target_os = "macos"))]
mod sys {
    pub(super) fn audit_session() -> Option<i64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_two_known_and_different_logins_make_a_server_earlier() {
        assert!(is_earlier(Some(100_001), Some(100_002)));
        assert!(!is_earlier(Some(100_001), Some(100_001)));
        assert!(!is_earlier(None, Some(100_001)));
        assert!(!is_earlier(Some(100_001), None));
        assert!(!is_earlier(None, None));
    }

    #[test]
    fn every_call_of_one_process_reads_one_login_and_none_off_macos() {
        let login = current();
        assert_eq!(current(), login);
        if cfg!(not(target_os = "macos")) {
            assert_eq!(login, None);
        }
    }

    /// The flags measured on macOS 27: a GUI login's `0x2030`, an ssh login's `0x1`.
    #[cfg(target_os = "macos")]
    #[test]
    fn only_a_session_with_graphic_access_is_a_login() {
        assert_eq!(sys::graphic(0x2030, 100_014), Some(100_014));
        assert_eq!(sys::graphic(0x1, 100_013), None);
    }
}
