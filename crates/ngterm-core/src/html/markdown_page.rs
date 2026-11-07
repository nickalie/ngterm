//! Inert Markdown documents and lexical picture references (upstream `MarkdownPage.swift`). The app service
//! canonicalizes candidates and performs bounded reads; no filesystem access happens while extracting paths.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::Serialize;

pub const MAX_BYTES: usize = 4 << 20;
pub const MAX_IMAGE_BYTES: usize = 32 << 20;
pub const MAX_IMAGE_PIXELS: u64 = super::image_page::MAX_PIXELS;
pub const MAX_IMAGES: usize = 256;

/// Each canonical picture's data URI once, with decoded source references pointing to that payload's index.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct Embedded {
    pub references: BTreeMap<String, usize>,
    pub uris: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownDocument {
    pub markdown: String,
    pub images: Embedded,
}

/// A decoded image reference and its lexically-contained candidate path; canonical containment is still required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    pub reference: String,
    pub path: PathBuf,
}

#[must_use]
pub fn image_references(markdown: &str, folder: &Path) -> Vec<ImageReference> {
    let mut seen = HashSet::new();
    Parser::new_ext(markdown, Options::all())
        .filter_map(|event| match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                let reference = crate::link_policy::percent_decode(&dest_url).unwrap_or_else(|| dest_url.into_string());
                if !seen.insert(reference.clone()) {
                    return None;
                }
                let path = local_path(&reference, folder)?;
                Some(ImageReference { reference, path })
            }
            _ => None,
        })
        .collect()
}

/// Lexical containment is checked before any canonicalization or filesystem probe can occur.
#[must_use]
pub fn local_path(reference: &str, folder: &Path) -> Option<PathBuf> {
    if !folder.is_absolute() || reference.is_empty() || reference.starts_with('~') {
        return None;
    }
    let reference_path = Path::new(reference);
    if !reference_path.is_absolute() && has_scheme(reference) {
        return None;
    }
    let root = normalized(folder);
    let candidate = normalized(&folder.join(reference_path));
    (candidate != root && super::contains(&root.to_string_lossy(), &candidate.to_string_lossy())).then_some(candidate)
}

fn has_scheme(reference: &str) -> bool {
    let Some((scheme, _)) = reference.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(char::is_alphabetic)
        && chars.all(|ch| ch.is_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            part => out.push(part.as_os_str()),
        }
    }
    out
}

/// Escaping walks Unicode scalars, including punctuation followed by a combining mark.
#[must_use]
pub fn escaped(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            ch => out.push(ch),
        }
    }
    out
}

/// Authored text and the sorted picture index are data, never scripts or authored HTML. The holder is a `div`: the
/// HTML parser drops a newline right after `<pre>`, which would move a `---` up to the first line (frontmatter).
#[must_use]
pub fn html(markdown: &str, images: &Embedded, style: &str) -> String {
    let map = serde_json::to_string(images).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" \
         content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><style>{style}</style></head><body>\
         <div id=\"ngterm-markdown\" hidden data-images=\"{}\">{}</div>\
         <main id=\"ngterm-page\"></main></body></html>",
        escaped(&map),
        escaped(markdown)
    )
}

#[cfg(test)]
#[path = "markdown_page_tests.rs"]
mod tests;
