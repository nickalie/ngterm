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

/// What a Markdown page may follow links within (upstream's `MarkdownLinkGrant`): its file and that file's folder, both
/// with symlinks resolved when the page opens and fixed for its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownLinkGrant {
    pub path: String,
    pub root: String,
}

/// A decoded image reference and its lexically-contained candidate path; canonical containment is still required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReference {
    pub reference: String,
    pub path: PathBuf,
}

/// A decoded image reference of a Markdown file on another machine and its lexically-contained candidate path there,
/// spelled as that machine spells paths; that machine still resolves it before it serves a picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteImageReference {
    pub reference: String,
    pub path: String,
}

/// The picture references of `markdown` lexically inside `folder`, in document order, each decoded reference once
/// (upstream's `imageReferences` without its symlink half). No filesystem access: the local page's reader resolves
/// each candidate before it reads it.
#[must_use]
pub fn image_references(markdown: &str, folder: &Path) -> Vec<ImageReference> {
    decoded_references(markdown)
        .filter_map(|reference| {
            let path = lexical_path(&reference, folder)?;
            Some(ImageReference { reference, path })
        })
        .collect()
}

/// The picture references of the Markdown file `markdown` that another machine holds at `file`, lexically inside its
/// folder there, in document order, each decoded reference once (upstream's `remoteImageReferences`). They are listed
/// by that machine's path rules, read from `file`'s own shape (`origin_path`), not by this OS's, so a viewer on Windows
/// lists a POSIX machine's pictures and the other way round; none for a `file` that is absolute on no machine. That
/// machine's page channel resolves symlinks before it serves a picture. A Markdown-spelled reference is left out: that
/// machine serves it as text.
#[must_use]
pub fn remote_image_references(markdown: &str, file: &str) -> Vec<RemoteImageReference> {
    let Some(folder) = super::origin_path::OriginFolder::of_file(file) else {
        return Vec::new();
    };
    decoded_references(markdown)
        .filter(|reference| !is_markdown(Path::new(reference)))
        .filter_map(|reference| {
            let path = folder.lexical_path(&reference)?;
            Some(RemoteImageReference { reference, path })
        })
        .collect()
}

/// What a page opened on `path` may follow links within (upstream's `linkGrant`), `None` when its links stay text: a
/// path that is not absolute, a target that is not Markdown, or a folder that is a file system root or `home`, all
/// judged with symlinks resolved.
#[must_use]
pub fn link_grant(path: &str, home: &str) -> Option<MarkdownLinkGrant> {
    if !super::is_absolute(path) {
        return None;
    }
    let file = super::real_path(Path::new(path));
    let root = file.parent()?;
    if !is_markdown(&file) || root.parent().is_none() || root == super::real_path(Path::new(home)) {
        return None;
    }
    Some(MarkdownLinkGrant {
        path: file.to_string_lossy().into_owned(),
        root: root.to_string_lossy().into_owned(),
    })
}

/// The path a link page reads for `requested`, an absolute path already percent-decoded (upstream's
/// `linkedDocument`): the request normalized, so its pictures come from the folder its links resolve against. `None`
/// unless both the request and its symlink-resolved target are Markdown strictly inside the grant's root, so a
/// `guide.md` alias of a `.env` is refused.
#[must_use]
pub fn linked_document(requested: &str, grant: &MarkdownLinkGrant) -> Option<String> {
    if !super::is_absolute(requested) || !is_markdown(Path::new(requested)) {
        return None;
    }
    let root = Path::new(&grant.root);
    let lexical = lexical_path(requested, root)?;
    let file = super::real_path(&lexical);
    (is_markdown(&file) && super::strictly_inside(&file, root)).then(|| lexical.to_string_lossy().into_owned())
}

/// Whether `path` names a Markdown file by its extension, in any case.
#[must_use]
pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            crate::link_policy::MARKDOWN_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// The decoded picture references of `markdown`, in document order, each once.
fn decoded_references(markdown: &str) -> impl Iterator<Item = String> + '_ {
    let mut seen = HashSet::new();
    Parser::new_ext(markdown, Options::all()).filter_map(move |event| match event {
        Event::Start(Tag::Image { dest_url, .. }) => {
            let reference = crate::link_policy::percent_decode(&dest_url).unwrap_or_else(|| dest_url.into_string());
            seen.insert(reference.clone()).then_some(reference)
        }
        _ => None,
    })
}

/// One decoded reference joined to `folder` and normalized without reading the filesystem (upstream's `lexicalPath`):
/// `None` unless the result lies strictly inside the folder, so no `~`, no scheme and no `..` escape.
#[must_use]
pub fn lexical_path(reference: &str, folder: &Path) -> Option<PathBuf> {
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

/// One decoded reference resolved against `folder` (upstream's `localPath`): `None` unless it names a file inside it
/// once the symlinks on both sides are resolved. The lexical check runs first, so a reference outside the folder is
/// refused without touching the filesystem.
#[must_use]
pub fn local_path(reference: &str, folder: &Path) -> Option<PathBuf> {
    let lexical = lexical_path(reference, folder)?;
    let root = super::real_path(&normalized(folder));
    let file = super::real_path(&lexical);
    super::strictly_inside(&file, &root).then_some(file)
}

pub(super) fn has_scheme(reference: &str) -> bool {
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
/// `links` lets the renderer keep relative links to neighbouring Markdown files (`data-links`).
#[must_use]
pub fn html(markdown: &str, images: &Embedded, style: &str, links: bool) -> String {
    let map = serde_json::to_string(images).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" \
         content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><style>{style}</style></head><body>\
         <div id=\"ngterm-markdown\" hidden data-images=\"{}\"{}>{}</div>\
         <main id=\"ngterm-page\"></main></body></html>",
        escaped(&map),
        if links { " data-links=\"local\"" } else { "" },
        escaped(markdown)
    )
}

#[cfg(test)]
#[path = "markdown_page_tests.rs"]
mod tests;
