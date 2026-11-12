//! A pane's working-directory report as libghostty handed it to the GTK build: [`PaneEvent::Pwd`] carries the raw
//! OSC 7 value, and [`reported_path`] turns it into the path Ghostty's `StreamHandler.reportPwd` (`termio/
//! stream_handler.zig` at `LIBGHOSTTY_VT_REV`) reported as `pwd_change`.
//!
//! - An empty value resets the pwd: the report is the empty path.
//! - Otherwise the value must parse as a `file://` or `kitty-shell-cwd://` URI (the scheme as written, case and all)
//!   whose authority `std.Uri.parseAfterScheme` (Ghostty's pinned Zig, 0.17) parses: it ends at the first `/`, `?`
//!   or `#`, its host follows the last `@`, and a port after the host's last `:` must parse as Zig's
//!   `parseInt(u16, _, 10)` takes it ([`is_port`]), or the report is ignored. The host must be one that
//!   `std.Io.net.HostName.fromUri` accepts: at most [`HOST_MAX_LEN`] bytes as written, then, percent-decoded, a valid
//!   RFC 1123 name
//!   (`HostName.validate`, [`is_valid_host_name`]): dot-separated labels of 1 to 63 ASCII letters, digits and
//!   hyphens, each starting and ending with a letter or digit, at most 253 bytes besides one trailing dot. The host
//!   must then be local: `localhost` or this machine's hostname exactly (`os/hostname.zig` `isLocal`). An empty host
//!   (`file:///tmp`) has no host, so Ghostty ignores it, and so does this. A MAC address host (`12:34:56:78:90:ab`,
//!   which macOS's private Wi-Fi address makes the hostname, and which Ghostty's `mac_address` parse on Linux keeps
//!   whole) is no valid name, so Ghostty ignores it since Zig 0.17's `fromUri` replaced `Uri.getHost`, which
//!   validated nothing.
//! - The path is percent-decoded and ends at a query or fragment, except for `kitty-shell-cwd://`, whose path is
//!   everything after the host, raw (Ghostty's `raw_path`).
//! - On Windows a path naming a drive as a URI spells it, `/C:/Users/me` (RFC 8089's form, which ngterm's
//!   PowerShell integration writes), is reported as Windows spells it, `C:\Users\me` ([`windows_form`]): the path the
//!   app's other working directories (a launch's, a restored one) carry there. Ghostty leaves that to the embedder.
//!   A network path (`//server/share`, `\\server\share`, `\\?\UNC\…`) is ignored there: any program printing to
//!   the pane can send one, and a spawn in it (its `is_dir` check included) would authenticate to that server over
//!   SMB, handing it the user's NTLM hash.
//!
//! [`PaneEvent::Pwd`]: crate::PaneEvent::Pwd

/// The schemes OSC 7 accepts.
const FILE: &str = "file";
const KITTY: &str = "kitty-shell-cwd";

