//! HTML overlays: a page occupying an overlay slot in place of a program (upstream's `HtmlOverlay.swift`,
//! `Session+HtmlOverlay.swift`): what a page shows ([`HtmlSource`]), its origin, the page-zoom ladder, the page value
//! the model carries ([`HtmlOverlay`]), the checks `session.overlay.open` runs before any web view exists (`grantError`,
//! `webURL`), the theme a page sees, and the seam to the frontend's web views ([`HtmlHost`]). The navigation policy,
//! the page bridge, the outcomes, the browser profile and a path's volume as the grant's checks compare it have modules
//! of their own.

pub mod bridge;
pub mod document_page;
pub mod image_page;
pub mod log_tail;
pub mod markdown_page;
pub mod navigation;
pub mod outcomes;
pub mod profile;
pub mod text_page;
pub mod theme;
mod volume;

#[cfg(test)]
mod page_tests;

pub use navigation::{HtmlNavigationAction, HtmlNavigationDecision, HtmlNavigationTarget, HtmlOrigin, decide};
pub use outcomes::HtmlPageOutcomes;
pub use theme::HtmlOverlayTheme;
pub use volume::{strictly_inside, volume};

use url::Url;

/// A page's `--html` file must be absolute (upstream's `grantError` messages, without the command prefix).
pub const FILE_NOT_ABSOLUTE: &str = "html file must be an absolute path";
pub const CWD_NOT_ABSOLUTE: &str = "cwd must be an absolute path";
pub const CWD_ROOT_OR_HOME: &str = "cwd must not be / or the home directory";
pub const CWD_IS_FILE: &str = "cwd must be a directory containing the html file";
pub const OUTSIDE_CWD: &str = "html file is outside cwd";

/// Why a file page and its `--cwd` grant cannot be shown (upstream's `HtmlOverlay.grantError`): the file must be
/// absolute; a grant must be absolute, neither the file system's root nor `home`, and a directory holding the file.
/// Paths compare by their standardized components, `.` and `..` resolved lexically, never through the file system.
/// On Windows an absolute path names its drive or share (`C:\`, `\\server\share\`), whose root a grant may not be
/// either, its volume compares in any of its spellings ([`volume`]: verbatim or not, a share's server as an address
/// carries it), and names compare in any case, as the file system does.
#[must_use]
pub fn grant_error(file: &str, grant_root: Option<&str>, home: &str) -> Option<&'static str> {
    if !is_absolute(file) {
        return Some(FILE_NOT_ABSOLUTE);
    }
    let grant_root = grant_root?;
    if !is_absolute(grant_root) {
        return Some(CWD_NOT_ABSOLUTE);
    }
    let root = components(grant_root);
    // a root alone: `/`, or on Windows a drive's or a share's
    if root.len() == 1 || root == components(home) {
        return Some(CWD_ROOT_OR_HOME);
    }
    let file = components(file);
    if root == file {
        return Some(CWD_IS_FILE);
    }
    (!file.starts_with(&root)).then_some(OUTSIDE_CWD)
}

/// Whether `path` lies inside `root`, by standardized components (upstream's `HtmlOverlay.contains`).
#[must_use]
pub fn contains(root: &str, path: &str) -> bool {
    components(path).starts_with(&components(root))
}

/// Whether `path` is absolute on this OS: `/`-rooted on Unix; on Windows rooted on a drive or a share (a `\`-rooted
/// path is relative to the current drive).
#[must_use]
pub fn is_absolute(path: &str) -> bool {
    if cfg!(windows) {
        std::path::Path::new(path).is_absolute()
    } else {
        path.starts_with('/')
    }
}

/// Dot-normalizes an absolute document path without probing the filesystem or changing name casing.
#[must_use]
pub fn normalize_path(path: &str) -> String {
    use std::path::{Component, Path, PathBuf};
    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().into_owned()
}

/// The components of an absolute path as Foundation's `standardizedFileURL.pathComponents` gives them: the root, then
/// each name, with `.` dropped, `..` taking the previous name off, and empty names (repeated or trailing slashes)
/// ignored. On Windows the root is the drive or share in its one spelling ([`volume`]) with its root, both separators
/// split, and every name is lowercased, since the file system compares names in any case.
#[cfg(not(windows))]
fn components(path: &str) -> Vec<String> {
    let mut parts = vec!["/".to_owned()];
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            name => parts.push(name.to_owned()),
        }
    }
    parts
}

#[cfg(windows)]
fn components(path: &str) -> Vec<String> {
    use std::path::Component;
    let path = std::path::Path::new(path);
    let mut parts: Vec<String> = Vec::new();
    let mut root = volume(path).unwrap_or_default();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::CurDir => {}
            Component::RootDir => root.push('\\'),
            Component::ParentDir => {
                parts.pop();
            }
            Component::Normal(name) => parts.push(name.to_string_lossy().to_lowercase()),
        }
    }
    let mut all = vec![root];
    all.extend(parts);
    all
}

/// The URL of an absolute http or https address with a host, `None` for anything else (upstream's
/// `HtmlSource.webURL`, which accepts a URL only when it has a web origin).
#[must_use]
pub fn web_url(text: &str) -> Option<Url> {
    // the URL standard the parser follows repairs `http:host`, `http:/host` and surrounding spaces into an address;
    // Foundation's `URL(string:)` finds no host there, so upstream refuses them
    let (_, rest) = text.split_once(':')?;
    if !rest.starts_with("//") || text.trim() != text {
        return None;
    }
    let url = Url::parse(text).ok()?;
    let web = matches!(url.scheme(), "http" | "https");
    (web && url.host_str().is_some_and(|host| !host.is_empty())).then_some(url)
}

