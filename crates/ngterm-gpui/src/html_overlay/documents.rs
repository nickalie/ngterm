//! Trusted document rendering, isolated from authored page JavaScript (upstream's `MarkdownAssets`).

use ngterm_core::html::{HtmlSource, image_page, markdown_page};

pub(super) const WORLD: &str = "ngterm-markdown";
pub(super) const STYLE: &str = include_str!("../../../../resources/markdown/page.css");
const MARKDOWN: &str = include_str!("../../../../resources/markdown/markdown-it-14.1.0.min.js");
const HIGHLIGHT: &str = include_str!("../../../../resources/markdown/highlight-11.11.1.min.js");
const RENDER: &str = include_str!("../../../../resources/markdown/render.js");

/// Runs only app-owned code, at document end, in the backend's named helper world.
pub(super) fn script() -> String {
    [MARKDOWN, HIGHLIGHT, RENDER].join(";\n")
}

/// Native engines install at document start; the app's isolated script waits for the inert data elements.
#[cfg(any(target_os = "macos", windows))]
pub(super) fn document_start_script() -> String {
    format!(
        "(() => {{ const render = () => {{\n{}\n}}; if(document.readyState === 'loading') document.addEventListener('DOMContentLoaded',render,{{once:true}}); else render(); }})();",
        script()
    )
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
