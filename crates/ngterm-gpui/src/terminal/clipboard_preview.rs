//! The Kitty clipboard confirmation's preview of what a read would disclose or a write would put on the clipboard
//! (Ghostty 83fb956: the GTK `ClipboardConfirmationDialog.setParts`, the macOS `confirmReadClipboard`).
//!
//! - Linux and Windows (Ghostty's GTK form): one page per representation, an empty one included (an empty part is
//!   how a write clears the clipboard), picked with a pager shown only past one page. A part whose MIME type names an
//!   image and that decodes is shown as the image, at most [`PREVIEW_HEIGHT`] logical pixels high; else valid UTF-8 as
//!   monospace text; else a "No preview available" note with its size.
//! - macOS (Ghostty's macOS form): one page holding the `text/plain` text, or a summary line per representation, with
//!   the first decodable image beneath.
//! - No representation at all: a note per direction (a write clears the clipboard; a read that named only types the
//!   clipboard lacks discloses just its type listing).
//!
//! Images are untrusted (the crate's rule, `kitty/textures.rs`: an image's size is never paid on the main thread):
//! only the page on screen is decoded, on the background executor, one decode at a time, under bounded dimensions
//! ([`MAX_TEXTURE_SIDE`]) and allocation, and kept only scaled down to the preview's size; changing pages drops it.
//! A prompt waiting in the window's queue decodes nothing until it is shown. Nothing a program names (its `name=`
//! key) is shown: only the bytes and MIME types it supplies, as data.

use std::sync::Arc;

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render, RenderImage,
    SharedString, StatefulInteractiveElement as _, Styled as _, Task, Window, div, img, px,
};
use ngterm_core::clipboard_policy::ClipboardAccess;
use ngterm_term::ClipboardContent;

use super::kitty::textures::{MAX_TEXTURE_SIDE, resample};

/// The preview's height, as the GTK dialog's `height-request` (and the image clamp).
pub const PREVIEW_HEIGHT: f32 = 200.;
/// The widest an image is drawn: the alert's content width.
const PREVIEW_WIDTH: f32 = 412.;
/// A decoded image is kept at most this many pixels per logical pixel of the preview (sharp on a 2x display).
const PREVIEW_DENSITY: f32 = 2.;
/// The most one decode of an untrusted image allocates.
const MAX_ALLOC: u64 = 64 * 1024 * 1024;
/// The most text a page lays out: the UI thread shapes every glyph of it, and a clipboard (or a write) can hold tens
/// of megabytes. The caption still gives the whole part's size.
pub const MAX_PREVIEW_TEXT: usize = 64 * 1024;

/// What one page shows.
#[derive(Clone)]
pub enum Body {
    /// A decoded image (scaled down to the preview) and its own pixel size.
    Image {
        image: Arc<RenderImage>,
        width: u32,
        height: u32,
    },
    Text(String),
    /// Data that is neither: its size in bytes.
    NoPreview(usize),
    /// An image part of this size in bytes, decoded while its page is shown.
    Decoding(usize),
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image { width, height, .. } => write!(f, "Image({width}x{height})"),
            Self::Text(text) => f.debug_tuple("Text").field(text).finish(),
            Self::NoPreview(size) => f.debug_tuple("NoPreview").field(size).finish(),
            Self::Decoding(size) => f.debug_tuple("Decoding").field(size).finish(),
        }
    }
}

/// The image parts a page decodes when shown.
#[derive(Clone)]
struct Pending {
    /// The candidates, in order: the first that decodes is shown.
    parts: Vec<Arc<[u8]>>,
    /// macOS: the image goes under the page's text (none when no part decodes); else it is the page's body (the
    /// part's text or a note when it does not decode).
    under_text: bool,
}

impl Pending {
    /// What the decode lands (run off the main thread).
    fn decode(&self) -> Option<Body> {
        let image = self.parts.iter().find_map(|part| decode_image(part));
        if self.under_text {
            return image;
        }
        image.or_else(|| self.parts.first().map(|part| plain_body(part)))
    }
}

/// One page: its title (the MIME type, or a summary title on macOS) and what it shows, an optional image under it.
#[derive(Clone)]
pub struct Page {
    pub title: String,
    pub body: Body,
    /// The part's size in bytes.
    pub size: usize,
    /// macOS: the first decodable image, shown under the text.
    pub image: Option<Body>,
    pending: Option<Pending>,
}

impl std::fmt::Debug for Page {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Page")
            .field("title", &self.title)
            .field("body", &self.body)
            .field("size", &self.size)
            .field("image", &self.image)
            .finish_non_exhaustive()
    }
}

impl Page {
    /// Whether the page waits for its image decode.
    #[must_use]
    pub fn is_decoding(&self) -> bool {
        matches!(self.body, Body::Decoding(_)) || matches!(self.image, Some(Body::Decoding(_)))
    }