/// Whether `--url` names this machine by one of three exact hosts: 127.0.0.1, `::1` or localhost (upstream's
/// `HtmlSource.isLoopback`). Ranges and aliases never match, so the web view cannot read an accepted host as a
/// different one. The host is compared as written, lowercased and with IPv6 brackets stripped: the URL standard
/// [`Url`] follows rewrites `127.1`, `2130706433` or `0x7f.0.0.1` to `127.0.0.1`, which Foundation, upstream's
/// parser, keeps as written.
#[must_use]
pub fn is_loopback(text: &str) -> bool {
    const HOSTS: [&str; 3] = ["127.0.0.1", "::1", "localhost"];
    // the host the web view will load must be one too: the URL standard reads `\` as `/`, so a raw `a\@localhost`
    // names `localhost` as written but loads `a`
    let Some(parsed) = web_url(text).as_ref().and_then(HtmlOrigin::new) else {
        return false;
    };
    if !HOSTS.contains(&parsed.host.as_str()) {
        return false;
    }
    let Some((_, rest)) = text.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = match host_port.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once(']').map_or(bracketed, |(host, _)| host),
        None => host_port.split_once(':').map_or(host_port, |(host, _)| host),
    };
    HOSTS.contains(&host.to_ascii_lowercase().as_str())
}

/// Where `session.overlay.navigate` takes a page (upstream's `HtmlNavigation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HtmlNavigation {
    Back,
    Forward,
    /// Open the page in the default web browser.
    Browser,
    /// Show a file page in the file manager.
    Finder,
}

impl HtmlNavigation {
    /// Parses the wire spelling.
    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "back" => Some(Self::Back),
            "forward" => Some(Self::Forward),
            "browser" => Some(Self::Browser),
            "finder" => Some(Self::Finder),
            _ => None,
        }
    }

    /// The wire spelling.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Forward => "forward",
            Self::Browser => "browser",
            Self::Finder => "finder",
        }
    }
}

/// What a page overlay shows (upstream's `HtmlSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HtmlSource {
    /// An absolute file. `grant_root` is the directory the web view may read from and the page's base; `None` grants
    /// no file access, so the frontend loads the file's text instead (a single-file grant would widen to its folder).
    File { path: String, grant_root: Option<String> },
    /// A web page, pinned to its origin by the navigation policy.
    Url(Url),
    /// A bounded raster image embedded without a file grant or page bridge.
    Image { path: String },
    /// A bounded local Markdown document, rendered by the trusted helper without authored scripts.
    Markdown { path: String },
    /// A source, config, text or log file shown read-only, scrolled to `line` when one is set: no file grant, bridge,
    /// link or navigation, but themed (upstream's `.text`).
    Text { path: String, line: Option<i64> },
    /// An HTML file a terminal click named, so untrusted (upstream's `.document`): the app reads its text and loads it
    /// with no file grant, bridge or page JavaScript, behind a policy and a network filter that load nothing from the
    /// network, themed like a file page. Never a CLI page.
    Document { path: String },
    /// A file page another machine opened, shown here from `ngterm-page://<page id>/` (upstream's `.replica`): `path`
    /// is its path on that machine, whose own grant decides what else may be fetched.
    Replica { path: String },
}

/// The URL scheme a replica page loads from (upstream's `HtmlSource.replicaScheme`, `agterm-page` there; the build's
/// own name, as every scheme it registers).
pub const REPLICA_SCHEME: &str = "ngterm-page";

/// The address replica `page` loads `path` from, `path` being the file's path on the machine that opened it (upstream's
/// `HtmlPageSchemeHandler.url`): the page's own id, lowercased, is the host, so a relative address resolves inside it.
/// The path is a file name, not an address: a literal `%` in it is escaped (as `URLComponents.path` does), so
/// [`replica_path`] gives back the same file; `set_path` escapes the rest (space, `?`, `#`).
#[must_use]
pub fn replica_url(page: uuid::Uuid, path: &str) -> Option<Url> {
    let mut url = Url::parse(&format!("{REPLICA_SCHEME}://{}/", page.hyphenated())).ok()?;
    let escaped = path.replace('%', "%25");
    if escaped.starts_with('/') {
        url.set_path(&escaped);
    } else {
        url.set_path(&format!("/{escaped}"));
    }
    Some(url)
}

/// The file a replica-scheme address names on the machine that opened the page, its path decoded; `None` for an address
/// of another scheme.
#[must_use]
pub fn replica_path(url: &Url) -> Option<String> {
    (url.scheme() == REPLICA_SCHEME)
        .then(|| crate::link_policy::percent_decode(url.path()).unwrap_or_else(|| url.path().to_owned()))
}

/// Whether a replica-scheme address belongs to page `page`: its host is the page's id, in any case (upstream's handler
/// check), so a read naming another page never reaches this one's origin.
#[must_use]
pub fn replica_url_of(url: &Url, page: uuid::Uuid) -> bool {
    url.scheme() == REPLICA_SCHEME
        && url
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case(&page.hyphenated().to_string()))
}

