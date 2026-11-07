//! The files a clicked link may name when only the file system can tell which, and what a click's chord asks for
//! (upstream's `LinkPolicy.locations`, `lineSuffix`, `relativePath`, `ClickIntent` and `clickIntent`).
//!
//! A link ending in `:line[:column]` names the literal path first, so a file named with a colon still wins, then the
//! path without the suffix, opened at that line. A relative link is joined to the clicking pane's directory before any
//! scheme parse, so `./main.go:42` never reads as a URL scheme. Ghostty keeps a sentence's trailing `.`, `?`, `!` or
//! `;` in the link too, so a path ending in one also names up to three spellings with one character removed at a time,
//! the literal first so a name really ending in one wins; on Windows, where no name ends in a dot, a spelling that does
//! is dropped for the shorter ones. Every candidate passed the same guards as a click
//! ([`super::guarded_local_path`]), so a refused path is never stat'ed; a remote pane's paths name files on another
//! machine and offer none. The frontend stats the candidates off its main thread, the first one found winning
//! ([`route_for_location`]), and a link with none on disk keeps the route it had as typed.
//!
//! On Windows a drive's colon stays part of the path: a `:line` suffix is split only where a name remains that is
//! still absolute.

use std::collections::HashSet;
use std::hash::BuildHasher;

use super::{
    ClickOrigin, Link, LinkOpenMode, Route, guarded_local_path, is_bare_path, local_file_path, local_host_names,
    local_path, route_disposition,
};
use crate::interrupt::KeyModifiers;

/// A local file a clicked link may name, with the line a `:line[:column]` suffix gave (upstream's `LinkLocation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLocation {
    pub path: String,
    pub line: Option<i64>,
}

impl LinkLocation {
    fn literal(path: String) -> Self {
        Self { path, line: None }
    }
}

/// [`locations_for_hosts_and_home`] with this machine's names and home.
#[must_use]
pub fn locations(raw: &str, origin: ClickOrigin, cwd: Option<&str>) -> Vec<LinkLocation> {
    let home = crate::paths::home_dir(&crate::paths::ProcessEnv).unwrap_or_default();
    locations_for_hosts_and_home(raw, origin, cwd, local_host_names(), &home.to_string_lossy())
}

/// The candidates a click on `raw` names, in the order they are tried: for a link ending in `:line[:column]`, the
/// literal path and then the path without the suffix at that line; for a relative link joined to `cwd`, the joined
/// path; for one ending in sentence punctuation, the literal path and then each shorter spelling
/// ([`trimmed_spellings`]), each split for a suffix the same way. Empty when [`super::route`] decides the link as
/// typed: no suffix, no trailing punctuation and not relative, a remote pane, a control character, a refused path.
#[must_use]
pub fn locations_for_hosts_and_home<S: BuildHasher>(
    raw: &str,
    origin: ClickOrigin,
    cwd: Option<&str>,
    local_hosts: &HashSet<String, S>,
    home: &str,
) -> Vec<LinkLocation> {
    if matches!(origin, ClickOrigin::RemotePane(_)) || raw.chars().any(char::is_control) {
        return Vec::new();
    }
    let mut relative = false;
    let path = if is_bare_path(raw) {
        raw.strip_prefix("~/")
            .map_or_else(|| raw.to_owned(), |rest| format!("{home}/{rest}"))
    } else if let Some(joined) = relative_path(raw, cwd) {
        relative = true;
        joined
    } else if let Some(path) = Link::parse(raw)
        .filter(|link| link.scheme == "file")
        .and_then(|link| local_file_path(&link, local_hosts))
    {
        path
    } else {
        return Vec::new();
    };
    let Some(literal) = guarded_local_path(&path) else {
        return Vec::new();
    };
    let mut spellings: Vec<String> = std::iter::once(literal)
        .chain(trimmed_spellings(&path).into_iter().filter_map(guarded_local_path))
        .collect();
    let trimmed = spellings.len() > 1;
    if cfg!(windows) && spellings.iter().any(|spelling| !spelling.ends_with('.')) {
        // Windows drops a name's trailing dots when it opens a path, so `review.md.` is `review.md` there: a spelling
        // ending in one would be found first and routed by its empty extension
        spellings.retain(|spelling| !spelling.ends_with('.'));
    }
    let mut found = Vec::new();
    for spelling in &spellings {
        let suffix = line_suffix(spelling);
        if relative || suffix.is_some() || trimmed {
            found.push(LinkLocation::literal(spelling.clone()));
        }
        if let Some((base, line)) = suffix
            && let Some(stripped) = guarded_local_path(base)
        {
            found.push(LinkLocation {
                path: stripped,
                line: Some(line),
            });
        }
    }
    found
}

/// What ends a sentence after a printed path; Ghostty's path rule keeps it in the link (upstream's
/// `trailingPunctuation`). A comma or a closing bracket already ends the link, and a detected one never ends in a
/// colon.
const TRAILING_PUNCTUATION: [char; 4] = ['.', '?', '!', ';'];

/// The most trailing punctuation characters [`trimmed_spellings`] removes, bounding what a click on a long run builds
/// and stats.
const MAX_TRIMMED: usize = 3;

