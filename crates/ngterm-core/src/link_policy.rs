//! Which link clicks a terminal opens and how (upstream's `LinkPolicy.swift`).
//!
//! A terminal renders untrusted program output, so an escape-sequence link (`GHOSTTY_ACTION_OPEN_URL`) can carry any
//! scheme. [`disposition`] maps a raw link to opening a web or mail URL with the desktop's handler, revealing a
//! local path in the file manager, showing bounded raster/Markdown documents, or ignoring it. Local raster and
//! Markdown paths, bare or in a `file://` URL, become document overlays; other local files are only revealed, never
//! opened in their default application (which could execute a script or `.desktop` file). A `file://` whose host is
//! not this machine is ignored, as are network/device paths and automount roots, where even looking at the path can
//! mount a network share. [`route`] adds the link-open setting on top: a
//! web link clicked in a pane or the scratch terminal may go to a session overlay (full size, 95%, 75% or 50%)
//! instead of the browser, and [`click_intent`] reads a click's chord: Ctrl/Super+Shift overrides that for one link,
//! Ctrl/Super+Alt reveals the file a local route names. A local source, config, text or log file becomes a read-only
//! text page ([`crate::html::text_page`]); a link ending in `:line[:column]`, or a relative one, names candidates
//! ([`locations`]) only the file system can choose between, which the frontend stats off its main thread.
//!
//! Host-free: the local host names are injected (the `*_for_hosts` variants), and the frontend only carries out the
//! route. Absolute and `~/` bare paths retain spaces, `%`, `?` and `#` literally; relative bare paths and control
//! characters are ignored. URLs are parsed here as RFC 3986 references, strictly (spaces, controls and broken
//! percent-escapes are invalid), in place of Foundation's `URL(string:)`.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::OnceLock;

use uuid::Uuid;

use crate::persist::settings::LinkOpenMode;

mod locations;

pub use locations::{
    ClickIntent, LinkLocation, click_intent, locations, locations_for_hosts_and_home, route_for_location,
};

/// The schemes safe to hand to the desktop's opener: web and mail only, none that hands off to a local executable or
/// handler.
pub const PERMITTED_SCHEMES: [&str; 4] = ["http", "https", "mailto", "ftp"];

/// Raster extensions routed to a bounded image overlay after the path guards.
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];
/// Text extensions routed to a bounded Markdown overlay after the path guards.
pub const MARKDOWN_EXTENSIONS: [&str; 2] = ["md", "markdown"];

/// The automount roots where revealing a path can mount a network share: autofs's `/net` (the `-hosts` map) and
/// `/misc` (`auto.misc`). Upstream's macOS list (`/Network`, `auto_home`'s `/home` and their `/System/Volumes/Data`
/// twins) names no automount on Linux, where `/home` holds the real home directories.
#[cfg(not(target_os = "macos"))]
const AUTOMOUNT_ROOTS: [&str; 2] = ["/net", "/misc"];
#[cfg(target_os = "macos")]
const AUTOMOUNT_ROOTS: [&str; 7] = [
    "/net",
    "/misc",
    "/network",
    "/home",
    "/system/volumes/data/home",
    "/system/volumes/data/net",
    "/system/volumes/data/network/servers",
];

/// What a link click does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkDisposition {
    /// Open the link, as given, with the desktop's handler for its scheme.
    Open(String),
    /// Reveal this local path, host-stripped and dot-normalized, in the file manager.
    Reveal(PathBuf),
    /// A local raster path, probed before it can occupy a document overlay.
    Image(PathBuf),
    /// A local Markdown path, checked before it can occupy a document overlay.
    Markdown(PathBuf),
    /// A local HTML file ([`crate::html::document_page::EXTENSIONS`]), read and checked before it can occupy a document
    /// overlay, where it shows with no file grant, bridge or page JavaScript.
    Document(PathBuf),
    /// A local file a text page shows ([`crate::html::text_page::is_eligible`]), at `line` when a `:line` suffix gave
    /// one, checked before it can occupy a document overlay.
    Text {
        path: PathBuf,
        line: Option<i64>,
    },
    Ignore,
}

/// Where a clicked link came from. Only a pane and the scratch terminal have an owning session a page could open on;
/// a HUD link stays in the browser so the click never replaces the HUD it sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickOrigin {
    Pane(Uuid),
    /// An attached primary or split pane; its printed paths belong to the origin.
    RemotePane(Uuid),
    Scratch(Uuid),
    Hud,
    ProgramOverlay,
    Quick,
}

/// What a link click does once the link-open setting applies. `Browser` is the desktop's opener, whatever handler the
/// scheme maps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Browser(String),
    /// A session web overlay over `session`, at `size_percent` of it (`None`: full size).
    Overlay {
        url: String,
        session: Uuid,
        size_percent: Option<i64>,
    },
    /// A local raster document at its natural size, with reveal as the refused-open fallback.
    Image {
        path: PathBuf,
        session: Uuid,
    },
    /// A local Markdown document, 95% in browser mode and the configured overlay size otherwise.
    Markdown {
        path: PathBuf,
        session: Uuid,
        size_percent: Option<i64>,
    },
    /// A local HTML document, sized as a Markdown document; a refused open reveals the file.
    Document {
        path: PathBuf,
        session: Uuid,
        size_percent: Option<i64>,
    },
    /// A local text page marked at `line`, sized as a Markdown document; a refused open reveals the file.
    Text {
        path: PathBuf,
        line: Option<i64>,
        session: Uuid,
        size_percent: Option<i64>,
    },
    Reveal(PathBuf),
    Ignore,
}