impl HtmlSource {
    /// The origin of an HTTP(S) URL without credentials, `None` for other schemes (upstream's `HtmlSource.origin`).
    #[must_use]
    pub fn origin_of(url: &Url) -> Option<String> {
        HtmlOrigin::new(url).map(|origin| origin.display())
    }

    /// What the source names, as the read-back and the strip's tooltip spell it.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::File { path, .. }
            | Self::Replica { path }
            | Self::Image { path }
            | Self::Markdown { path }
            | Self::Document { path }
            | Self::Text { path, .. } => path.clone(),
            Self::Url(url) => url.as_str().to_owned(),
        }
    }
}

/// The page-zoom ladder the font-size commands walk on an HTML page, one factor for every page (upstream's
/// `HtmlZoom`).
pub mod zoom {
    pub const STEPS: [f64; 11] = [0.5, 0.75, 0.85, 1.0, 1.15, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
    pub const SMALLEST: f64 = 0.5;
    pub const LARGEST: f64 = 3.0;

    /// A stored factor bounded to the ladder's range; `None` and a non-finite value mean actual size.
    #[must_use]
    pub fn resolve(stored: Option<f64>) -> f64 {
        stored
            .filter(|value| value.is_finite())
            .map_or(1.0, |value| value.clamp(SMALLEST, LARGEST))
    }

    /// The next factor from `zoom` for a font binding action (`increase_font_size:1`, `decrease_font_size:1`,
    /// `reset_font_size`), `None` for any other action. An off-ladder factor steps to its neighbor.
    #[must_use]
    pub fn applying(action: &str, zoom: f64) -> Option<f64> {
        // Swift's `split(separator:)` drops empty pieces
        match action.split(':').find(|piece| !piece.is_empty()) {
            Some("increase_font_size") => Some(STEPS.iter().copied().find(|step| *step > zoom).unwrap_or(LARGEST)),
            Some("decrease_font_size") => Some(
                STEPS
                    .iter()
                    .copied()
                    .rev()
                    .find(|step| *step < zoom)
                    .unwrap_or(SMALLEST),
            ),
            Some("reset_font_size") => Some(1.0),
            _ => None,
        }
    }
}

/// The page's load progress as the frontend's web view last reported it (upstream's `HtmlLoadState`). The wire spelling
/// is also a handed-over page's `page.state` `load`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HtmlLoadState {
    #[default]
    Loading,
    Loaded,
    Failed,
}

impl HtmlLoadState {
    /// The wire spelling.
    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::Failed => "failed",
        }
    }
}

/// What a reload loads: the original source (`overlay.reload`, the artifact an agent rewrote) or the page the user
/// navigated to (`--current`, the toolbar button) (upstream's `HtmlReloadTarget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HtmlReloadTarget {
    #[default]
    Original,
    Current,
}

/// The page the web view shows and where its history can go; `page` differs from the source after a navigation
/// (upstream's `HtmlPageInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPageInfo {
    /// A file page's path, else the address.
    pub page: String,
    pub title: Option<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// How a page was opened: its `session.overlay.open` flags (upstream's `HtmlOverlay` init arguments).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "upstream's five independent page flags, each its own wire boolean"
)]
pub struct HtmlPageFlags {
    /// History, reload, browser and source-sharing buttons in the identity strip.
    pub navigation: bool,
    /// The page runs its own scripts; ngterm's theme script runs either way.
    pub javascript: bool,
    /// No identity strip: a file page, or a URL page on a loopback host whose frames stay on its origin.
    pub chromeless: bool,
    /// A URL page on the saved browser store; every other page gets an in-memory one of its own.
    pub persistent: bool,
    /// A URL page's main frame may leave its first origin for any HTTP(S) site.
    pub browse: bool,
}

/// A page occupying an overlay slot (`session.overlay.open --html` or `--url`) in place of a program (upstream's
/// `HtmlOverlay`). `id` travels with the value, so a pane swap or promotion moves the page and the frontend's web view
/// follows it; every frontend report addresses the page by that id. A page this machine opened in a session's slot is
/// persisted with its session ([`HtmlOverlay::persisted`], an extension beyond upstream, which keeps every overlay
/// ephemeral): its source, flags and slot, never its id or what it loaded since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlOverlay {
    pub id: uuid::Uuid,
    /// The page's id on the machine that opened it, set only on a page shown here for it (upstream's `replicaOf`).
    pub replica_of: Option<uuid::Uuid>,
    pub source: HtmlSource,
    pub flags: HtmlPageFlags,
    pub load_state: HtmlLoadState,
    pub load_error: Option<String>,
    /// What the web view shows now, as last reported; `None` until the first load finishes and after a reload of the
    /// original.
    pub current: Option<HtmlPageInfo>,
    /// Bumped by every reload; the frontend reloads what `reload_target` names when it moves.
    pub reload_revision: u64,
    pub reload_target: HtmlReloadTarget,
    /// The page is a page companion's row (upstream's `content == .page`): it never hides, so its strip offers no
    /// Hide, and its Close closes the row.
    pub row: bool,
    /// A terminal click opened the page (upstream's `clicked`), which [`crate::Core::release_hidden_clicked_page`]
    /// lets a later click replace while it is hidden. Local: presentation frames do not carry it, and a snapshot does
    /// not write it.
    pub clicked: bool,
    /// Makes a text page a log page (upstream's `textLog`, `--tail` and `--follow-file`); a snapshot writes this request.
    pub text_log: Option<text_page::TextLog>,
    /// A log page's live state (upstream's `logState`), set exactly when `text_log` is; a snapshot never writes it.
    pub log_state: Option<text_page::TextLogState>,
    /// A text page's search, `None` with none (upstream's `textSearch`); a snapshot never writes it.
    pub text_search: Option<text_page::TextSearch>,
    /// Moves with every query set or closed (upstream's `searchRevision`): a page report naming another revision is
    /// dropped.
    pub search_revision: u64,
    /// A diff page's last report on its changes (upstream's `textChanges`), `None` until it reports and on any other
    /// page; never emitted, and a snapshot never writes it.
    pub text_changes: Option<text_page::TextChanges>,
}