/// The path Ghostty reports for the raw OSC 7 value `raw` on a machine named `hostname`; `None` when Ghostty ignores
/// the report (an invalid URI, another scheme, no host, a host that is not local).
#[must_use]
pub fn reported_path(raw: &str, hostname: &str) -> Option<String> {
    if raw.is_empty() {
        return Some(String::new());
    }
    let (scheme, rest) = raw.split_once("://")?;
    if scheme != FILE && scheme != KITTY {
        tracing::debug!(scheme, "OSC 7 scheme must be file or kitty-shell-cwd");
        return None;
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    let Some(host) = host_of(authority) else {
        tracing::debug!("OSC 7 uri has an invalid port");
        return None;
    };
    if host.len() > HOST_MAX_LEN {
        tracing::debug!("OSC 7 host is too long");
        return None;
    }
    let host = decode(host);
    if !is_valid_host_name(&host) {
        tracing::debug!(host, "OSC 7 uri contains an invalid hostname");
        return None;
    }
    if host != "localhost" && host != hostname {
        tracing::debug!(host, "OSC 7 host must be local");
        return None;
    }
    let path = if scheme == KITTY {
        path.to_owned()
    } else {
        let end = path.find(['?', '#']).unwrap_or(path.len());
        decode(&path[..end])
    };
    if !cfg!(windows) {
        return Some(path);
    }
    let path = windows_form(&path);
    if network_path(&path) {
        tracing::debug!("OSC 7 path names a network share, which Windows would contact");
        return None;
    }
    Some(path)
}

/// Whether Windows reads `path` as a network (UNC) path or a device path: it starts with two separators, either kind.
fn network_path(path: &str) -> bool {
    let mut chars = path.chars();
    chars.next().is_some_and(|c| matches!(c, '/' | '\\')) && chars.next().is_some_and(|c| matches!(c, '/' | '\\'))
}

/// `path` as Windows spells it when it names a drive as a URI does (`/C:/Users/me`, `/C:`): without the leading
/// slash, with `\` separators (`C:\Users\me`, `C:\`); any other path as it is.
#[must_use]
pub fn windows_form(path: &str) -> String {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
        && bytes.get(3).is_none_or(|byte| matches!(byte, b'/' | b'\\'));
    if !drive {
        return path.to_owned();
    }
    let native = path[1..].replace('/', "\\");
    if native.len() == 2 { native + "\\" } else { native }
}

/// This machine's hostname, as `gethostname(2)` gives it (on Windows `GetComputerNameW`'s, the name Ghostty's
/// `isLocal` compares against there).
#[cfg(unix)]
#[must_use]
pub fn local_hostname() -> Option<String> {
    let mut buffer = [0_u8; 256];
    // SAFETY: the buffer is valid for its length, and `gethostname` writes at most that many bytes.
    let status = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if status != 0 {
        return None;
    }
    let len = buffer.iter().position(|byte| *byte == 0).unwrap_or(buffer.len());
    std::str::from_utf8(&buffer[..len]).ok().map(str::to_owned)
}

/// This machine's hostname, as `GetComputerNameW` gives it (Ghostty's `isLocal` on Windows).
#[cfg(windows)]
#[must_use]
pub fn local_hostname() -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;

    let mut buffer = [0_u16; 256];
    let mut len = u32::try_from(buffer.len()).ok()?;
    // SAFETY: the buffer is valid for `len` UTF-16 units, and `GetComputerNameW` writes at most that many (and the
    // name's length, without the NUL, into `len` on success).
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &raw mut len) } == 0 {
        return None;
    }
    let len = usize::try_from(len).ok()?.min(buffer.len());
    String::from_utf16(&buffer[..len]).ok()
}

/// The longest host, in bytes, a URI may carry (`std.Io.net.HostName.max_len`): the longest name's text, with its
/// trailing dot.
const HOST_MAX_LEN: usize = 254;
/// The longest name without that trailing dot.
const HOST_NAME_MAX_LEN: usize = HOST_MAX_LEN - 1;
/// The longest label of a name.
const LABEL_MAX_LEN: usize = 63;

/// The host of a URI's authority as Zig 0.17's `std.Uri.parseAfterScheme` takes it: past any user info (after its
/// last `@`), before a port (after the host's last `:`); `None` when that port is no [`is_port`] (Zig's
/// `error.InvalidPort`). A MAC address host has colons left either way, or no port (Ghostty's `mac_address` retry on
/// Linux keeps it whole), so it stays no valid name.
fn host_of(authority: &str) -> Option<&str> {
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    match host.rsplit_once(':') {
        Some((name, port)) => is_port(port).then_some(name),
        None => Some(host),
    }
}