    /// The page with its image decoded here, on this thread (tests and the test helpers that read a preview).
    #[must_use]
    pub fn decoded(mut self) -> Self {
        if self.is_decoding()
            && let Some(pending) = &self.pending
        {
            let decoded = pending.decode();
            self.land(decoded);
        }
        self
    }

    fn land(&mut self, decoded: Option<Body>) {
        match &self.pending {
            Some(pending) if pending.under_text => self.image = decoded,
            Some(_) => self.body = decoded.unwrap_or(Body::NoPreview(self.size)),
            None => {}
        }
    }

    /// Drops the decoded image (the page is no longer shown); it is decoded again when shown.
    fn unload(&mut self) {
        match &self.pending {
            Some(pending) if pending.under_text => {
                let size = pending.parts.first().map_or(0, |part| part.len());
                self.image = Some(Body::Decoding(size));
            }
            Some(_) => self.body = Body::Decoding(self.size),
            None => {}
        }
    }
}

/// The size an image `width`×`height` is kept at: fitted inside the preview at [`PREVIEW_DENSITY`], never enlarged.
#[must_use]
pub fn kept_size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = fitted(width, height, PREVIEW_WIDTH);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive side no larger than the original u32"
    )]
    let side = |fit: f32, own: u32| ((fit * PREVIEW_DENSITY).round() as u32).clamp(1, own.max(1));
    (side(w, width), side(h, height))
}

/// `data` as an image when it decodes, with its own pixel size, kept at [`kept_size`] (run off the main thread).
#[must_use]
pub fn decode_image(data: &[u8]) -> Option<Body> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_SIDE);
    limits.max_image_height = Some(MAX_TEXTURE_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let rgba = reader.decode().ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let kept = kept_size(width, height);
    // GPUI draws BGRA
    let pixels = resample(rgba.as_raw(), 4, (width, height), kept, |pixel| {
        [pixel[2], pixel[1], pixel[0], pixel[3]]
    })?;
    let buffer = image::RgbaImage::from_raw(kept.0, kept.1, pixels)?;
    Some(Body::Image {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        width,
        height,
    })
}

/// What a part that is not shown as an image shows: UTF-8 text (its first [`MAX_PREVIEW_TEXT`] bytes), else a note
/// with its size.
fn plain_body(data: &[u8]) -> Body {
    match std::str::from_utf8(data) {
        Ok(text) => Body::Text(bounded_text(text)),
        Err(_) => Body::NoPreview(data.len()),
    }
}

/// `text` as a preview shows it: at most [`MAX_PREVIEW_TEXT`] bytes, cut at a character boundary, and a line saying
/// how much is left out. Only the preview is cut: what the program reads or writes is the whole text.
#[must_use]
pub fn bounded_text(text: &str) -> String {
    if text.len() <= MAX_PREVIEW_TEXT {
        return text.to_owned();
    }
    let end = text.floor_char_boundary(MAX_PREVIEW_TEXT);
    format!("{}\n… {} more", &text[..end], format_size(text.len() - end))
}

/// What a part shows (GTK's `previewPart`): an image MIME type that decodes as the image, else UTF-8 text, else a
/// note with its size. An empty part is the empty text. Decodes on this thread: the view decodes through
/// [`Preview`], off the main thread.
#[must_use]
pub fn body(mime: &str, data: &[u8]) -> Body {
    if mime.starts_with("image/")
        && let Some(image) = decode_image(data)
    {
        return image;
    }
    plain_body(data)
}

/// The Linux and Windows form: a page per part, in order, an image part still to decode.
#[must_use]
pub fn pages(contents: &[ClipboardContent]) -> Vec<Page> {
    contents
        .iter()
        .map(|content| {
            let size = content.data.len();
            let (body, pending) = if content.mime.starts_with("image/") {
                let pending = Pending {
                    parts: vec![Arc::from(content.data.as_slice())],
                    under_text: false,
                };
                (Body::Decoding(size), Some(pending))
            } else {
                (plain_body(&content.data), None)
            };
            Page {
                title: content.mime.clone(),
                body,
                size,
                image: None,
                pending,
            }
        })
        .collect()
}

