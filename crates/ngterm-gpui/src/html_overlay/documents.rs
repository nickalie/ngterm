//! Trusted document rendering, isolated from authored page JavaScript (upstream's `MarkdownAssets`): the renderers
//! Markdown and text pages run in the backend's named world (a `WKContentWorld`, a WPE script world, a CDP isolated
//! world on `WebView2`), with the page's own JavaScript off, and the stylesheets they wear. A clicked HTML file runs no
//! renderer: its own markup is the page, behind its policy (`document_page`).

use ngterm_core::html::{HtmlSource, document_page, errors, image_page, markdown_page, text_page};

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

/// The renderer `source` runs, at document end, in the backend's named helper world; `None` for a page with none.
pub(super) fn renderer(source: &HtmlSource) -> Option<String> {
    match source {
        HtmlSource::Markdown { .. } => Some(script()),
        HtmlSource::Text { .. } => Some(text_script()),
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

/// Reads document data on a worker, never through the web engine's file or network permissions.
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
        HtmlSource::Text { path, line } => {
            let text = ngterm_app::documents::load_text(std::path::Path::new(path))?;
            let state = text_page::PageState::default();
            Ok(text_page::html(
                &text,
                text_page::language(path),
                *line,
                &text_style(),
                &state,
            ))
        }
        HtmlSource::Document { path } => {
            ngterm_app::documents::load_document(std::path::Path::new(path), document_page_builder())
        }
        _ => unreachable!("only document sources are read through bounded probes"),
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

    /// A text page reads its file on the worker and builds the inert document with its mapped language and line,
    /// wearing the Markdown stylesheet with `text.css` over it; a binary file fails with upstream's probe message.
    #[test]
    fn a_text_source_reads_into_its_inert_document() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.go");
        std::fs::write(&path, "\npackage <main>\n").unwrap();
        let source = HtmlSource::Text {
            path: path.to_string_lossy().into_owned(),
            line: Some(2),
        };
        let html = read(&source).unwrap();
        assert!(html.contains(
            "<div id=\"ngterm-text\" hidden data-language=\"go\" data-line=\"2\">\npackage &lt;main&gt;\n</div>"
        ));
        assert!(html.contains(TEXT_STYLE) && html.contains(STYLE));
        let binary = dir.path().join("blob.log");
        std::fs::write(&binary, b"a\0b").unwrap();
        let source = HtmlSource::Text {
            path: binary.to_string_lossy().into_owned(),
            line: None,
        };
        assert_eq!(
            read(&source).unwrap_err(),
            format!("not a UTF-8 text file: {}", binary.display())
        );
        assert_eq!(browser_refusal(&source), Some(errors::TEXT_IN_OVERLAY));
        assert_eq!(renderer(&source), Some(text_script()));
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
