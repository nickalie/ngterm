//! Trusted document rendering, isolated from authored page JavaScript (upstream's `MarkdownAssets`): the renderers
//! Markdown and text pages run in the backend's named world (a `WKContentWorld`, a WPE script world, a CDP isolated
//! world on `WebView2`), with the page's own JavaScript off, and the stylesheets they wear. A clicked HTML file runs no
//! renderer: its own markup is the page, behind its policy (`document_page`).

use ngterm_core::html::{
    HtmlPageKind, HtmlSource, ReplicaContent, document_page, errors, image_page, markdown_page, text_page,
};
use ngterm_core::remote::replica_page::ContentFailures;

pub(super) const WORLD: &str = "ngterm-markdown";
pub(super) const STYLE: &str = include_str!("../../../../resources/markdown/page.css");
const MARKDOWN: &str = include_str!("../../../../resources/markdown/markdown-it-14.1.0.min.js");
const HIGHLIGHT: &str = include_str!("../../../../resources/markdown/highlight-11.11.1.min.js");
const RENDER: &str = include_str!("../../../../resources/markdown/render.js");
const TEXT: &str = include_str!("../../../../resources/markdown/text.js");
const TEXT_STYLE: &str = include_str!("../../../../resources/markdown/text.css");

/// The Markdown renderer: markdown-it, highlight.js and `render.js`.
pub(super) fn script() -> String {
    [MARKDOWN, HIGHLIGHT, RENDER].join(";\n")
}

/// The text page renderer (upstream's `textScript`): highlight.js and `text.js`, with no Markdown parser. Until the
/// page has a host channel, `text.js`'s posts to the host fail inside its own `try`, and are dropped.
///
/// A log document carries its build generation (`data-load`), which `text.js` (vendored as upstream ships it) does not
/// report: the wrapper reads it before `text.js` takes the source element away, and once `text.js` ran and posted its
/// own `{ready}`, posts `{ready, load}`, the report a log page's follower takes, so an older document's late report
/// cannot release a newer build (a fix over upstream).
pub(super) fn text_script() -> String {
    let handler = super::text_page::HANDLER;
    format!(
        "{HIGHLIGHT};\n(() => {{ const load = Number(document.getElementById('ngterm-text')?.dataset.load);\n{TEXT};\n\
         if (Number.isSafeInteger(load)) {{ try {{ window.webkit.messageHandlers.{handler}.postMessage({{ ready: true, \
         load }}); }} catch (_) {{}} }} }})();"
    )
}

/// The text page's stylesheet (upstream's `textStyle`): the Markdown stylesheet, for the theme colors and highlight
/// rules both pages share, with `text.css` over it.
pub(super) fn text_style() -> String {
    format!("{STYLE}\n{TEXT_STYLE}")
}

/// The renderer `source` runs, at document end, in the backend's named helper world; `None` for a page with none. A
/// replica runs its kind's, which a page built here needs as a local one does (upstream's `installScripts` by kind).
pub(super) fn renderer(source: &HtmlSource) -> Option<String> {
    match source.kind() {
        HtmlPageKind::Markdown => Some(script()),
        HtmlPageKind::Text => Some(text_script()),
        _ => None,
    }
}

/// Why a page `source` shows refuses the browser hand-off: the app built its document, so a browser would get the raw
/// file instead.
pub(super) fn browser_refusal(source: &HtmlSource) -> Option<&'static str> {
    match source {
        HtmlSource::Markdown { .. } => Some(errors::MARKDOWN_IN_OVERLAY),
        HtmlSource::Text { .. } => Some(errors::TEXT_IN_OVERLAY),
        HtmlSource::Document { .. } => Some(errors::DOCUMENT_IN_OVERLAY),
        _ => None,
    }
}

/// The page a clicked HTML file's bytes make for this platform's engine: `WebView2`'s Chromium reaches link hints past
/// the CSP and its request filter, so its page also makes every `<link>` inert and turns DNS prefetch off
/// ([`document_page::chromium_page`]); `WebKit` checks them against the page's content filter.
fn document_page_builder() -> fn(Vec<u8>) -> Option<String> {
    if cfg!(windows) {
        document_page::chromium_page
    } else {
        document_page::page
    }
}

/// The CDP binding a text page's renderer reports through on `WebView2`, which has no message handler in a named world.
#[cfg(windows)]
pub(super) const RENDERER_BINDING: &str = "ngtermTextPageBinding";

/// The renderer's `window.webkit.messageHandlers.ngtermTextPage` on `WebView2`: a shim in its world over
/// [`RENDERER_BINDING`], which carries the report as JSON.
#[cfg(windows)]
fn renderer_channel() -> String {
    let handler = super::text_page::HANDLER;
    format!(
        "window.webkit = {{ messageHandlers: {{ {handler}: {{ postMessage: (body) => \
         {RENDERER_BINDING}(JSON.stringify(body)) }} }} }};"
    )
}