impl Route {
    /// The file a local route names, which the reveal chord selects in the file manager instead; `None` for a web
    /// route and an ignored link (upstream's `localFile`).
    #[must_use]
    pub fn local_file(&self) -> Option<&std::path::Path> {
        match self {
            Self::Image { path, .. }
            | Self::Markdown { path, .. }
            | Self::Document { path, .. }
            | Self::Text { path, .. }
            | Self::Reveal(path) => Some(path),
            Self::Browser(_) | Self::Overlay { .. } | Self::Ignore => None,
        }
    }

    /// Whether the click opens a log page following the file (upstream's `TextLink.opensAsLog`): a plain click on a
    /// `.log` or `.out` text page; a `path:line` click opens the static page at its line.
    #[must_use]
    pub fn opens_as_log(&self) -> bool {
        matches!(self, Self::Text { path, line: None, .. } if crate::html::text_page::is_log(&path.to_string_lossy()))
    }
}

/// The lowercased host names counting as this machine for a `file://` link: `localhost` and the kernel's host name
/// (what GNU `ls --hyperlink` emits as `file://<host>/…`; `eza` leaves the host empty, which always counts), with the
/// `.local`-stripped short forms. Read once, from `/proc` (no resolver, no network).
#[must_use]
pub fn local_host_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let hostname = ["/proc/sys/kernel/hostname", "/etc/hostname"]
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .map(|name| name.trim().to_owned())
            .unwrap_or_default();
        expanded_host_names(["localhost", hostname.as_str()])
    })
}

/// Normalizes each raw host name and adds the `.local`-stripped short form beside the full one; empty names (and a
/// bare `.local`'s empty short form) are skipped.
#[must_use]
pub fn expanded_host_names<'a>(raw: impl IntoIterator<Item = &'a str>) -> HashSet<String> {
    let mut out = HashSet::new();
    for name in raw {
        let normalized = normalized_host(name);
        if normalized.is_empty() {
            continue;
        }
        if let Some(short) = normalized.strip_suffix(".local")
            && !short.is_empty()
        {
            out.insert(short.to_owned());
        }
        out.insert(normalized);
    }
    out
}

/// A host lowercased, without a trailing FQDN dot, so matching is stable.
#[must_use]
pub fn normalized_host(host: &str) -> String {
    let lower = host.to_lowercase();
    match lower.strip_suffix('.') {
        Some(stripped) => stripped.to_owned(),
        None => lower,
    }
}

/// [`disposition_for_hosts`] with this machine's names.
#[must_use]
pub fn disposition(raw: &str) -> LinkDisposition {
    disposition_for_hosts(raw, local_host_names())
}

/// Maps a raw terminal link to what a click does: permitted web/mail schemes open; absolute and `~/` bare paths
/// are literal, while local `file://` paths are decoded and dot-normalized. Raster/Markdown paths become bounded
/// document overlays and other local paths are revealed. Relative bare paths, non-local file hosts, network/device
/// paths, automount roots (checked after `..` normalization), other schemes and malformed URLs are ignored.
#[must_use]
pub fn disposition_for_hosts<S: BuildHasher>(raw: &str, local_hosts: &HashSet<String, S>) -> LinkDisposition {
    let home = crate::paths::home_dir(&crate::paths::ProcessEnv).unwrap_or_default();
    disposition_for_hosts_and_home(raw, local_hosts, &home.to_string_lossy())
}

/// Classifies local paths with an injected home; no filesystem access occurs before the automount guard.
#[must_use]
pub fn disposition_for_hosts_and_home<S: BuildHasher>(
    raw: &str,
    local_hosts: &HashSet<String, S>,
    home: &str,
) -> LinkDisposition {
    if is_bare_path(raw) {
        if raw.chars().any(char::is_control) {
            return LinkDisposition::Ignore;
        }
        let expanded = raw
            .strip_prefix("~/")
            .map_or_else(|| raw.to_owned(), |rest| format!("{home}/{rest}"));
        return local_path(&expanded, None);
    }
    let Some(link) = Link::parse(raw) else {
        return LinkDisposition::Ignore;
    };
    if PERMITTED_SCHEMES.contains(&link.scheme.as_str()) {
        return LinkDisposition::Open(raw.to_owned());
    }
    if link.scheme != "file" {
        return LinkDisposition::Ignore;
    }
    local_file_path(&link, local_hosts).map_or(LinkDisposition::Ignore, |path| local_path(&path, None))
}

/// The decoded local path a `file://` link names, `None` for a host that is not this machine or a path that does not
/// decode.
fn local_file_path<S: BuildHasher>(link: &Link<'_>, local_hosts: &HashSet<String, S>) -> Option<String> {
    let host = normalized_host(link.host.as_deref().unwrap_or_default());
    if !host.is_empty() && !local_hosts.contains(&host) {
        return None;
    }
    // decoded first, so an encoded slash or dot segment is seen for what it is
    let path = percent_decode(link.path)?;
    // A Windows drive in a file URL is preceded by URL's root slash; the native path is C:/…, not \C:….
    #[cfg(windows)]
    let path = if path.as_bytes().get(2) == Some(&b':')
        && path.as_bytes().get(1).is_some_and(u8::is_ascii_alphabetic)
        && path.starts_with('/')
    {
        path[1..].to_owned()
    } else {
        path
    };
    Some(path)
}

/// Bare paths detected in terminal output; remote panes never resolve them locally.
#[must_use]
pub fn is_bare_path(raw: &str) -> bool {
    crate::html::is_absolute(raw) || raw.starts_with("~/")
}