/// Whether `port` parses as Zig 0.17's `std.fmt.parseInt(u16, port, 10)` has it: an optional `+` or `-`, then decimal
/// digits that `_` may separate but not start or end, at most 65535 (only zero with a `-`). Not empty.
fn is_port(port: &str) -> bool {
    let (negative, digits) = match port.as_bytes().first() {
        Some(b'+') => (false, &port[1..]),
        Some(b'-') => (true, &port[1..]),
        _ => (false, port),
    };
    if digits.is_empty()
        || digits.starts_with('_')
        || digits.ends_with('_')
        || !digits.bytes().all(|byte| byte.is_ascii_digit() || byte == b'_')
    {
        return false;
    }
    let mut value = 0_u32;
    for digit in digits.bytes().filter(u8::is_ascii_digit) {
        value = value * 10 + u32::from(digit - b'0');
        if value > u32::from(u16::MAX) {
            return false;
        }
    }
    !negative || value == 0
}

/// Whether `host` is a valid host name as Zig 0.17's `std.Io.net.HostName.validate` (RFC 1123) has it: not empty, at
/// most [`HOST_NAME_MAX_LEN`] bytes besides one trailing dot, dot-separated labels of 1 to [`LABEL_MAX_LEN`] ASCII
/// letters, digits and hyphens, none starting with a hyphen and each ending with a letter or digit.
fn is_valid_host_name(host: &str) -> bool {
    let name = host.strip_suffix('.').unwrap_or(host);
    !name.is_empty()
        && name.len() <= HOST_NAME_MAX_LEN
        && name.split('.').all(|label| {
            let bytes = label.as_bytes();
            !bytes.is_empty()
                && bytes.len() <= LABEL_MAX_LEN
                && bytes[0] != b'-'
                && bytes[bytes.len() - 1].is_ascii_alphanumeric()
                && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        })
}