impl HtmlOverlay {
    /// The identity strip's height in points (upstream's `stripHeight`).
    pub const STRIP_HEIGHT: f64 = 28.0;
    /// The strip and the rule under it (upstream's `chromeHeight`).
    pub const CHROME_HEIGHT: f64 = Self::STRIP_HEIGHT + 1.0;

    /// What the panel draws above the page: the strip and its rule, none for a chromeless page (upstream's `chrome`).
    #[must_use]
    pub fn chrome(&self) -> f64 {
        if self.flags.chromeless {
            0.0
        } else {
            Self::CHROME_HEIGHT
        }
    }

    /// A loading page with a fresh id.
    #[must_use]
    pub fn new(source: HtmlSource, flags: HtmlPageFlags) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            replica_of: None,
            source,
            flags,
            load_state: HtmlLoadState::Loading,
            load_error: None,
            current: None,
            reload_revision: 0,
            reload_target: HtmlReloadTarget::Original,
            row: false,
            clicked: false,
            text_log: None,
            log_state: None,
            text_search: None,
            search_revision: 0,
            text_changes: None,
        }
    }

    /// This page as a log page for `log`, following as it asks; `None` leaves a plain page.
    #[must_use]
    pub fn with_text_log(mut self, log: Option<text_page::TextLog>) -> Self {
        self.text_log = log;
        self.log_state = log.map(text_page::TextLogState::new);
        self
    }

    /// Whether the session's snapshot writes the page: one this machine opened in an overlay slot. A replica belongs
    /// to the machine that opened it, and a page companion's row is never persisted.
    #[must_use]
    pub fn persisted(&self) -> bool {
        self.replica_of.is_none() && !self.row && !matches!(self.source, HtmlSource::Replica { .. })
    }

    /// Whether ngterm themes the page: a file page, replica, Markdown, text or clicked HTML document (upstream's
    /// `themed`); a clicked HTML file takes a file page's theme, so it looks the same clicked or opened with `--html`.
    #[must_use]
    pub fn themed(&self) -> bool {
        matches!(
            self.source,
            HtmlSource::File { .. }
                | HtmlSource::Replica { .. }
                | HtmlSource::Markdown { .. }
                | HtmlSource::Text { .. }
                | HtmlSource::Document { .. }
        )
    }

    /// Whether the source receives the control bridge; document renderers never do.
    #[must_use]
    pub fn bridged(&self) -> bool {
        matches!(self.source, HtmlSource::File { .. } | HtmlSource::Replica { .. })
    }

    /// Whether this is a local content page the app reads and builds, with no browser or file access: an image,
    /// Markdown, text or clicked HTML document (upstream's `isContent`).
    #[must_use]
    pub fn content(&self) -> bool {
        matches!(
            self.source,
            HtmlSource::Image { .. }
                | HtmlSource::Markdown { .. }
                | HtmlSource::Text { .. }
                | HtmlSource::Document { .. }
        )
    }

    /// Whether the page loads only once the frontend's network filter is in place, failing with
    /// [`errors::NETWORK_FILTER_UNAVAILABLE`] when it cannot be: a clicked HTML document, whose link hints (preconnect,
    /// DNS prefetch) no CSP covers (upstream's `DocumentRules`).
    #[must_use]
    pub fn blocks_network(&self) -> bool {
        matches!(self.source, HtmlSource::Document { .. })
    }

    /// Whether a page is loaded from generated or file text without a file grant (upstream's `textLoaded`).
    #[must_use]
    pub fn text_loaded(&self) -> bool {
        matches!(
            self.source,
            HtmlSource::File { grant_root: None, .. }
                | HtmlSource::Image { .. }
                | HtmlSource::Markdown { .. }
                | HtmlSource::Text { .. }
                | HtmlSource::Document { .. }
        )
    }

    /// What the panel names the page by, from what the app loaded and never from the page's title, which the page
    /// writes (upstream's `identity`): the file shown, the origin a URL page is confined to, or for a browsing page
    /// the origin of the document it shows now (an address with no web origin, `about:blank`, by that address).
    #[must_use]
    pub fn identity(&self) -> String {
        match &self.source {
            HtmlSource::Image { path }
            | HtmlSource::Markdown { path }
            | HtmlSource::Text { path, .. }
            | HtmlSource::Document { path } => last_path_component(path),
            HtmlSource::File { path, .. } | HtmlSource::Replica { path } => {
                let shown = self
                    .current
                    .as_ref()
                    .map_or(path.as_str(), |current| current.page.as_str());
                last_path_component(shown)
            }
            HtmlSource::Url(url) => {
                let shown = self.current.as_ref().filter(|_| self.flags.browse).map(|current| {
                    Url::parse(&current.page)
                        .ok()
                        .and_then(|url| HtmlSource::origin_of(&url))
                        .unwrap_or_else(|| current.page.clone())
                });
                shown
                    .or_else(|| HtmlSource::origin_of(url))
                    .unwrap_or_else(|| url.as_str().to_owned())
            }
        }
    }
}