/// `path` lexically normalized, `None` for a relative, network, device or automount path, or one holding a control
/// character: a decoded `%00` would make the open read the name before it (upstream's `guardedLocalPath`). Nothing
/// here touches the file system.
fn guarded_local_path(path: &str) -> Option<String> {
    if !(crate::html::is_absolute(path) || path.starts_with('/'))
        || network_or_device_path(path)
        || path.contains('\0')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    let normalized = if cfg!(windows) {
        crate::html::normalize_path(path)
    } else {
        lexically_normalized_absolute_path(path)
    };
    (!is_automount_path(&normalized.replace('\\', "/"))).then_some(normalized)
}

/// Classifies a decoded local path (upstream's `localPath`): an image, a Markdown document, an HTML document, a text
/// page at `line`, or a reveal; ignored when [`guarded_local_path`] refuses it. HTML goes ahead of the text check.
fn local_path(path: &str, line: Option<i64>) -> LinkDisposition {
    let Some(normalized) = guarded_local_path(path) else {
        return LinkDisposition::Ignore;
    };
    let text = crate::html::text_page::is_eligible(&normalized);
    let path = PathBuf::from(normalized);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        LinkDisposition::Image(path)
    } else if MARKDOWN_EXTENSIONS.contains(&extension.as_str()) {
        LinkDisposition::Markdown(path)
    } else if crate::html::document_page::EXTENSIONS.contains(&extension.as_str()) {
        LinkDisposition::Document(path)
    } else if text {
        LinkDisposition::Text { path, line }
    } else {
        LinkDisposition::Reveal(path)
    }
}

// Refuse UNC/device namespaces before normalizing or touching the filesystem, including mixed separators.
// Windows disk prefixes are local; verbatim disk paths have a double separator prefix but still name a drive.
fn network_or_device_path(path: &str) -> bool {
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = std::path::Path::new(path).components().next() {
        return !matches!(
            prefix.kind(),
            std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)
        );
    }
    let mut chars = path.chars();
    matches!(chars.next(), Some('/' | '\\')) && matches!(chars.next(), Some('/' | '\\'))
}

/// [`route_for_hosts`] with this machine's names.
#[must_use]
pub fn route(raw: &str, mode: LinkOpenMode, origin: ClickOrigin) -> Route {
    route_for_hosts(raw, mode, origin, local_host_names())
}

/// The route of a raw link: [`disposition_for_hosts`] decides open, reveal or ignore, then a web link an overlay can
/// show, clicked in a pane or the scratch terminal, goes to that session's overlay at the mode's size under any mode
/// but [`LinkOpenMode::Browser`]. Whether the overlay is free right now is the caller's check.
#[must_use]
pub fn route_for_hosts<S: BuildHasher>(
    raw: &str,
    mode: LinkOpenMode,
    origin: ClickOrigin,
    local_hosts: &HashSet<String, S>,
) -> Route {
    if matches!(origin, ClickOrigin::RemotePane(_)) && is_bare_path(raw) {
        return Route::Ignore;
    }
    route_disposition(disposition_for_hosts(raw, local_hosts), mode, origin)
}

/// A disposition's route by the origin rules [`route_for_hosts`] describes; a local document or text page shows over
/// the clicking pane's or scratch terminal's session at the mode's size (95% in browser mode, where a browser would
/// get the raw file), and every other origin reveals it.
fn route_disposition(disposition: LinkDisposition, mode: LinkOpenMode, origin: ClickOrigin) -> Route {
    let page_size = if mode == LinkOpenMode::Browser {
        Some(95)
    } else {
        mode.overlay_size_percent()
    };
    match disposition {
        LinkDisposition::Ignore => Route::Ignore,
        LinkDisposition::Reveal(path) => Route::Reveal(path),
        LinkDisposition::Image(path) => match origin {
            ClickOrigin::Pane(session) | ClickOrigin::Scratch(session) => Route::Image { path, session },
            _ => Route::Reveal(path),
        },
        LinkDisposition::Markdown(path) => match origin {
            ClickOrigin::Pane(session) | ClickOrigin::Scratch(session) => Route::Markdown {
                path,
                session,
                size_percent: page_size,
            },
            _ => Route::Reveal(path),
        },
        LinkDisposition::Document(path) => match origin {
            ClickOrigin::Pane(session) | ClickOrigin::Scratch(session) => Route::Document {
                path,
                session,
                size_percent: page_size,
            },
            _ => Route::Reveal(path),
        },
        LinkDisposition::Text { path, line } => match origin {
            ClickOrigin::Pane(session) | ClickOrigin::Scratch(session) => Route::Text {
                path,
                line,
                session,
                size_percent: page_size,
            },
            _ => Route::Reveal(path),
        },
        LinkDisposition::Open(url) => {
            if mode == LinkOpenMode::Browser || !overlay_can_show(&url) {
                return Route::Browser(url);
            }
            match origin {
                ClickOrigin::Pane(session) | ClickOrigin::RemotePane(session) | ClickOrigin::Scratch(session) => {
                    Route::Overlay {
                        url,
                        session,
                        size_percent: mode.overlay_size_percent(),
                    }
                }
                ClickOrigin::Hud | ClickOrigin::ProgramOverlay | ClickOrigin::Quick => Route::Browser(url),
            }
        }
    }
}

/// Whether a session web overlay can show `url`: https, or plain http from a host [`loads_plain_http`] accepts.
/// `mailto`, `ftp` and other plain http go to the desktop's handler.
fn overlay_can_show(url: &str) -> bool {
    let Some(link) = Link::parse(url) else {
        return false;
    };
    match link.scheme.as_str() {
        "https" => true,
        "http" => loads_plain_http(link.host.as_deref().unwrap_or_default()),
        _ => false,
    }
}