/// `path` with up to [`MAX_TRIMMED`] trailing [`TRAILING_PUNCTUATION`] characters removed, one at a time and longest
/// first, so a name really ending in one still wins (upstream's `trimmedSpellings`). It stops before leaving a last
/// component of punctuation alone, so `/x/..` never becomes `/x/.`.
fn trimmed_spellings(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = path;
    while out.len() < MAX_TRIMMED {
        let Some(shorter) = rest.strip_suffix(TRAILING_PUNCTUATION) else {
            break;
        };
        rest = shorter;
        let name = rest.rsplit(separator).next().unwrap_or(rest);
        if name.chars().all(|ch| TRAILING_PUNCTUATION.contains(&ch)) {
            break;
        }
        out.push(rest);
    }
    out
}

/// The path separators a relative link and a path's last component are read with: `/`, and `\` on Windows.
fn separator(ch: char) -> bool {
    ch == '/' || (cfg!(windows) && ch == '\\')
}

/// A relative link joined to an absolute, control-free `cwd` (upstream's `relativePath`). A first segment with a colon
/// is a scheme or a host and port, never a directory, so `http://localhost:3000` stays a web link; one starting with
/// `~` is a home path. Unlike upstream, a name with no separator at all (`notes.txt`) is joined too, as this build
/// joined a detected relative path before (Ghostty's `resolvePathForOpening`), a colon in it still refusing it.
fn relative_path(raw: &str, cwd: Option<&str>) -> Option<String> {
    let cwd = cwd.filter(|cwd| crate::html::is_absolute(cwd) && !cwd.chars().any(char::is_control))?;
    let first = raw.split(separator).next().unwrap_or(raw);
    if first.is_empty() || first.contains(':') || first.starts_with('~') {
        return None;
    }
    Some(std::path::Path::new(cwd).join(raw).to_string_lossy().into_owned())
}

/// A trailing `:line` or `:line:column` split off `path`, `None` unless the line is 1 or more and an absolute name
/// remains that does not end in a separator (upstream's `lineSuffix`). The column is dropped: a text page scrolls to
/// the line.
fn line_suffix(path: &str) -> Option<(&str, i64)> {
    let number = |part: &str| -> Option<i64> {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let (rest, last) = path.rsplit_once(':')?;
    let last = number(last)?;
    let (base, line) = match rest.rsplit_once(':') {
        Some((before, line)) if number(line).is_some() && !before.is_empty() => (before, number(line)?),
        _ => (rest, last),
    };
    let named = !base.is_empty() && !base.ends_with(separator) && crate::html::is_absolute(base);
    (line >= 1 && named).then_some((base, line))
}

/// The route of a location [`locations`] offered and the frontend found on disk, by the same origin rules as a typed
/// link (upstream's `route(for: LinkLocation)`).
#[must_use]
pub fn route_for_location(location: &LinkLocation, mode: LinkOpenMode, origin: ClickOrigin) -> Route {
    route_disposition(local_path(&location.path, location.line), mode, origin)
}

/// What a link click's chord asks for: the setting's route, the browser, or a reveal in the file manager (upstream's
/// `ClickIntent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickIntent {
    Normal,
    Browser,
    Reveal,
}

/// Reads a link click's chord (upstream's `clickIntent`, Ctrl or Super standing for ⌘ and Alt for ⌥). Ctrl/Super+Shift
/// exactly asks for the browser over an overlay setting from a pane or the scratch terminal, except under mouse
/// capture, where it is the only link click and so follows the setting. Ctrl/Super+Alt exactly reveals a local path
/// in the file manager from any origin; under capture, where Shift is the bypass, Ctrl/Super+Alt+Shift does.
///
/// Ctrl+Alt is `AltGr` on many Windows keyboard layouts, so a click with `AltGr` held reveals there too; on Linux a
/// desktop may grab Super+Alt (or Ctrl+Alt) clicks for its own window moves before the terminal sees them, leaving the
/// other spelling of the chord.
#[must_use]
pub fn click_intent(
    modifiers: KeyModifiers,
    mode: LinkOpenMode,
    origin: ClickOrigin,
    mouse_captured: bool,
) -> ClickIntent {
    let capture = if mouse_captured {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    let reveal = [KeyModifiers::CONTROL, KeyModifiers::COMMAND]
        .iter()
        .any(|command| modifiers == (*command | KeyModifiers::OPTION | capture));
    if reveal {
        return ClickIntent::Reveal;
    }
    let browser = modifiers == (KeyModifiers::CONTROL | KeyModifiers::SHIFT)
        || modifiers == (KeyModifiers::COMMAND | KeyModifiers::SHIFT);
    if !browser || mode == LinkOpenMode::Browser || mouse_captured {
        return ClickIntent::Normal;
    }
    match origin {
        ClickOrigin::Pane(_) | ClickOrigin::RemotePane(_) | ClickOrigin::Scratch(_) => ClickIntent::Browser,
        ClickOrigin::Hud | ClickOrigin::ProgramOverlay | ClickOrigin::Quick => ClickIntent::Normal,
    }
}

#[cfg(test)]
#[path = "locations_tests.rs"]
mod tests;