/// The macOS form: one page with the `text/plain` text when it is UTF-8 (bounded as [`bounded_text`] says), else a "type (N bytes)" line per part, and
/// the first image part that decodes, still to decode.
#[must_use]
pub fn summary(contents: &[ClipboardContent]) -> Page {
    let text = contents
        .iter()
        .find(|content| content.mime == "text/plain")
        .and_then(|content| std::str::from_utf8(&content.data).ok())
        .map_or_else(
            || {
                contents
                    .iter()
                    .map(|content| format!("{} ({} bytes)", content.mime, content.data.len()))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            bounded_text,
        );
    let parts: Vec<Arc<[u8]>> = contents
        .iter()
        .filter(|content| content.mime.starts_with("image/"))
        .map(|content| Arc::from(content.data.as_slice()))
        .collect();
    let image = parts.first().map(|part| Body::Decoding(part.len()));
    Page {
        title: String::new(),
        body: Body::Text(text),
        size: contents.iter().map(|content| content.data.len()).sum(),
        image,
        pending: (!parts.is_empty()).then_some(Pending {
            parts,
            under_text: true,
        }),
    }
}

/// The pages this OS shows for `contents`: none when there is no representation.
#[must_use]
pub fn pages_for_os(contents: &[ClipboardContent]) -> Vec<Page> {
    if contents.is_empty() {
        Vec::new()
    } else if cfg!(target_os = "macos") {
        vec![summary(contents)]
    } else {
        pages(contents)
    }
}

/// What the preview says when there is no representation to show.
#[must_use]
pub fn empty_note(access: ClipboardAccess) -> &'static str {
    match access {
        ClipboardAccess::Read => {
            "None of the requested types: the program will see only which types the clipboard holds"
        }
        ClipboardAccess::Write => "Nothing: the clipboard will be cleared",
    }
}

/// An image `width`×`height` fitted inside `max_width`×[`PREVIEW_HEIGHT`] logical pixels, never enlarged (GTK's
/// `content-fit: contain` in an `AdwClamp` of the preview height).
#[must_use]
pub fn fitted(width: u32, height: u32, max_width: f32) -> (f32, f32) {
    #[expect(clippy::cast_precision_loss, reason = "a display approximation")]
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let scale = (max_width / width).min(PREVIEW_HEIGHT / height).min(1.0);
    (width * scale, height * scale)
}

/// A size as glib's `g_format_size` writes it: bytes below a kilobyte, else SI units with one decimal.
#[must_use]
pub fn format_size(size: usize) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if size < 1000 {
        return if size == 1 {
            "1 byte".into()
        } else {
            format!("{size} bytes")
        };
    }
    #[expect(clippy::cast_precision_loss, reason = "a display approximation")]
    let mut value = size as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The preview view inside the alert.
pub struct Preview {
    pages: Vec<Page>,
    selected: usize,
    /// What it says with no page.
    empty: &'static str,
    /// The page whose image decodes now, and the task landing it (dropped, and so cancelled, on a page change).
    decoding: Option<(usize, Task<()>)>,
}

impl Preview {
    #[must_use]
    pub fn new(pages: Vec<Page>, empty: &'static str) -> Self {
        Self {
            pages,
            selected: 0,
            empty,
            decoding: None,
        }
    }

