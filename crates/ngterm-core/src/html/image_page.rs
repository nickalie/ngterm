//! Raster image documents and sizing (upstream `ImagePage.swift`). File reads and header probes live in
//! `ngterm-app`; this module only builds inert documents and computes display points.

use base64::Engine as _;

use crate::hud::HudPaneFrame;
use crate::model::OverlayPointSize;

pub const BASE_PERCENT: i64 = 95;
pub const MINIMUM_POINTS: f64 = 240.0;
pub const MAX_BYTES: usize = 64 << 20;
pub const MAX_PIXELS: u64 = 100_000_000;

/// The exact bytes that were sniffed and probed, with the first frame's header metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
}

impl ImageData {
    #[must_use]
    pub fn html(&self) -> String {
        html(&self.bytes, &self.mime_type)
    }

    #[must_use]
    pub fn natural(&self, scale: f64) -> (f64, f64) {
        natural(self.width, self.height, self.orientation, scale)
    }

    #[must_use]
    pub fn data_uri(&self) -> String {
        data_uri(&self.bytes, &self.mime_type)
    }
}

/// The raster formats an untrusted file is allowed to hand to the image header reader.
#[must_use]
pub fn mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[must_use]
pub fn allows_pixels(width: u64, height: u64) -> bool {
    width.checked_mul(height).is_some_and(|pixels| pixels <= MAX_PIXELS)
}

/// One image pixel per display pixel, with EXIF quarter turns applied before converting to points.
#[must_use]
pub fn natural(width: u32, height: u32, orientation: u8, scale: f64) -> (f64, f64) {
    let scale = if scale > 0.0 && scale.is_finite() { scale } else { 1.0 };
    let (width, height) = if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    };
    (f64::from(width) / scale, f64::from(height) / scale)
}

/// Content points at the page zoom, preserving aspect while fitting the detail area's point cap and strip.
#[must_use]
pub fn points(natural: (f64, f64), zoom: f64, bounds: Option<HudPaneFrame>) -> OverlayPointSize {
    let (mut width, mut height) = (natural.0 * zoom, natural.1 * zoom);
    if width <= 0.0 || height <= 0.0 || !width.is_finite() || !height.is_finite() {
        return OverlayPointSize {
            width: Some(240),
            height: Some(240),
        };
    }
    if width < MINIMUM_POINTS {
        height *= MINIMUM_POINTS / width;
        width = MINIMUM_POINTS;
    }
    if let Some(bounds) = bounds {
        let room = (
            bounds.width * crate::model::MAX_SHARE,
            bounds.height * crate::model::MAX_SHARE - super::HtmlOverlay::CHROME_HEIGHT,
        );
        let fit = 1.0_f64.min(room.0 / width).min(room.1 / height);
        if fit > 0.0 {
            width *= fit;
            height *= fit;
        }
    }
    OverlayPointSize {
        width: Some(rounded_points(width)),
        height: Some(rounded_points(height)),
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "finite positive display points saturate to the wire integer"
)]
fn rounded_points(value: f64) -> i64 {
    (value.round() as i64).max(1)
}

#[must_use]
pub fn data_uri(bytes: &[u8], mime_type: &str) -> String {
    format!(
        "data:{mime_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

/// A transparent, self-contained raster page. Neither authored script nor a file grant is needed.
#[must_use]
pub fn html(bytes: &[u8], mime_type: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" \
         content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'\"><style>\
         html,body{{margin:0;height:100%;background:transparent;overflow:hidden}}\
         img{{display:block;width:100%;height:100%;object-fit:contain}}\
         </style></head><body><img alt=\"\" src=\"{}\"></body></html>",
        data_uri(bytes, mime_type)
    )
}

#[cfg(test)]
#[path = "image_page_tests.rs"]
mod tests;