/// `WebView2` installs at document creation and `WKWebView` at document end; the isolated script renders when the DOM
/// is ready, waiting for `DOMContentLoaded` when needed so the inert data elements exist. On `WebView2` a text page's
/// renderer reports through `renderer_channel`'s shim. `None` for a page with no renderer.
#[cfg(any(target_os = "macos", windows))]
pub(super) fn document_start_script(source: &HtmlSource) -> Option<String> {
    #[cfg(windows)]
    let channel = if matches!(source, HtmlSource::Text { .. }) {
        renderer_channel()
    } else {
        String::new()
    };
    #[cfg(not(windows))]
    let channel = "";
    renderer(source).map(|renderer| {
        format!(
            "(() => {{ {channel} const render = () => {{\n{renderer}\n}}; if(document.readyState === 'loading') document.addEventListener('DOMContentLoaded',render,{{once:true}}); else render(); }})();"
        )
    })
}

/// Reads an image, Markdown or clicked HTML page's document on a worker, never through the web engine's file or network
/// permissions.
pub(super) fn read(source: &HtmlSource) -> Result<String, String> {
    match source {
        HtmlSource::Image { path } => {
            let image = ngterm_app::documents::probe_image(std::path::Path::new(path))?;
            Ok(image_page::html(&image.bytes, &image.mime_type))
        }
        HtmlSource::Markdown { path } => {
            let document = ngterm_app::documents::probe_markdown(std::path::Path::new(path))?;
            Ok(markdown_page::html(&document.markdown, &document.images, STYLE))
        }
        HtmlSource::Document { path } => {
            ngterm_app::documents::load_document(std::path::Path::new(path), document_page_builder())
        }
        // a text page's documents are its bridge's (`text_page`), which carry the model's search
        _ => unreachable!("only image, markdown and document sources are read through bounded probes"),
    }
}

/// A blocking read of a file of the machine that opened a page: its path there, the cap and the failures' wording.
pub(super) type OriginRead<'a> = dyn FnMut(&str, usize, ContentFailures) -> Result<Vec<u8>, String> + 'a;

/// Builds the document of a page another machine opened from the bytes `read` brings from that machine (upstream's
/// `replicaContentPage`), on a worker: `read` takes the file's path there, the cap its read is under and how its failures
/// are worded, and blocks until the origin answered. The bytes are judged again here as a local file's would be, since
/// a file can change between the origin's check and this read; a Markdown page's pictures are read the same way under
/// the page's shared budget. A text page is the static one, with no search or log state, as a local page's documents
/// carry.
pub(super) fn build_replica(path: &str, content: ReplicaContent, read: &mut OriginRead<'_>) -> Result<String, String> {
    let Some((limit, failures)) = ngterm_app::documents::replica_read(content, path) else {
        return Err(errors::REMOTE_FILE.to_owned());
    };
    let bytes = read(path, limit, failures)?;
    match content {
        ReplicaContent::Html => Err(errors::REMOTE_FILE.to_owned()),
        ReplicaContent::Image { .. } => {
            let image = ngterm_app::documents::image_from_bytes(bytes, path)?;
            Ok(image_page::html(&image.bytes, &image.mime_type))
        }
        ReplicaContent::Markdown => {
            // a picture that cannot be read is left to its alt text, as a local page's is
            let fetch = |picture: &str, room: usize| read(picture, room, ContentFailures::default()).ok();
            let document = ngterm_app::documents::markdown_from_bytes(bytes, path, fetch)?;
            Ok(markdown_page::html(&document.markdown, &document.images, STYLE))
        }
        ReplicaContent::Text { line } => {
            let text = ngterm_app::documents::text_from_bytes(bytes, path)?;
            let state = text_page::PageState::default();
            Ok(text_page::html(
                &text,
                text_page::language(path),
                line,
                &text_style(),
                &state,
            ))
        }
        ReplicaContent::Document => ngterm_app::documents::document_from_bytes(bytes, path, document_page_builder()),
    }
}

/// A completion from an older load or from a released page must never reach its engine.
#[derive(Debug, Default)]
pub(super) struct Generation(std::cell::Cell<u64>);

impl Generation {
    pub(super) fn next(&self) -> u64 {
        let next = self.0.get().wrapping_add(1);
        self.0.set(next);
        next
    }

    pub(super) fn current(&self, generation: u64) -> bool {
        self.0.get() == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A text page runs the text renderer and refuses the browser hand-off; its documents are its bridge's.
    #[test]
    fn a_text_source_runs_the_text_renderer() {
        let source = HtmlSource::Text {
            path: "/tmp/main.go".to_owned(),
            line: Some(2),
        };
        assert_eq!(browser_refusal(&source), Some(errors::TEXT_IN_OVERLAY));
        assert_eq!(renderer(&source), Some(text_script()));
        assert!(text_style().contains(TEXT_STYLE) && text_style().contains(STYLE));
    }

    fn png(width: u32) -> Vec<u8> {
        use image::ImageEncoder as _;
        let mut bytes = Vec::new();
        image::codecs::png::PngEncoder::new(&mut bytes)
            .write_image(
                &vec![7; (width * 4) as usize],
                width,
                1,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        bytes
    }

    /// Reads from `files` (path to bytes) as an origin's channel would, recording each path and cap asked.
    fn origin(
        files: Vec<(String, Vec<u8>)>,
        asked: &mut Vec<(String, usize)>,
    ) -> impl FnMut(&str, usize, ContentFailures) -> Result<Vec<u8>, String> + '_ {
        move |path: &str, limit: usize, failures: ContentFailures| {
            asked.push((path.to_owned(), limit));
            files
                .iter()
                .find(|(file, _)| file == path)
                .map(|(_, bytes)| bytes.clone())
                .ok_or(failures.unreadable)
        }
    }

    fn os(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        }
    }