/// A `/`-spelled absolute test path as this OS spells it: as is on Unix, on drive `C:` with backslashes on Windows; a
/// relative one keeps its form.
#[cfg(test)]
pub(crate) fn os_path(unix: &str) -> String {
    if cfg!(windows) {
        let backslashed = unix.replace('/', "\\");
        if unix.starts_with('/') {
            format!("C:{backslashed}")
        } else {
            backslashed
        }
    } else {
        unix.to_owned()
    }
}

/// The last component of a path as Foundation's `lastPathComponent` gives it: trailing slashes ignored, `/` for the
/// root.
pub(crate) fn last_path_component(path: &str) -> String {
    // Windows paths separate by either slash
    let separators: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let trimmed = path.trim_end_matches(separators);
    if trimmed.is_empty() {
        return if path.is_empty() { String::new() } else { "/".to_owned() };
    }
    trimmed.rsplit(separators).next().unwrap_or(trimmed).to_owned()
}

/// The frontend's web views, as the dispatcher reaches them (upstream's `HtmlOverlayRegistry` calls in
/// `ControlServer+SessionActions.swift`). A build without one refuses pages.
pub trait HtmlHost {
    /// Reads a bounded raster image before the control path takes an overlay slot.
    ///
    /// # Errors
    /// The upstream image probe failure when the path cannot be read safely.
    fn probe_image(&self, _path: &str) -> Result<image_page::ImageData, String> {
        Err("could not read image".to_owned())
    }

    /// Reads and validates a text page's file before taking a slot (upstream's `textFileRefusal`); rendering reads it
    /// afresh.
    ///
    /// # Errors
    /// The upstream text probe failure (`unreadable text file: P`, `text file too large (over 4 MB): P`, `not a UTF-8
    /// text file: P`).
    fn check_text(&self, path: &str) -> Result<(), String> {
        Err(format!("unreadable text file: {path}"))
    }

    /// Reads a log page's last `tail` lines the way its page will, before taking a slot (upstream's `textFileRefusal`
    /// for a log): only the file's end is read, so its size is never refused, and a binary end is.
    ///
    /// # Errors
    /// The upstream text probe failure (`unreadable text file: P`, `not a UTF-8 text file: P`).
    fn check_log(&self, path: &str, _tail: usize) -> Result<(), String> {
        Err(format!("unreadable text file: {path}"))
    }

    /// Checks a Markdown source descriptor before taking a slot; rendering reads it afresh.
    ///
    /// # Errors
    /// The upstream Markdown probe failure when the descriptor is not a bounded regular file.
    fn check_markdown(&self, _path: &str) -> Result<(), String> {
        Err("could not read markdown file".to_owned())
    }

    /// The current page zoom and the backing scale of the window opening a natural-sized picture.
    fn page_zoom(&self) -> f64 {
        1.0
    }
    fn display_scale(&self, _window: uuid::Uuid) -> f64 {
        1.0
    }

    /// Why no web view can run here, `None` when one can; asked before any page is accepted, so a web engine that
    /// cannot start is refused to the caller rather than met when the page mounts. Upstream has no such case: its web
    /// views always run.
    fn web_views_unavailable(&self) -> Option<String> {
        None
    }

    /// Why a persistent page cannot get the saved browser store, `None` when it can (upstream's
    /// `persistentStoreFailure`); asked before a persistent page is accepted, so the refusal reaches the caller.
    fn persistent_store_failure(&self) -> Option<String>;

    /// Performs a history step or hand-off on page `page`'s web view (upstream's `HtmlOverlayRegistry.navigate`).
    ///
    /// # Errors
    ///
    /// The control error text ([`errors`]): the page has no web view yet, no history that way, no browser, or a
    /// file-manager hand-off of a URL page.
    fn navigate(&self, page: uuid::Uuid, navigation: HtmlNavigation) -> Result<(), String>;

    /// Starts removing everything the saved browser store holds (upstream's `clearPersistentStore`, its refusals in
    /// the order [`profile::clear_check`] gives them): [`StoreClear::Nothing`] when no store was ever created,
    /// [`StoreClear::Started`] while the web engine empties it, until [`HtmlHost::saved_store_clear`] stops answering
    /// [`ClearProgress::Running`].
    ///
    /// # Errors
    ///
    /// Why the store cannot be cleared now, the text `browser.clear` reports after its own name.
    fn clear_saved_store(&self) -> Result<StoreClear, String> {
        Err(errors::PERSISTENT_UNAVAILABLE.to_owned())
    }

    /// How the clear [`HtmlHost::clear_saved_store`] started is going; a failure is answered once.
    fn saved_store_clear(&self) -> ClearProgress {
        ClearProgress::Done
    }

    /// Steps text page `page`'s search once (upstream's `TextPageBridge.step`): its renderer applies `step`'s query
    /// first when it holds an older revision, moves to the next or previous match and answers the count, once, on the
    /// channel handed back. `None`, as a channel that closes unanswered, when the page's renderer is not up to take it.
    fn step_text_search(&self, _page: uuid::Uuid, _step: SearchStep) -> Option<SearchAnswer> {
        None
    }

