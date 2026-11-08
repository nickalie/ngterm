//! The page a text overlay shows (upstream's `TextPage.swift`): a source, config, log or other text file named by
//! untrusted output, shown read-only. The text travels as escaped, inert data and the page loads nothing; the trusted
//! renderer (highlight.js and `text.js`, in the backend's isolated world) builds the gutter, the highlighted code and
//! the marked line from it. A log page is a text page that shows only its file's last lines ([`TextLog`]), following
//! the end as it grows; its live state is [`TextLogState`].

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::markdown_page::escaped;

/// The most a text page reads of its file.
pub const MAX_BYTES: usize = 4 << 20;
/// The most text the page highlights; a larger file renders plain. Highlighting measured about 0.23 s over plain at
/// 256 KiB in `WebKit`, and 1.2 s at 1 MiB (upstream's measurement).
pub const MAX_HIGHLIGHT_BYTES: usize = 256 << 10;
/// How many lines a log page keeps when the request names none.
pub const DEFAULT_TAIL: usize = 5_000;
/// The most lines a log page may ask for.
pub const MAX_TAIL: usize = 100_000;
/// The most a log page reads from its file's end, so one huge line cannot pull in the file.
pub const MAX_TAIL_BYTES: usize = 2 << 20;

/// Lowercased extensions a plain click opens as a log page, following its end.
const LOG_EXTENSIONS: &[&str] = &["log", "out"];
/// The language a patch maps to, whose plain page steps between its changes.
const DIFF_LANGUAGE: &str = "diff";

/// A text page's request to show only a log's last `tail` lines, following its end when `follow` is set (upstream's
/// `TextLog`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextLog {
    pub tail: usize,
    pub follow: bool,
}

/// What the app last read for a log page (upstream's `TextLogReport`): the lines shown, a clipped first line, a file
/// that is missing (`waiting`) or turned binary (`unreadable`, the last good text still shown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextLogReport {
    pub lines: usize,
    pub clipped: bool,
    pub waiting: bool,
    pub unreadable: bool,
}

/// A log page's live state (upstream's `TextLogState`): whether it follows the file, and the app's last report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextLogState {
    pub following: bool,
    pub report: TextLogReport,
}

impl TextLogState {
    /// The state a page opened with `log` starts in: following as requested, nothing read yet.
    #[must_use]
    pub fn new(log: TextLog) -> Self {
        Self {
            following: log.follow,
            report: TextLogReport::default(),
        }
    }
}

/// A text page's search (upstream's `TextSearch`): the query and the page's last report on it, `current` counted from 1
/// and 0 with no match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextSearch {
    pub query: String,
    pub matches: usize,
    pub current: usize,
}

impl TextSearch {
    /// A query the page has not counted yet.
    #[must_use]
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Self::default()
        }
    }
}

/// A diff page's count of changes, a change being one run of removed and added lines, and the one it last stepped to,
/// from 1 and 0 before any step (upstream's `TextChanges`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextChanges {
    pub count: usize,
    pub current: usize,
}

/// The search a built page starts with (upstream's `TextPage.SearchState`): the model's query (empty with none) and its
/// revision, and the match the page showed last, from 1 and 0 for none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchState {
    pub query: String,
    pub revision: u64,
    pub current: usize,
}

/// What a built page starts from beyond its file: a log page's state, which replaces the line, and `anchor`, the scroll
/// offset a rebuilt page reopens at (a paused log page's or a plain page's; upstream's `html` `log` and `anchor`), and
/// the page's search, which a rebuild keeps (upstream's `search`).
/// `load` is the document's build generation, which the renderer echoes in its ready report, so a ready report from an
/// older document cannot release a newer one's build (a fix over upstream, whose ready report names no document).
/// `change` is the change a rebuilt diff page reopens on, from 1 (upstream's `change`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageState<'a> {
    pub log: Option<&'a TextLogState>,
    pub anchor: Option<f64>,
    pub load: Option<u64>,
    pub search: Option<&'a SearchState>,
    pub change: Option<usize>,
}