/// Whether the app's web views load plain http from `host`: local networking only, read off the host's syntax, never
/// off where it resolves (upstream's App Transport Security rule): an unqualified name, a `.local` name, or an IP
/// literal, a public one included.
fn loads_plain_http(host: &str) -> bool {
    let host = normalized_host(host);
    if host.is_empty() {
        return false;
    }
    let local_name = host.rsplit_once('.').is_some_and(|(_, last)| last == "local");
    if local_name || !host.contains(['.', ':']) {
        return true;
    }
    host.parse::<Ipv4Addr>().is_ok() || host.parse::<Ipv6Addr>().is_ok()
}

/// Whether a dot-normalized absolute path is an automount root or under one: exact or a `<root>/…` child, so
/// `/networkx` is not caught.
fn is_automount_path(path: &str) -> bool {
    #[cfg(target_os = "macos")]
    let path = path.to_lowercase();
    AUTOMOUNT_ROOTS.iter().any(|root| {
        path.strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

/// Collapses `.` and `..` in an absolute path lexically: no filesystem access, no symlink resolution. A `..` at the
/// root is dropped.
fn lexically_normalized_absolute_path(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        match component {
            "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(component),
        }
    }
    format!("/{}", out.join("/"))
}

/// The parts of a URL the policy reads.
#[derive(Debug, PartialEq, Eq)]
struct Link<'a> {
    /// Lowercased.
    scheme: String,
    /// The authority's host, percent-decoded, without an IPv6 literal's brackets; `None` without an authority.
    host: Option<String>,
    /// The path, still percent-encoded.
    path: &'a str,
}

impl<'a> Link<'a> {
    /// Parses an absolute RFC 3986 URI; `None` for a relative reference, a character a URI cannot hold (see
    /// [`uri_char`]), a broken percent-escape, or a malformed authority.
    fn parse(raw: &'a str) -> Option<Self> {
        if !raw.chars().all(uri_char) || !valid_escapes(raw) {
            return None;
        }
        let (scheme, rest) = raw.split_once(':')?;
        let mut chars = scheme.chars();
        if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return None;
        }
        let hier = rest.split(['?', '#']).next().unwrap_or_default();
        let (host, path) = match hier.strip_prefix("//") {
            Some(after) => {
                let end = after.find('/').unwrap_or(after.len());
                (Some(authority_host(&after[..end])?), &after[end..])
            }
            None => (None, hier),
        };
        Some(Self {
            scheme: scheme.to_ascii_lowercase(),
            host,
            path,
        })
    }
}

/// The host of an authority (`[userinfo@]host[:port]`), percent-decoded; `None` for a port that is not a number or
/// an unterminated IPv6 literal.
fn authority_host(authority: &str) -> Option<String> {
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host_port)| host_port);
    let (host, port) = if let Some(literal) = host_port.strip_prefix('[') {
        let (host, after) = literal.split_once(']')?;
        let port = if after.is_empty() { "" } else { after.strip_prefix(':')? };
        (host, port)
    } else {
        host_port.rsplit_once(':').unwrap_or((host_port, ""))
    };
    if !port.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    percent_decode(host)
}

/// Whether `c` may appear in a URI as written: no space, no control character, none of the characters RFC 3986
/// excludes (the double quote, angle brackets, backslash, caret, grave accent, braces and bar). Non-ASCII passes (an
/// IRI).
fn uri_char(c: char) -> bool {
    !c.is_control() && !c.is_whitespace() && !matches!(c, '"' | '<' | '>' | '\\' | '^' | '`' | '{' | '|' | '}')
}

/// Whether every `%` starts a two-hex-digit escape.
fn valid_escapes(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.iter().enumerate().filter(|(_, b)| **b == b'%').all(|(index, _)| {
        bytes
            .get(index + 1..index + 3)
            .is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit))
    })
}