    /// Steps diff page `page` to its next (`forward`) or previous change once (upstream's `TextPageBridge.stepChange`):
    /// its renderer moves and answers its count and the change it reached, once, on the channel handed back. `None`, as
    /// a channel that closes unanswered, when the page's renderer is not up to take it.
    fn step_text_changes(&self, _page: uuid::Uuid, _forward: bool) -> Option<ChangesAnswer> {
        None
    }
}

/// One step of a text page's search: the direction, and the model's query and its revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchStep {
    pub forward: bool,
    pub query: String,
    pub revision: u64,
}

/// What a text page's renderer counted: its matches and the one it reached, from 1 and 0 for none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchCount {
    pub matches: usize,
    pub current: usize,
}

/// The renderer's answer to a [`SearchStep`], once; a channel that closes unanswered means the page could not take it.
pub type SearchAnswer = std::sync::mpsc::Receiver<SearchCount>;

/// A diff page renderer's answer to a change step, once; a channel that closes unanswered means the page could not
/// take it.
pub type ChangesAnswer = std::sync::mpsc::Receiver<text_page::TextChanges>;

/// How a clear of the saved browser store is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClearProgress {
    /// The web engine is still emptying the store.
    Running,
    /// The store is empty.
    Done,
    /// The web engine reported it could not empty the store: its error text, which `browser.clear` reports after its
    /// own name (upstream's `removeData` cannot fail; the web engine's clear can).
    Failed(String),
}

/// What [`HtmlHost::clear_saved_store`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreClear {
    /// No saved store was ever created: nothing to remove, and none is created for it.
    Nothing,
    /// The web engine is emptying the store.
    Started,
}

/// The page refusals only the frontend's host can tell (upstream's `OverlayHtmlError`).
pub mod errors {
    use super::HtmlNavigation;

    /// The page has not been shown yet, so there is no web view to drive.
    pub const NOT_REALIZED: &str = "html overlay not realized";
    pub const NO_BROWSER: &str = "no default web browser to open the page in";
    pub const MARKDOWN_IN_OVERLAY: &str = "a markdown page opens only in its overlay";
    /// A text page is built by the app, so a browser would get the raw file instead (upstream's `textInOverlay`).
    pub const TEXT_IN_OVERLAY: &str = "a text page opens only in its overlay";
    /// A clicked HTML file came from terminal output, and a browser would run its scripts (upstream's
    /// `documentInOverlay`).
    pub const DOCUMENT_IN_OVERLAY: &str = "a clicked html page opens only in its overlay";
    /// A clicked HTML document loads only behind its network filter, which the web engine refused (upstream's
    /// `DocumentRules.unavailable`).
    pub const NETWORK_FILTER_UNAVAILABLE: &str = "the network filter for a clicked html page could not be installed";
    pub const FINDER_REQUIRES_FILE: &str = "show in Finder requires a file page";
    pub const PERSISTENT_UNAVAILABLE: &str = "persistent browser storage is not available";
    /// The saved store is being emptied: neither a second clear nor a persistent page may use it (upstream's
    /// `BrowserClearError.clearing`).
    pub const CLEARING: &str = "browser storage is being cleared";
    /// A file page shown for another machine has no file here to reveal or open (upstream's
    /// `OverlayHtmlError.remoteFile`).
    pub const REMOTE_FILE: &str = "the page's file is on another Mac";
    /// A page shown here for another machine is answered there, through the page itself (upstream's
    /// `OverlayHtmlError.replicaSubmit`).
    pub const REPLICA_SUBMIT: &str = "submit on the Mac that opened the page";

    /// Persistent pages still hold the saved store and would write their logins back (upstream's
    /// `BrowserClearError.pagesOpen`).
    #[must_use]
    pub fn pages_open(count: usize) -> String {
        format!(
            "{count} persistent {} still open",
            if count == 1 { "page" } else { "pages" }
        )
    }