/// Lowercased extensions and the bundled highlight.js language each maps to.
const LANGUAGES: &[(&str, &str)] = &[
    ("swift", "swift"),
    ("go", "go"),
    ("py", "python"),
    ("rb", "ruby"),
    ("rs", "rust"),
    ("js", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("ts", "typescript"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("cxx", "cpp"),
    ("hpp", "cpp"),
    ("m", "objectivec"),
    ("mm", "objectivec"),
    ("java", "java"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("cs", "csharp"),
    ("php", "php"),
    ("pl", "perl"),
    ("pm", "perl"),
    ("lua", "lua"),
    ("r", "r"),
    ("sql", "sql"),
    ("css", "css"),
    ("scss", "scss"),
    ("less", "less"),
    ("graphql", "graphql"),
    ("sh", "bash"),
    ("bash", "bash"),
    ("zsh", "bash"),
    ("json", "json"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("toml", "ini"),
    ("ini", "ini"),
    ("cfg", "ini"),
    ("conf", "ini"),
    ("xml", "xml"),
    ("plist", "xml"),
    ("diff", DIFF_LANGUAGE),
    ("patch", DIFF_LANGUAGE),
    ("mk", "makefile"),
    ("vb", "vbnet"),
];

/// Lowercased extensions shown as plain text.
const PLAIN_EXTENSIONS: &[&str] = &["txt", "text", "log", "out", "csv", "tsv", "env", "lock"];

/// Exact file names and the language each maps to.
const BASENAME_LANGUAGES: &[(&str, &str)] = &[
    ("Makefile", "makefile"),
    ("GNUmakefile", "makefile"),
    (".bashrc", "bash"),
    (".zshrc", "bash"),
    (".profile", "bash"),
    (".bash_profile", "bash"),
    (".zprofile", "bash"),
    (".gitconfig", "ini"),
    (".editorconfig", "ini"),
];

/// Exact file names shown as plain text.
const PLAIN_BASENAMES: &[&str] = &[
    "Dockerfile",
    "LICENSE",
    "README",
    "CHANGELOG",
    "Procfile",
    ".gitignore",
    ".env",
];

/// The last component of `path` and its lowercased extension, as Foundation's `lastPathComponent` and `pathExtension`
/// read them: a leading dot starts no extension (`.bashrc` has none).
fn name_and_extension(path: &str) -> (String, String) {
    let name = super::last_path_component(path);
    let extension = std::path::Path::new(&name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    (name, extension)
}

/// The bundled highlight.js language for `path`, by lowercased extension and then by exact file name; `None` means
/// the file shows as plain text, or is not a text page at all (upstream's `language(forFile:)`).
#[must_use]
pub fn language(path: &str) -> Option<&'static str> {
    let (name, extension) = name_and_extension(path);
    LANGUAGES
        .iter()
        .find(|(known, _)| *known == extension)
        .or_else(|| BASENAME_LANGUAGES.iter().find(|(known, _)| *known == name))
        .map(|(_, language)| *language)
}

/// Whether a clicked or requested `path` opens as a text page (upstream's `isEligible`). Markdown, HTML and SVG are not
/// text pages: the first has its own page, the others are documents an author may expect rendered.
#[must_use]
pub fn is_eligible(path: &str) -> bool {
    let (name, extension) = name_and_extension(path);
    language(path).is_some()
        || PLAIN_EXTENSIONS.contains(&extension.as_str())
        || PLAIN_BASENAMES.contains(&name.as_str())
}

/// Whether `path` names a patch, whose plain page steps between its changes (upstream's `isDiff`).
#[must_use]
pub fn is_diff(path: &str) -> bool {
    language(path) == Some(DIFF_LANGUAGE)
}

/// Maps each path a patch's `--- ` or `+++ ` row names, as the row spells it up to a tab, to the file's language, a
/// quoted path looked up without its quotes; a file with no known language is left out (upstream's `diffLanguages`).
/// A changed row can look like a header, so the renderer looks up only rows outside hunks. Rows split at every newline
/// Swift's `isNewline` knows, so a CRLF patch maps its paths too.
#[must_use]
pub fn diff_languages(text: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let rows = text.split(['\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}']);
    for row in rows.filter(|row| row.starts_with("--- ") || row.starts_with("+++ ")) {
        let path = row[4..].split('\t').next().unwrap_or_default();
        let file = if path.chars().count() > 1 && path.starts_with('"') && path.ends_with('"') {
            &path[1..path.len() - 1]
        } else {
            path
        };
        if let Some(language) = language(file) {
            found.insert(path.to_owned(), language.to_owned());
        }
    }
    found
}

/// Whether a plain click on `path` opens it as a log page, following its end (upstream's `isLog`): a `.log` or `.out`
/// file, in any case.
#[must_use]
pub fn is_log(path: &str) -> bool {
    LOG_EXTENSIONS.contains(&name_and_extension(path).1.as_str())
}

/// The file's text, `None` when it is not valid UTF-8 or holds a NUL, which marks a binary (upstream's `validate`).
#[must_use]
pub fn validate(data: Vec<u8>) -> Option<String> {
    if data.contains(&0) {
        return None;
    }
    String::from_utf8(data).ok()
}

/// The document the renderer fills (upstream's `TextPage.html`): the text as the content of a hidden element, the
/// language and target line as attributes, and `style`, the app's own stylesheet. The language is dropped over
/// [`MAX_HIGHLIGHT_BYTES`], so the renderer shows that text plain. The policy loads nothing at all. The holder is a
/// `div`: the parser drops a newline right after `<pre>`, which would shift every line number. A log page (`state.log`
/// set) carries its follow state and report flags instead of a line, and the renderer draws no gutter; a finite
/// `state.anchor` is the scroll offset the rebuilt page reopens at, and `state.load` the build generation the renderer's
/// ready report echoes. A search carries its revision, and with a query the escaped query and the match shown.
/// A plain page of a diff is marked `data-diff` whatever its size, so change navigation outlives the highlighting
/// limit, with `state.change` as the change it reopens on; under the limit it also carries [`diff_languages`] as JSON
/// (`data-files`), so each file's lines are highlighted in that file's language.
#[must_use]
pub fn html(text: &str, language: Option<&str>, line: Option<i64>, style: &str, state: &PageState) -> String {
    let mut attributes = String::new();
    let highlighted = text.len() <= MAX_HIGHLIGHT_BYTES;
    if state.log.is_none() && language == Some(DIFF_LANGUAGE) {
        attributes.push_str(" data-diff");
        if let Some(change) = state.change.filter(|change| *change > 0) {
            let _ = write!(attributes, " data-change=\"{change}\"");
        }
        let files = if highlighted {
            diff_languages(text)
        } else {
            BTreeMap::new()
        };
        if !files.is_empty()
            && let Ok(json) = serde_json::to_string(&files)
        {
            let _ = write!(attributes, " data-files=\"{}\"", escaped(&json));
        }
    }
    if let Some(search) = state.search {
        let _ = write!(attributes, " data-revision=\"{}\"", search.revision);
        if !search.query.is_empty() {
            let _ = write!(
                attributes,
                " data-query=\"{}\" data-current=\"{}\"",
                escaped(&search.query),
                search.current
            );
        }
    }
    if let Some(language) = language.filter(|_| highlighted) {
        let _ = write!(attributes, " data-language=\"{}\"", escaped(language));
    }
    if let Some(log) = state.log {
        let _ = write!(attributes, " data-log data-following=\"{}\"", log.following);
        for (set, name) in [
            (log.report.clipped, "clipped"),
            (log.report.waiting, "waiting"),
            (log.report.unreadable, "unreadable"),
        ] {
            if set {
                let _ = write!(attributes, " data-{name}");
            }
        }
    } else if let Some(line) = line {
        let _ = write!(attributes, " data-line=\"{line}\"");
    }
    if let Some(anchor) = state.anchor.filter(|anchor| anchor.is_finite()) {
        let _ = write!(attributes, " data-anchor=\"{}\"", swift_double(anchor));
    }
    if let Some(load) = state.load {
        let _ = write!(attributes, " data-load=\"{load}\"");
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">\
         <style>{style}</style></head><body>\
         <div id=\"ngterm-text\" hidden{attributes}>{}</div>\
         <main id=\"ngterm-page\"></main></body></html>",
        escaped(text)
    )
}

/// `anchor` as Swift's `Double` description spells it, which the renderer parses back: a whole number keeps its `.0`.
fn swift_double(anchor: f64) -> String {
    let shown = anchor.to_string();
    if shown.contains(['.', 'e', 'E']) {
        shown
    } else {
        format!("{shown}.0")
    }
}

#[cfg(test)]
#[path = "text_page_tests.rs"]
mod tests;