/// Percent-decodes `text`; a `%` not followed by two hex digits stays as it is.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(index + 1).and_then(|byte| hex(*byte)),
                bytes.get(index + 2).and_then(|byte| hex(*byte)),
            )
        {
            out.push(high << 4 | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).and_then(|digit| u8::try_from(digit).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "box";

    #[test]
    fn windows_ignores_a_network_share_a_program_reports() {
        for share in [
            "//attacker/share",
            r"\\attacker\share",
            r"/\attacker\share",
            r"\\?\UNC\attacker\share",
        ] {
            assert!(network_path(share), "{share}");
        }
        for local in [r"C:\Users\me", "/home/u", "/", ""] {
            assert!(!network_path(local), "{local}");
        }
        let reported = reported_path("kitty-shell-cwd://localhost//attacker/share", HOST);
        let expected = if cfg!(windows) { None } else { Some("//attacker/share") };
        assert_eq!(reported.as_deref(), expected);
    }

    #[test]
    fn a_local_file_uri_reports_its_decoded_path() {
        assert_eq!(
            reported_path("file://box/home/u/my%20dir", HOST).as_deref(),
            Some("/home/u/my dir")
        );
        assert_eq!(reported_path("file://localhost/tmp", HOST).as_deref(), Some("/tmp"));
        assert_eq!(
            reported_path("file://box/tmp?x=1#top", HOST).as_deref(),
            Some("/tmp"),
            "a query and a fragment end the path"
        );
        // Ghostty compares the scheme as written (`std.mem.eql`, a case-sensitive `startsWith` for kitty's)
        assert_eq!(reported_path("FILE://box/tmp", HOST), None);
        assert_eq!(reported_path("KITTY-SHELL-CWD://box/tmp", HOST), None);
        assert_eq!(
            reported_path("file://user@box:22/srv", HOST).as_deref(),
            Some("/srv"),
            "user info and a port are not the host"
        );
    }

    #[test]
    fn kitty_shell_cwd_keeps_its_path_raw() {
        assert_eq!(
            reported_path("kitty-shell-cwd://box/a%20b?c#d", HOST).as_deref(),
            Some("/a%20b?c#d")
        );
    }

    #[test]
    fn a_drive_spelled_as_a_uri_does_is_spelled_as_windows_does() {
        assert_eq!(windows_form("/C:/Users/me/my dir"), r"C:\Users\me\my dir");
        assert_eq!(windows_form(r"/C:\a b\percent%20#dir"), r"C:\a b\percent%20#dir");
        assert_eq!(windows_form("/c:"), r"c:\");
        assert_eq!(windows_form("/C:/"), r"C:\");
        for other in ["/home/u", "/C", "/CD:/x", "C:/x", "/1:/x", ""] {
            assert_eq!(windows_form(other), other, "{other}");
        }
        let reported = reported_path("kitty-shell-cwd://box/C:/Users/me", HOST);
        let expected = if cfg!(windows) { r"C:\Users\me" } else { "/C:/Users/me" };
        assert_eq!(reported.as_deref(), Some(expected));
        let reported = reported_path(r"kitty-shell-cwd://localhost/C:\a b\percent%20#dir", HOST);
        let expected = if cfg!(windows) {
            r"C:\a b\percent%20#dir"
        } else {
            r"/C:\a b\percent%20#dir"
        };
        assert_eq!(reported.as_deref(), Some(expected));
    }

    #[test]
    fn an_empty_value_resets_the_pwd() {
        assert_eq!(reported_path("", HOST).as_deref(), Some(""));
    }

    #[test]
    fn a_foreign_host_another_scheme_or_no_host_is_ignored() {
        assert_eq!(reported_path("file://elsewhere/tmp", HOST), None);
        assert_eq!(reported_path("file:///tmp", HOST), None, "an empty host is not local");
        assert_eq!(reported_path("http://box/tmp", HOST), None);
        assert_eq!(reported_path("/tmp", HOST), None, "a bare path is no URI");
    }

    #[test]
    fn a_local_name_or_localhost_is_accepted_percent_decoded() {
        assert_eq!(
            reported_path("kitty-shell-cwd://box/tmp", HOST).as_deref(),
            Some("/tmp")
        );
        assert_eq!(
            reported_path("kitty-shell-cwd://localhost/tmp", HOST).as_deref(),
            Some("/tmp")
        );
        assert_eq!(
            reported_path("file://my-box.lan/tmp", "my-box.lan").as_deref(),
            Some("/tmp"),
            "a dotted name with a hyphen"
        );
        assert_eq!(
            reported_path("file://%62%6F%78/tmp", HOST).as_deref(),
            Some("/tmp"),
            "the host is validated and compared once decoded"
        );
        assert_eq!(
            reported_path("file://box./tmp", "box.").as_deref(),
            Some("/tmp"),
            "one trailing dot is a valid name"
        );
        assert_eq!(reported_path("file://box./tmp", HOST), None, "but not this machine's");
        // the longest valid name: 253 bytes, labels of 63
        let longest = [&"a".repeat(63)[..], &"b".repeat(63), &"c".repeat(63), &"d".repeat(61)].join(".");
        assert_eq!(longest.len(), 253);
        assert_eq!(
            reported_path(&format!("file://{longest}/tmp"), &longest).as_deref(),
            Some("/tmp")
        );
        assert_eq!(
            reported_path(&format!("file://{longest}./tmp"), &format!("{longest}.")).as_deref(),
            Some("/tmp")
        );
    }

    /// Zig 0.17's `HostName.fromUri` (in place of `Uri.getHost`, which validated nothing) refuses a host that is no
    /// valid RFC 1123 name, even when it is this machine's hostname: the report is ignored.
    #[test]
    fn an_invalid_or_overlong_host_is_ignored_even_when_it_matches() {
        for host in [
            "my_box",
            "-box",
            "box-",
            "a..b",
            "..",
            "b\u{f6}x",
            "box\u{1b}]52;c;SGVsbG8=\u{7}",
            "[::1]",
        ] {
            assert_eq!(reported_path(&format!("file://{host}/tmp"), host), None, "{host:?}");
        }
        assert_eq!(
            reported_path("file://my%5Fbox/tmp", "my_box"),
            None,
            "a decoded invalid byte"
        );
        let label = "a".repeat(64);
        assert_eq!(
            reported_path(&format!("file://{label}/tmp"), &label),
            None,
            "a 64-byte label"
        );
        let long = [&"a".repeat(63)[..], &"b".repeat(63), &"c".repeat(63), &"d".repeat(62)].join(".");
        assert_eq!(long.len(), 254);
        assert_eq!(
            reported_path(&format!("file://{long}/tmp"), &long),
            None,
            "a 254-byte name"
        );
        // longer than 254 bytes as written, though its decoded name would be short enough
        let encoded = "%61".repeat(85);
        assert_eq!(reported_path(&format!("file://{encoded}/tmp"), &"a".repeat(85)), None);
        assert_eq!(
            reported_path(&format!("file://{}/tmp", "%61".repeat(21)), &"a".repeat(21)).as_deref(),
            Some("/tmp"),
            "a short encoded name"
        );
        // a MAC address host (macOS's private Wi-Fi address as the hostname) has colons, so it is no name
        assert_eq!(reported_path("file://12:34:56:78:90:ab/tmp", HOST), None);
        assert_eq!(
            reported_path("file://12:34:56:78:90:ab/tmp", "12:34:56:78:90:ab"),
            None,
            "a MAC address host is refused even when it is this machine's"
        );
        assert_eq!(
            reported_path("kitty-shell-cwd://12:34:56:78:90:12/tmp", "12:34:56:78:90:12"),
            None
        );
    }

    #[test]
    fn a_missing_or_empty_host_is_ignored() {
        assert_eq!(reported_path("file:///tmp", ""), None);
        assert_eq!(reported_path("kitty-shell-cwd:///tmp", HOST), None);
        assert_eq!(reported_path("file://", HOST), None);
        assert_eq!(reported_path("file://@/tmp", HOST), None, "only user info");
        assert_eq!(
            reported_path("file://me@x@box/tmp", HOST).as_deref(),
            Some("/tmp"),
            "the host follows the last @"
        );
    }

    /// Zig's `parseAfterScheme` ends the authority at the first `/`, `?` or `#`, and refuses a port that is no
    /// `u16` (`error.InvalidPort`, which Ghostty ignores the report for).
    #[test]
    fn the_authority_ends_at_a_path_query_or_fragment_and_its_port_must_be_a_u16() {
        for port in ["22", "0", "65535", "+22", "-0", "2_2", "0_0_065535"] {
            assert_eq!(
                reported_path(&format!("file://box:{port}/tmp"), HOST).as_deref(),
                Some("/tmp"),
                "{port:?}"
            );
        }
        for port in [
            "", "65536", "99999", "-1", "+", "-", "_22", "22_", "0x16", " 22", "2 2", "ssh",
        ] {
            assert_eq!(reported_path(&format!("file://box:{port}/tmp"), HOST), None, "{port:?}");
        }
        assert_eq!(
            reported_path("file://user@box:/tmp", HOST),
            None,
            "an empty port after user info"
        );
        assert_eq!(
            reported_path("file://box?q", HOST).as_deref(),
            Some(""),
            "an empty path before a query"
        );
        assert_eq!(reported_path("file://box#f/x", HOST).as_deref(), Some(""));
        assert_eq!(reported_path("file://box:22?q/x", HOST).as_deref(), Some(""));
        assert_eq!(
            reported_path("kitty-shell-cwd://box?a/b#c", HOST).as_deref(),
            Some("?a/b#c"),
            "kitty's raw path is everything after the host"
        );
        assert_eq!(reported_path("file://box:9?q", HOST).as_deref(), Some(""));
        assert_eq!(reported_path("file://box:99999?q", HOST), None);
    }

    #[test]
    fn this_machine_has_a_hostname() {
        assert!(local_hostname().is_some_and(|name| !name.is_empty()));
    }
}
