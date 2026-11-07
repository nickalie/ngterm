//! The page a text overlay shows (upstream's `TextPage.swift`): a source, config, log or other text file named by
//! untrusted output, shown read-only. The text travels as escaped, inert data and the page loads nothing; the trusted
//! renderer (highlight.js and `text.js`, in the backend's isolated world) builds the gutter, the highlighted code and
//! the marked line from it.

use std::fmt::Write as _;

use super::markdown_page::escaped;

/// The most a text page reads of its file.
pub const MAX_BYTES: usize = 4 << 20;
/// The most text the page highlights; a larger file renders plain. Highlighting measured about 0.23 s over plain at
/// 256 KiB in `WebKit`, and 1.2 s at 1 MiB (upstream's measurement).
pub const MAX_HIGHLIGHT_BYTES: usize = 256 << 10;

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
    ("diff", "diff"),
    ("patch", "diff"),
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
/// `div`: the parser drops a newline right after `<pre>`, which would shift every line number.
#[must_use]
pub fn html(text: &str, language: Option<&str>, line: Option<i64>, style: &str) -> String {
    let mut attributes = String::new();
    if let Some(language) = language.filter(|_| text.len() <= MAX_HIGHLIGHT_BYTES) {
        let _ = write!(attributes, " data-language=\"{}\"", escaped(language));
    }
    if let Some(line) = line {
        let _ = write!(attributes, " data-line=\"{line}\"");
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

#[cfg(test)]
#[path = "text_page_tests.rs"]
mod tests;