    /// The page has nowhere to go in that direction.
    #[must_use]
    pub fn no_history(navigation: HtmlNavigation) -> String {
        format!("no page to go {} to", navigation.wire_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> String {
        os_path("/Users/me")
    }

    /// Upstream's `HtmlOverlayTests.grantContainment`.
    #[test]
    fn a_grant_must_contain_the_file() {
        for (file, grant, valid) in [
            ("/a/b/report.html", None, true),
            ("/a/b/report.html", Some("/a/b"), true),
            ("/a/b/report.html", Some("/a"), true),
            ("/a/b/report.html", Some("/a/b/report.html"), false),
            ("/a/b/./report.html", Some("/a/b/report.html/"), false),
            ("/a/bc/report.html", Some("/a/b"), false),
            ("/x/report.html", Some("/a"), false),
            ("relative/report.html", None, false),
            ("/a/report.html", Some("relative"), false),
        ] {
            let (file, grant) = (os_path(file), grant.map(os_path));
            assert_eq!(
                grant_error(&file, grant.as_deref(), &home()).is_none(),
                valid,
                "{file} {grant:?}"
            );
        }
    }

    /// Upstream's `HtmlOverlayTests.grantRefusesRootAndHome`.
    #[test]
    fn a_grant_is_never_the_root_or_home() {
        for (grant, valid) in [
            ("/", false),
            ("/Users/me", false),
            ("/Users/me/", false),
            ("/Users/me/./", false),
            ("/Users/me/docs", true),
            ("/Users", true),
        ] {
            let error = grant_error(&os_path("/Users/me/docs/report.html"), Some(&os_path(grant)), &home());
            assert_eq!(error.is_none(), valid, "{grant}");
            if !valid {
                assert_eq!(error, Some(CWD_ROOT_OR_HOME));
            }
        }
    }

    #[test]
    fn each_refusal_names_its_reason() {
        let error =
            |file: &str, grant: Option<&str>| grant_error(&os_path(file), grant.map(os_path).as_deref(), &home());
        assert_eq!(error("x.html", None), Some(FILE_NOT_ABSOLUTE));
        assert_eq!(error("/a/x.html", Some("a")), Some(CWD_NOT_ABSOLUTE));
        assert_eq!(error("/a/x.html", Some("/a/x.html")), Some(CWD_IS_FILE));
        assert_eq!(error("/b/x.html", Some("/a")), Some(OUTSIDE_CWD));
        assert_eq!(error("/a/../b/x.html", Some("/b")), None, ".. resolves lexically");
        assert!(contains(&os_path("/a"), &os_path("/a/b/c")) && !contains(&os_path("/a/b"), &os_path("/a/c")));
    }

    /// Windows paths: a drive or share is the root, `\\`-rooted paths are relative to the current drive, either slash
    /// separates and names compare in any case.
    #[test]
    #[cfg(windows)]
    fn windows_paths_are_rooted_on_a_drive_or_share() {
        let home = r"C:\Users\me";
        assert_eq!(grant_error(r"\a\x.html", None, home), Some(FILE_NOT_ABSOLUTE));
        assert_eq!(grant_error("/a/x.html", None, home), Some(FILE_NOT_ABSOLUTE));
        assert_eq!(grant_error(r"C:\a\x.html", Some(r"\a"), home), Some(CWD_NOT_ABSOLUTE));
        assert_eq!(grant_error(r"D:\a\x.html", Some(r"D:\"), home), Some(CWD_ROOT_OR_HOME));
        assert_eq!(
            grant_error(r"C:\Users\Me\x.html", Some(r"c:/users/ME/"), home),
            Some(CWD_ROOT_OR_HOME)
        );
        assert_eq!(
            grant_error(r"\\host\share\a\x.html", Some(r"\\host\share\"), home),
            Some(CWD_ROOT_OR_HOME)
        );
        assert_eq!(
            grant_error(r"\\host\share\a\x.html", Some(r"\\host\share\a"), home),
            None
        );
        assert_eq!(grant_error(r"C:\Site\Page.HTML", Some("c:/site"), home), None);
        assert_eq!(
            grant_error(r"D:\site\page.html", Some(r"C:\site"), home),
            Some(OUTSIDE_CWD)
        );
        assert_eq!(
            grant_error(r"C:\site\..\..\page.html", Some(r"C:\site"), home),
            Some(OUTSIDE_CWD)
        );
        assert!(contains(r"C:\Site", "c:/site/sub/x.png"));
        assert_eq!(last_path_component(r"C:\site\page.html"), "page.html");
        assert_eq!(last_path_component(r"C:\site\sub\"), "sub");
    }

    /// A grant and a file name their volume in any of its spellings: verbatim or plain, a share's server as the user
    /// typed it or as a page's address carries it (lowercased, punycoded).
    #[test]
    #[cfg(windows)]
    fn a_windows_volume_compares_in_any_of_its_spellings() {
        let home = r"C:\Users\me";
        for (file, grant) in [
            (r"C:\site\index.html", r"\\?\C:\site"),
            (r"\\?\c:\site\index.html", r"C:\site"),
            (r"\\server\share\site\index.html", r"\\?\UNC\Server\Share\site"),
            (r"\\xn--as-yja\share\site\index.html", r"\\ÑAS\share\site"),
            (r"\\?\UNC\ñas\Share\site\index.html", r"\\xn--as-yja\share\site"),
            (r"\\127.0.0.1\share\site\index.html", r"\\localhost\share\site"),
        ] {
            assert_eq!(grant_error(file, Some(grant), home), None, "{file} {grant}");
            assert!(contains(grant, file), "{file} {grant}");
        }
        for (file, grant) in [
            (r"D:\site\index.html", r"\\?\C:\site"),
            (r"\\other\share\site\index.html", r"\\?\UNC\server\share\site"),
            (r"\\server\other\site\index.html", r"\\server\share\site"),
            (r"\\localhost\C$\site\index.html", r"C:\site"),
            // the URL parser would read these as `nas`; Windows reads them as names of their own
            (r"\\n%61s\share\site\index.html", r"\\nas\share\site"),
            (r"\\nas\share\site\index.html", r"\\n%61s\share\site"),
            (r"\\ｎａｓ\share\site\index.html", r"\\nas\share\site"),
        ] {
            assert_eq!(
                grant_error(file, Some(grant), home),
                Some(OUTSIDE_CWD),
                "{file} {grant}"
            );
            assert!(!contains(grant, file), "{file} {grant}");
        }
        assert_eq!(
            grant_error(r"C:\x.html", Some(r"\\?\C:\"), home),
            Some(CWD_ROOT_OR_HOME)
        );
        assert_eq!(
            grant_error(r"C:\Users\me\x.html", Some(r"\\?\C:\Users\Me"), home),
            Some(CWD_ROOT_OR_HOME)
        );
    }

    /// Upstream's `HtmlOverlayTests.webURLAcceptsOnlyAbsoluteHttpWithAHost`.
    #[test]
    fn web_urls_are_absolute_http_with_a_host() {
        for (text, valid) in [
            ("http://localhost:5173/", true),
            ("https://example.com/docs?q=1#top", true),
            ("HTTPS://example.com", true),
            ("http://127.0.0.1:8000", true),
            ("http://[::1]:8080/", true),
            ("ftp://example.com/", false),
            ("file:///tmp/r.html", false),
            ("localhost:5173", false),
            ("/tmp/r.html", false),
            ("http://", false),
            ("javascript:alert(1)", false),
            ("", false),
            ("page.html", false),
            ("http:example.com", false),
            ("http:/example.com", false),
            (" http://example.com", false),
        ] {
            assert_eq!(web_url(text).is_some(), valid, "{text}");
        }
    }

    /// Upstream's `HtmlOverlayTests.isLoopbackMatchesOnlyTheExactLoopbackHosts`, plus spellings the URL standard
    /// would rewrite to 127.0.0.1 and a user-info part.
    #[test]
    fn is_loopback_matches_only_the_exact_loopback_hosts() {
        for (text, loopback) in [
            ("http://127.0.0.1:8792/", true),
            ("https://127.0.0.1/app", true),
            ("http://[::1]:8792/", true),
            ("http://localhost:5173/", true),
            ("http://LOCALHOST/", true),
            ("http://127.0.0.2/", false),
            ("http://127.1/", false),
            ("http://2130706433/", false),
            ("http://0.0.0.0:8792/", false),
            ("http://[::ffff:127.0.0.1]/", false),
            ("http://localhost./", false),
            ("http://app.localhost/", false),
            ("http://localhost.example.com/", false),
            ("http://example.com/", false),
            ("ftp://127.0.0.1/", false),
            ("localhost:5173", false),
            ("", false),
            // beyond upstream's table: what WHATWG normalizes stays refused, the host after user info counts
            ("http://0x7f.0.0.1/", false),
            ("http://[0:0:0:0:0:0:0:1]/", false),
            ("http://127.0.0.1@example.com/", false),
            ("http://user@localhost:5173/", true),
            ("http://localhost?x=1", true),
            ("http://localhost#top", true),
            ("http://example.com\\@localhost/", false),
        ] {
            assert_eq!(is_loopback(text), loopback, "{text}");
        }
    }

    #[test]
    fn navigations_round_trip() {
        for navigation in [
            HtmlNavigation::Back,
            HtmlNavigation::Forward,
            HtmlNavigation::Browser,
            HtmlNavigation::Finder,
        ] {
            assert_eq!(HtmlNavigation::from_wire(navigation.wire_name()), Some(navigation));
        }
        assert_eq!(HtmlNavigation::from_wire("sideways"), None);
    }

    /// Upstream's `HtmlPageSchemeHandler.url` and its handler's host check.
    #[test]
    fn a_replica_address_names_its_page_and_carries_the_origin_path() {
        let page = uuid::Uuid::new_v4();
        let url = replica_url(page, "/Users/u/my docs/é.html").unwrap();
        assert_eq!(url.scheme(), REPLICA_SCHEME);
        assert_eq!(url.host_str(), Some(page.hyphenated().to_string().as_str()));
        assert_eq!(replica_path(&url).as_deref(), Some("/Users/u/my docs/é.html"));
        assert!(replica_url_of(&url, page));
        let relative = url.join("img/a b.png").unwrap();
        assert_eq!(replica_path(&relative).as_deref(), Some("/Users/u/my docs/img/a b.png"));
        assert!(replica_url_of(&relative, page));
        let upper = Url::parse(&format!(
            "{REPLICA_SCHEME}://{}/a.css",
            page.hyphenated().to_string().to_uppercase()
        ))
        .unwrap();
        assert!(replica_url_of(&upper, page), "the host compares in any case");
        assert!(!replica_url_of(&url, uuid::Uuid::new_v4()), "another page's address");
        let other = Url::parse("ngterm-file:///Users/u/a.html").unwrap();
        assert!(replica_path(&other).is_none() && !replica_url_of(&other, page));
        assert_eq!(
            replica_url(page, "index.html")
                .map(|url| url.path().to_owned())
                .as_deref(),
            Some("/index.html")
        );
    }

    /// A file name is not an address: escapes and URL delimiters in it name the same file again.
    #[test]
    fn a_replica_address_keeps_a_file_name_with_escapes_and_delimiters() {
        let page = uuid::Uuid::new_v4();
        for path in [
            "/docs/a%20b.html",
            "/docs/100%.html",
            "/docs/q?x=1#frag.html",
            "/docs/%E9 é.html",
        ] {
            let url = replica_url(page, path).unwrap();
            assert_eq!(replica_path(&url).as_deref(), Some(path), "{url}");
            assert!(url.query().is_none() && url.fragment().is_none(), "{url}");
        }
        let url = replica_url(page, "/docs/a%20b.html").unwrap();
        assert_eq!(url.path(), "/docs/a%2520b.html");
    }
}