    /// The pages (tests).
    #[must_use]
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// The page shown.
    #[must_use]
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// What it says with no page.
    #[must_use]
    pub fn empty(&self) -> &'static str {
        self.empty
    }

    /// Shows page `index` when there is one: the page left drops its decoded image, the new one starts decoding.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.pages.len() || index == self.selected {
            return;
        }
        self.decoding = None;
        if let Some(left) = self.pages.get_mut(self.selected) {
            left.unload();
        }
        self.selected = index;
        self.load_selected(cx);
        cx.notify();
    }

    /// Starts decoding the shown page's image on the background executor, unless it is decoded or decoding.
    pub fn load_selected(&mut self, cx: &mut Context<Self>) {
        let index = self.selected;
        let Some(page) = self.pages.get(index) else {
            return;
        };
        if !page.is_decoding() || self.decoding.as_ref().is_some_and(|(at, _)| *at == index) {
            return;
        }
        let Some(pending) = page.pending.clone() else {
            return;
        };
        let decoding = cx.background_executor().spawn(async move { pending.decode() });
        let task = cx.spawn(async move |preview, cx| {
            let decoded = decoding.await;
            let landed = preview.update(cx, |preview, cx| {
                if preview.selected != index {
                    return;
                }
                preview.decoding = None;
                if let Some(page) = preview.pages.get_mut(index) {
                    page.land(decoded);
                }
                cx.notify();
            });
            if landed.is_err() {
                tracing::debug!(target: "ngterm::surface", "the clipboard preview is gone before its image decoded");
            }
        });
        self.decoding = Some((index, task));
    }

    fn render_body(body: &Body, colors: ngterm_core::chrome::ChromeColors) -> gpui_kit::AnyElement {
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        match body {
            Body::Image { image, width, height } => {
                let (w, h) = fitted(*width, *height, PREVIEW_WIDTH);
                div()
                    .flex()
                    .justify_center()
                    .child(
                        img(Arc::clone(image))
                            .debug_selector(|| "clipboard-preview-image".to_owned())
                            .w(px(w))
                            .h(px(h)),
                    )
                    .into_any_element()
            }
            Body::Text(text) if text.is_empty() => div()
                .debug_selector(|| "clipboard-preview-empty-part".to_owned())
                .text_color(dim)
                .text_size(px(12.))
                .child("Empty")
                .into_any_element(),
            Body::Text(text) => div()
                .debug_selector(|| "clipboard-preview-text".to_owned())
                .font_family("monospace")
                .text_size(px(12.))
                .whitespace_normal()
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            Body::Decoding(size) => div()
                .debug_selector(|| "clipboard-preview-loading".to_owned())
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .text_color(dim)
                .child("Loading preview…")
                .child(format_size(*size))
                .into_any_element(),
            Body::NoPreview(size) => div()
                .debug_selector(|| "clipboard-preview-no-preview".to_owned())
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_1()
                .child("No preview available")
                .child(div().text_color(dim).child(format_size(*size)))
                .into_any_element(),
        }
    }

    fn render_pager(&self, colors: ngterm_core::chrome::ChromeColors, cx: &mut Context<Self>) -> impl IntoElement {
        let ink = crate::chrome::color(colors.foreground, 1.0);
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let count = self.pages.len();
        let selected = self.selected;
        let step = |id: &'static str, glyph: &'static str, target: Option<usize>, cx: &mut Context<Self>| {
            div()
                .id(id)
                .debug_selector(move || id.to_owned())
                .role(gpui_kit::Role::Button)
                .aria_label(SharedString::from(if id.ends_with("previous") {
                    "Previous part"
                } else {
                    "Next part"
                }))
                .w(px(24.))
                .h(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .text_color(if target.is_some() { ink } else { dim })
                .when(target.is_some(), |step| {
                    step.cursor_pointer()
                        .hover(|style| style.bg(crate::chrome::color(colors.foreground, 0.1)))
                })
                .child(glyph)
                .when_some(target, |step, target| {
                    step.on_click(cx.listener(move |preview, _, _, cx| preview.select(target, cx)))
                })
        };
        let title = self
            .pages
            .get(selected)
            .map(|page| page.title.clone())
            .unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(step("clipboard-preview-previous", "‹", selected.checked_sub(1), cx))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .child(SharedString::from(title)),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(dim)
                    .child(format!("{} of {count}", selected + 1)),
            )
            .child(step(
                "clipboard-preview-next",
                "›",
                (selected + 1 < count).then_some(selected + 1),
                cx,
            ))
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.load_selected(cx);
        let colors = crate::chrome::current(cx);
        let dim = crate::chrome::color(colors.foreground, crate::chrome::DIM_ALPHA);
        let well = div()
            .id("clipboard-preview")
            .debug_selector(|| "clipboard-preview".to_owned())
            .h(px(PREVIEW_HEIGHT))
            .overflow_y_scroll()
            .p_2()
            .rounded(px(8.))
            .bg(crate::chrome::color(colors.foreground, 0.06))
            .border_1()
            .border_color(crate::chrome::color(colors.foreground, 0.12))
            .text_size(px(13.));
        let Some(page) = self.pages.get(self.selected) else {
            return div().flex().flex_col().child(
                well.child(
                    div()
                        .debug_selector(|| "clipboard-preview-note".to_owned())
                        .text_color(dim)
                        .child(self.empty),
                ),
            );
        };
        let body = Self::render_body(&page.body, colors);
        let image = page.image.as_ref().map(|image| Self::render_body(image, colors));
        let caption = (!page.title.is_empty() || self.pages.len() == 1).then(|| format_size(page.size));
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .when(self.pages.len() > 1, |content| {
                content.child(self.render_pager(colors, cx))
            })
            .child(well.child(div().flex().flex_col().gap_2().child(body).children(image)))
            .when_some(caption, |content, caption| {
                let mime = if self.pages.len() == 1 {
                    page.title.clone()
                } else {
                    String::new()
                };
                content.child(
                    div()
                        .debug_selector(|| "clipboard-preview-caption".to_owned())
                        .flex()
                        .justify_between()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(dim)
                        .child(div().min_w(px(0.)).overflow_hidden().text_ellipsis().child(mime))
                        .child(div().flex_shrink_0().child(caption)),
                )
            })
    }
}

/// A preview view for `contents` as this OS shows it, for a read or a write (`access`). Nothing is decoded until it
/// is drawn.
pub fn view(contents: &[ClipboardContent], access: ClipboardAccess, cx: &mut App) -> gpui_kit::Entity<Preview> {
    let pages = pages_for_os(contents);
    gpui_kit::AppContext::new(cx, |_| Preview::new(pages, empty_note(access)))
}

#[cfg(test)]
#[path = "clipboard_preview_tests.rs"]
mod tests;