    /// Upstream's `testAMarkdownReplicaRendersWithItsFetchedPictureAndNoBridge` at the builder: the page's own file
    /// under its kind's cap, then each picture inside its folder there under the shared budget, embedded; a reference
    /// leaving the folder is never asked for.
    #[test]
    fn a_markdown_replica_builds_with_its_fetched_picture() {
        let (doc, shot) = (os("/o/docs/doc.md"), os("/o/docs/shot.png"));
        let markdown = b"# Remote\n\n![s](shot.png) ![x](../outside.png)\n".to_vec();
        let mut asked = Vec::new();
        let html = build_replica(
            &doc,
            ReplicaContent::Markdown,
            &mut origin(vec![(doc.clone(), markdown), (shot.clone(), png(40))], &mut asked),
        )
        .unwrap();
        assert_eq!(
            asked,
            [(doc, markdown_page::MAX_BYTES), (shot, markdown_page::MAX_IMAGE_BYTES)]
        );
        assert!(html.contains("data:image/png;base64,"), "{html}");
        assert!(html.contains("# Remote"));
        assert_eq!(
            renderer(&HtmlSource::Replica {
                path: String::new(),
                content: ReplicaContent::Markdown
            }),
            Some(script())
        );
    }

    /// An image, text or document replica builds the page a local one of its kind shows from the fetched bytes, judged
    /// here again; a failed read keeps its reason; an HTML replica loads through its scheme, never here.
    #[test]
    fn a_content_replica_builds_its_kinds_page_from_fetched_bytes() {
        let path = os("/o/a");
        let build = |content, bytes: Vec<u8>| {
            let mut asked = Vec::new();
            build_replica(&path, content, &mut origin(vec![(path.clone(), bytes)], &mut asked))
        };
        let image = build(ReplicaContent::Image { width: 3, height: 1 }, png(3)).unwrap();
        assert!(
            image.contains("<img") && image.contains("data:image/png;base64,"),
            "{image}"
        );
        assert_eq!(
            build(ReplicaContent::Image { width: 1, height: 1 }, b"<svg/>".to_vec()).unwrap_err(),
            format!("not a png, jpg, gif or webp image: {path}")
        );
        let text = build(ReplicaContent::Text { line: Some(2) }, b"one\ntwo\n".to_vec()).unwrap();
        assert!(text.contains("two"), "{text}");
        assert_eq!(
            build(ReplicaContent::Text { line: None }, vec![0x61, 0x00]).unwrap_err(),
            format!("not a UTF-8 text file: {path}")
        );
        let document = build(ReplicaContent::Document, b"<p>review</p>".to_vec()).unwrap();
        assert!(document.contains(document_page::POLICY), "{document}");
        let mut refused =
            |_: &str, _: usize, _: ContentFailures| Err("file is too large for the page: /o/a".to_owned());
        assert_eq!(
            build_replica(&path, ReplicaContent::Text { line: None }, &mut refused).unwrap_err(),
            "file is too large for the page: /o/a"
        );
        assert_eq!(
            build(ReplicaContent::Html, Vec::new()).unwrap_err(),
            errors::REMOTE_FILE
        );
        let text_replica = HtmlSource::Replica {
            path: path.clone(),
            content: ReplicaContent::Text { line: None },
        };
        assert_eq!(renderer(&text_replica), Some(text_script()));
    }

    /// A clicked HTML file reads on the worker into its own markup behind the policy, with no renderer; an unreadable
    /// one fails with upstream's probe message, and the browser hand-off is refused.
    #[test]
    fn a_document_source_reads_into_its_own_page_behind_the_policy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr.html");
        std::fs::write(
            &path,
            "<!doctype html><title>pr</title><link rel=preconnect href=http://h>",
        )
        .unwrap();
        let source = HtmlSource::Document {
            path: path.to_string_lossy().into_owned(),
        };
        let html = read(&source).unwrap();
        assert!(
            html.starts_with(&format!("<!doctype html>{}", document_page::POLICY)),
            "{html}"
        );
        assert_eq!(html.contains("<ngterm-inert-link"), cfg!(windows), "{html}");
        assert_eq!(renderer(&source), None);
        assert_eq!(browser_refusal(&source), Some(errors::DOCUMENT_IN_OVERLAY));
        let missing = HtmlSource::Document {
            path: dir.path().join("missing.html").to_string_lossy().into_owned(),
        };
        assert_eq!(
            read(&missing).unwrap_err(),
            format!("unreadable html file: {}", dir.path().join("missing.html").display())
        );
    }
}