/// Decodes percent-escapes; `None` when the result is not UTF-8.
pub(crate) fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escaped = (byte == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(decoded) = escaped {
            out.push(decoded);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupt::KeyModifiers;

    /// A fixed set, so host matching does not depend on the machine.
    fn hosts() -> HashSet<String> {
        ["myhost", "localhost"].into_iter().map(str::to_owned).collect()
    }

    fn disposition_of(raw: &str) -> LinkDisposition {
        disposition_for_hosts(raw, &hosts())
    }

    fn reveal(path: &str) -> LinkDisposition {
        LinkDisposition::Reveal(PathBuf::from(path))
    }

    /// Upstream's `onlyAPlainClickOnALogOpensAsALogPage`: a `.log` or `.out` file in any case, with no line.
    #[test]
    fn only_a_plain_click_on_a_log_opens_as_a_log_page() {
        let session = uuid::Uuid::nil();
        for (path, line, expected) in [
            ("/x/app.log", None, true),
            ("/x/run.OUT", None, true),
            ("/x/app.log", Some(42), false),
            ("/x/notes.txt", None, false),
            ("/x/a.go", None, false),
        ] {
            let route = Route::Text {
                path: PathBuf::from(path),
                line,
                session,
                size_percent: None,
            };
            assert_eq!(route.opens_as_log(), expected, "{path} {line:?}");
        }
        assert!(!Route::Reveal(PathBuf::from("/x/app.log")).opens_as_log());
    }

    // disposition (open / reveal / ignore)

    /// The garbage suffixes model a C string over-read past the link's length: they leave the URL unparseable.
    #[test]
    fn trailing_garbage_does_not_yield_a_web_url() {
        assert_eq!(
            disposition_of("https://example.com\0/etc/other"),
            LinkDisposition::Ignore
        );
        assert_eq!(
            disposition_of("https://example.com extra junk"),
            LinkDisposition::Ignore
        );
    }

    #[test]
    fn web_schemes_open() {
        for raw in [
            "http://example.com",
            "https://example.com/path?q=1#frag",
            "HTTPS://EXAMPLE.COM",
            "mailto:someone@example.com",
            "ftp://host/file.txt",
        ] {
            assert_eq!(disposition_of(raw), LinkDisposition::Open(raw.to_owned()), "{raw}");
        }
    }

    /// A local `file://` reveals the host-stripped, dot-normalized path, so every host form of `/tmp/x.bin` resolves
    /// to the same plain path.
    #[test]
    fn local_file_links_reveal() {
        for (raw, expected) in [
            ("file:///tmp/x.bin", "/tmp/x.bin"),
            ("file:/tmp/x.bin", "/tmp/x.bin"),
            ("file://localhost/tmp/x.bin", "/tmp/x.bin"),
            ("file://myhost/tmp/x.bin", "/tmp/x.bin"),
            ("file://MYHOST/tmp/x.bin", "/tmp/x.bin"),
            ("file://myhost./tmp/x.bin", "/tmp/x.bin"),
            ("FILE:///tmp/x.bin", "/tmp/x.bin"),
            ("file:///tmp/../tmp/x.bin", "/tmp/x.bin"),
            ("file://myhost/tmp/a%20b.bin", "/tmp/a b.bin"),
            (
                "file:///usr/share/applications/x.desktop",
                "/usr/share/applications/x.desktop",
            ),
            ("file://user@myhost:22/tmp/x.bin", "/tmp/x.bin"),
        ] {
            assert_eq!(disposition_of(raw), reveal(expected), "{raw}");
        }
    }

    #[test]
    fn automount_paths_are_ignored() {
        for raw in [
            "file:///net/server/share/x.bin",
            "file:///misc/cd/x",
            "file://localhost/net/server/share",
            "file:///tmp/../net/server/share",
            "file:///tmp/%2E%2E/net/server/share",
            "file:///net",
            "file:///misc",
        ] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    /// Siblings merely starting with an automount root reveal; `/home` is a real home root off macOS.
    #[test]
    fn automount_lookalikes_and_homes_still_reveal() {
        for path in ["/networkx/x.bin", "/nettools/x.bin", "/miscellany/x.bin"] {
            assert_eq!(disposition_of(&format!("file://{path}")), reveal(path), "{path}");
        }
        for path in ["/home/someone/x.bin", "/home"] {
            let expected = if cfg!(target_os = "macos") {
                LinkDisposition::Ignore
            } else {
                reveal(path)
            };
            assert_eq!(disposition_of(&format!("file://{path}")), expected, "{path}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_automount_aliases_and_case_variants_are_rejected_lexically() {
        for path in [
            "/Network/Servers/host/a.md",
            "/HOME/a.png",
            "/System/Volumes/Data/home/u/a.png",
            "/System/Volumes/Data/net/host/a.md",
            "/System/Volumes/Data/Network/Servers/host/a.png",
        ] {
            assert_eq!(
                disposition_of(&format!("file://{path}")),
                LinkDisposition::Ignore,
                "{path}"
            );
        }
        for path in ["/Networkx/a.bin", "/System/Volumes/Data/homework/a.bin"] {
            assert_eq!(disposition_of(&format!("file://{path}")), reveal(path), "{path}");
        }
    }

    /// The classifier never resolves symlinks: a link inside a symlinked directory keeps the link's path. Unix only: a
    /// Windows temporary path (`C:\...`) is not a `file://` path as written, and its symlink needs a privilege.
    #[cfg(unix)]
    #[test]
    fn reveal_does_not_resolve_symlinks() -> std::io::Result<()> {
        let base = tempfile::tempdir()?;
        let target = base.path().join("target");
        std::fs::create_dir_all(&target)?;
        // the leaf must exist, else a symlink-resolving regression would return the link's path too
        std::fs::write(target.join("x.bin"), b"")?;
        std::os::unix::fs::symlink(&target, base.path().join("link"))?;
        let leaf = base.path().join("link").join("x.bin");
        let raw = format!("file://{}", leaf.display());
        assert_eq!(disposition_of(&raw), LinkDisposition::Reveal(leaf));
        Ok(())
    }

    /// An empty or relative `file://` path would reveal the working directory: ignored.
    #[test]
    fn empty_or_relative_file_paths_are_ignored() {
        for raw in ["file://localhost", "file://myhost", "file:relative"] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    #[test]
    fn non_local_file_links_are_ignored() {
        for raw in ["file://otherhost/tmp/x.bin", "file://remote.example.com/share/x.bin"] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    #[test]
    fn other_schemes_are_ignored() {
        for raw in [
            "javascript:alert(1)",
            "vscode://file/x",
            "tel:+15550100",
            "example.com",
            "",
            "   ",
            "1http://example.com",
            "https://example.com/%zz",
            "http://example.com:port/",
        ] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    #[test]
    fn unc_paths_are_ignored() {
        for raw in [
            "file:////server/share/x.bin",
            "file://localhost//server/share/x.bin",
            "file://localhost/%2Fserver/share",
        ] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    #[test]
    fn network_and_device_prefixes_are_refused_before_any_path_probe() {
        for path in [
            r"\\server\share\a.png",
            r"\\?\UNC\server\share\a.md",
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\a.png",
            "//server/share/a.png",
            r"/\server/share/a.md",
            r"\/server/share/a.txt",
        ] {
            assert!(network_or_device_path(path), "{path}");
            assert_eq!(local_path(path, None), LinkDisposition::Ignore, "{path}");
            assert_eq!(disposition_of(path), LinkDisposition::Ignore, "{path}");
        }
        for raw in [
            "file://localhost/%5c%5cserver/share/a.png",
            "file://localhost/%2f%5cserver/share/a.md",
        ] {
            assert_eq!(disposition_of(raw), LinkDisposition::Ignore, "{raw}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_local_disk_and_verbatim_disk_paths_still_open_documents() {
        for path in [r"C:\local\a.png", r"\\?\C:\local\a.png"] {
            assert!(!network_or_device_path(path), "{path}");
            assert_eq!(
                local_path(path, None),
                LinkDisposition::Image(PathBuf::from(path)),
                "{path}"
            );
        }
    }

    #[test]
    fn an_encoded_nul_or_invalid_utf8_path_is_ignored() {
        assert_eq!(disposition_of("file:///tmp/a%00b"), LinkDisposition::Ignore);
        assert_eq!(disposition_of("file:///tmp/%ff"), LinkDisposition::Ignore);
    }

    // local_host_names / expanded_host_names

    #[test]
    fn expanded_host_names_normalize_and_add_the_local_short_form() {
        let out = expanded_host_names(["MyMac.local", "Box.", "localhost", "", ".local"]);
        assert!(out.contains("mymac.local"));
        assert!(out.contains("mymac"));
        assert!(out.contains("box"));
        assert!(out.contains("localhost"));
        assert!(!out.contains(""));
    }

    #[test]
    fn local_host_names_always_have_localhost() {
        assert!(local_host_names().contains("localhost"));
    }

    /// The default host set reveals an empty-host or localhost file link.
    #[test]
    fn default_local_hosts_reveal_local_files() {
        for raw in ["file:///tmp/x.bin", "file://localhost/tmp/x.bin"] {
            assert_eq!(disposition(raw), reveal("/tmp/x.bin"), "{raw}");
        }
    }

    // route (browser / overlay / reveal / ignore)

    fn origins(session: Uuid) -> [ClickOrigin; 6] {
        [
            ClickOrigin::Pane(session),
            ClickOrigin::RemotePane(session),
            ClickOrigin::Scratch(session),
            ClickOrigin::Hud,
            ClickOrigin::ProgramOverlay,
            ClickOrigin::Quick,
        ]
    }

    fn route_of(raw: &str, mode: LinkOpenMode, origin: ClickOrigin) -> Route {
        route_for_hosts(raw, mode, origin, &hosts())
    }

    #[test]
    fn browser_mode_sends_web_links_to_the_browser() {
        for raw in ["http://example.com/a", "https://example.com/a?q=1"] {
            for origin in origins(Uuid::new_v4()) {
                assert_eq!(
                    route_of(raw, LinkOpenMode::Browser, origin),
                    Route::Browser(raw.to_owned())
                );
            }
        }
    }

    #[test]
    fn overlay_mode_opens_pane_and_scratch_web_links_on_the_owning_session() {
        let session = Uuid::new_v4();
        for raw in ["http://localhost:5173/a", "HTTPS://example.com/a"] {
            for origin in [
                ClickOrigin::Pane(session),
                ClickOrigin::RemotePane(session),
                ClickOrigin::Scratch(session),
            ] {
                let url = raw.to_owned();
                assert_eq!(
                    route_of(raw, LinkOpenMode::Overlay, origin),
                    Route::Overlay {
                        url,
                        session,
                        size_percent: None
                    }
                );
            }
        }
    }

    /// Upstream's `eachOverlayModeCarriesItsSizeToTheRoute`.
    #[test]
    fn each_overlay_mode_carries_its_size_to_the_route() {
        let session = Uuid::new_v4();
        for (mode, size_percent) in [
            (LinkOpenMode::Overlay, None),
            (LinkOpenMode::Overlay95, Some(95)),
            (LinkOpenMode::Overlay75, Some(75)),
            (LinkOpenMode::Overlay50, Some(50)),
        ] {
            assert_eq!(
                route_of("https://example.com/a", mode, ClickOrigin::Pane(session)),
                Route::Overlay {
                    url: "https://example.com/a".to_owned(),
                    session,
                    size_percent
                },
                "{mode:?}"
            );
        }
    }

    /// Regression upstream: a plain-http page the app's web views cannot load was routed to the overlay.
    #[test]
    fn overlay_mode_takes_plain_http_only_from_hosts_a_web_view_may_load() {
        let session = Uuid::new_v4();
        for (raw, overlay) in [
            ("http://localhost/", true),
            ("http://LOCALHOST:8080/x", true),
            ("http://devbox/", true),
            ("http://mac.local:3000/", true),
            ("http://127.0.0.1:5173/", true),
            ("http://93.184.216.34/", true),
            ("http://[::1]:8080/", true),
            ("http://[2001:db8::1]/", true),
            ("http://example.com/", false),
            ("http://www.example.com:8080/a", false),
            ("http://local/", true),
            ("http://notlocal.localx/", false),
            ("http://1.2.3.example/", false),
            ("http://999.1.1.1/", false),
            ("https://example.com/", true),
        ] {
            let url = raw.to_owned();
            let expected = if overlay {
                Route::Overlay {
                    url,
                    session,
                    size_percent: None,
                }
            } else {
                Route::Browser(url)
            };
            let routed = route_of(raw, LinkOpenMode::Overlay, ClickOrigin::Pane(session));
            assert_eq!(routed, expected, "{raw}");
        }
    }

    #[test]
    fn overlay_mode_keeps_sessionless_origins_in_the_browser() {
        for origin in [ClickOrigin::Hud, ClickOrigin::ProgramOverlay, ClickOrigin::Quick] {
            assert_eq!(
                route_of("https://example.com", LinkOpenMode::Overlay, origin),
                Route::Browser("https://example.com".to_owned())
            );
        }
    }

    #[test]
    fn non_web_schemes_always_go_to_the_system_handler() {
        for raw in ["mailto:someone@example.com", "ftp://host/file.txt"] {
            for mode in LinkOpenMode::ALL {
                let routed = route_of(raw, *mode, ClickOrigin::Pane(Uuid::new_v4()));
                assert_eq!(routed, Route::Browser(raw.to_owned()), "{raw}");
            }
        }
    }

    #[test]
    fn file_links_reveal_and_other_schemes_stay_ignored_in_every_mode() {
        for mode in LinkOpenMode::ALL {
            for origin in origins(Uuid::new_v4()) {
                assert_eq!(
                    route_of("file:///tmp/x.bin", *mode, origin),
                    Route::Reveal(PathBuf::from("/tmp/x.bin"))
                );
                assert_eq!(route_of("x-custom://run", *mode, origin), Route::Ignore);
                assert_eq!(route_of("file://elsewhere/tmp/x.bin", *mode, origin), Route::Ignore);
            }
        }
    }

    // the browser chord (the Ctrl/Super+Shift override)

    /// Whether `click_intent` sends the click to the browser.
    fn wants_browser(modifiers: KeyModifiers, mode: LinkOpenMode, origin: ClickOrigin, mouse_captured: bool) -> bool {
        click_intent(modifiers, mode, origin, mouse_captured) == ClickIntent::Browser
    }

    const OVERLAY_MODES: [LinkOpenMode; 4] = [
        LinkOpenMode::Overlay,
        LinkOpenMode::Overlay95,
        LinkOpenMode::Overlay75,
        LinkOpenMode::Overlay50,
    ];

    fn chords() -> [KeyModifiers; 2] {
        [
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            KeyModifiers::COMMAND | KeyModifiers::SHIFT,
        ]
    }

    /// Upstream's `commandShiftAsksForTheBrowserInEveryOverlayMode` (Ctrl or Super standing for ⌘).
    #[test]
    fn the_chord_asks_for_the_browser_in_every_overlay_mode() {
        let session = Uuid::new_v4();
        for origin in [
            ClickOrigin::Pane(session),
            ClickOrigin::RemotePane(session),
            ClickOrigin::Scratch(session),
        ] {
            for chord in chords() {
                for mode in OVERLAY_MODES {
                    assert!(wants_browser(chord, mode, origin, false), "{mode:?} {origin:?}");
                }
                assert!(!wants_browser(chord, LinkOpenMode::Browser, origin, false));
            }
        }
    }

    /// Upstream's `onlyTheExactCommandShiftChordAsksForTheBrowser`.
    #[test]
    fn only_the_exact_chord_asks_for_the_browser() {
        let (ctrl, sup, alt, shift) = (
            KeyModifiers::CONTROL,
            KeyModifiers::COMMAND,
            KeyModifiers::OPTION,
            KeyModifiers::SHIFT,
        );
        for modifiers in [
            ctrl,
            sup,
            shift,
            KeyModifiers::NONE,
            ctrl | shift | alt,
            sup | shift | alt,
            sup | shift | ctrl,
            ctrl | alt,
            sup | alt,
        ] {
            let session = ClickOrigin::Pane(Uuid::new_v4());
            assert!(
                !wants_browser(modifiers, LinkOpenMode::Overlay, session, false),
                "{modifiers:?}"
            );
        }
    }

    /// Upstream's `commandShiftUnderMouseCaptureFollowsTheSetting`.
    #[test]
    fn the_chord_under_mouse_capture_follows_the_setting() {
        let session = Uuid::new_v4();
        for origin in [
            ClickOrigin::Pane(session),
            ClickOrigin::RemotePane(session),
            ClickOrigin::Scratch(session),
        ] {
            for chord in chords() {
                assert!(!wants_browser(chord, LinkOpenMode::Overlay, origin, true));
            }
        }
    }

    /// Upstream's `sessionlessOriginsNeverTakeTheOverride`.
    #[test]
    fn sessionless_origins_never_take_the_override() {
        for origin in [ClickOrigin::Hud, ClickOrigin::ProgramOverlay, ClickOrigin::Quick] {
            for chord in chords() {
                assert!(!wants_browser(chord, LinkOpenMode::Overlay, origin, false));
            }
        }
    }
    #[test]
    fn local_document_paths_are_literal_and_expand_home_without_io() {
        let home = crate::html::os_path(if cfg!(target_os = "macos") {
            "/Users/u"
        } else {
            "/home/u"
        });
        for (raw, expected) in [
            ("/tmp/a%20b?# c.PNG", "/tmp/a%20b?# c.PNG"),
            ("/tmp/x/../a.webp", "/tmp/a.webp"),
        ] {
            let raw = crate::html::os_path(raw);
            assert_eq!(
                disposition_for_hosts_and_home(&raw, &hosts(), &home),
                LinkDisposition::Image(PathBuf::from(crate::html::os_path(expected)))
            );
        }
        assert_eq!(
            disposition_for_hosts_and_home("~/a.png", &hosts(), &home),
            LinkDisposition::Image(std::path::Path::new(&home).join("a.png"))
        );
        for raw in ["/tmp/a.md", "/tmp/a.MARKDOWN"] {
            let raw = crate::html::os_path(raw);
            assert_eq!(
                disposition_for_hosts_and_home(&raw, &hosts(), &home),
                LinkDisposition::Markdown(PathBuf::from(raw))
            );
        }
        for raw in [
            "/net/a.png",
            "/tmp/../net/a.md",
            "//remote/a.png",
            "/tmp/a\0.png",
            "relative.md",
        ] {
            assert_eq!(
                disposition_for_hosts_and_home(raw, &hosts(), "/home/u"),
                LinkDisposition::Ignore,
                "{raw}"
            );
        }
    }

    #[test]
    fn documents_route_locally_and_remote_panes_never_open_them() {
        let session = Uuid::new_v4();
        let image = crate::html::os_path("/tmp/a.png");
        for mode in LinkOpenMode::ALL {
            for origin in [ClickOrigin::Pane(session), ClickOrigin::Scratch(session)] {
                assert_eq!(
                    route_of(&image, *mode, origin),
                    Route::Image {
                        path: PathBuf::from(&image),
                        session
                    }
                );
                assert_eq!(
                    route_of("file:///tmp/a.md", *mode, origin),
                    Route::Markdown {
                        path: PathBuf::from("/tmp/a.md"),
                        session,
                        size_percent: if *mode == LinkOpenMode::Browser {
                            Some(95)
                        } else {
                            mode.overlay_size_percent()
                        }
                    }
                );
            }
            for origin in [
                ClickOrigin::RemotePane(session),
                ClickOrigin::Hud,
                ClickOrigin::Quick,
                ClickOrigin::ProgramOverlay,
            ] {
                assert_eq!(
                    route_of("file:///tmp/a.png", *mode, origin),
                    Route::Reveal(PathBuf::from("/tmp/a.png"))
                );
                assert_eq!(
                    route_of("file:///tmp/a.md", *mode, origin),
                    Route::Reveal(PathBuf::from("/tmp/a.md"))
                );
            }
            assert_eq!(route_of(&image, *mode, ClickOrigin::RemotePane(session)), Route::Ignore);
        }
    }

    /// Upstream's `localHtmlPathsOpenAsDocuments`: a bare, `~/` or `file://` path to an `.html` or `.htm` file, in any
    /// case, is a document, ahead of the text check.
    #[test]
    fn local_html_paths_open_as_documents() {
        let home = crate::html::os_path("/Users/me");
        for (raw, path) in [
            (
                crate::html::os_path("/tmp/agdoc/pr.html"),
                crate::html::os_path("/tmp/agdoc/pr.html"),
            ),
            (
                crate::html::os_path("/tmp/INDEX.HTM"),
                crate::html::os_path("/tmp/INDEX.HTM"),
            ),
            ("~/page.html".to_owned(), crate::html::os_path("/Users/me/page.html")),
        ] {
            assert_eq!(
                disposition_for_hosts_and_home(&raw, &hosts(), &home),
                LinkDisposition::Document(PathBuf::from(path)),
                "{raw}"
            );
        }
        let spaced = PathBuf::from(crate::html::os_path("/tmp/a b.html"));
        let url = url::Url::from_file_path(&spaced).unwrap();
        assert!(url.as_str().ends_with("/tmp/a%20b.html"));
        assert_eq!(disposition_of(url.as_str()), LinkDisposition::Document(spaced));
    }

    /// Upstream's `documentOpensOnTheOwningSessionAtItsModeSize`, `sessionlessOriginsRevealDocuments` and the remote
    /// pane's HTML rows: a pane or the scratch terminal shows the file over its session at the mode's size, 95 in
    /// browser mode; every other origin reveals a `file://` link, and a remote pane ignores a bare path.
    #[test]
    fn a_document_opens_on_the_owning_session_at_its_mode_size() {
        let session = Uuid::new_v4();
        let path = PathBuf::from(crate::html::os_path("/tmp/pr.html"));
        let bare = crate::html::os_path("/tmp/pr.html");
        let file = url::Url::from_file_path(&path).unwrap().to_string();
        for mode in LinkOpenMode::ALL {
            let percent = if *mode == LinkOpenMode::Browser {
                Some(95)
            } else {
                mode.overlay_size_percent()
            };
            for origin in [ClickOrigin::Pane(session), ClickOrigin::Scratch(session)] {
                for raw in [&bare, &file] {
                    assert_eq!(
                        route_of(raw, *mode, origin),
                        Route::Document {
                            path: path.clone(),
                            session,
                            size_percent: percent
                        },
                        "{raw} {mode:?} {origin:?}"
                    );
                }
            }
            for origin in [
                ClickOrigin::Hud,
                ClickOrigin::ProgramOverlay,
                ClickOrigin::Quick,
                ClickOrigin::RemotePane(session),
            ] {
                assert_eq!(
                    route_of(&file, *mode, origin),
                    Route::Reveal(path.clone()),
                    "{origin:?}"
                );
            }
            assert_eq!(route_of(&bare, *mode, ClickOrigin::RemotePane(session)), Route::Ignore);
        }
        let route = Route::Document {
            path: path.clone(),
            session,
            size_percent: Some(95),
        };
        assert_eq!(route.local_file(), Some(path.as_path()));
    }

    #[test]
    fn native_file_urls_resolve_to_the_same_document_path_on_each_platform() {
        let path = PathBuf::from(crate::html::os_path("/tmp/local document.md"));
        let url = url::Url::from_file_path(&path).unwrap();
        assert_eq!(disposition_of(url.as_str()), LinkDisposition::Markdown(path));
    }
}
