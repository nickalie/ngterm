//! The Kitty graphics a frame shows ([`KittyView`]), in this crate's own types.
//!
//! The view is captured with the frame, under the VT lock (the library's image handles die on the terminal's next
//! mutation). Placements are converted only when the render state's placements generation changes; otherwise the
//! previous view's placements are reused. An image's pixels are copied only when its `(id, generation)` is new — an
//! unchanged image is shared with the previous view. The pane's IO thread prefetches new pixels after each batch
//! ([`Prefetch`]), so a paint that captures them (on the frontend's main thread) only shares them. An image a paint
//! catches in the middle of a batch, or an animation frame advanced by the capture's tick, is copied on the paint
//! thread under the VT lock.
//!
//! Placement geometry comes exclusively from Ghostty's render-state image API (`render.h`, `src/terminal/render.zig`
//! at `LIBGHOSTTY_VT_REV`, <https://github.com/ghostty-org/ghostty>, MIT, Copyright Mitchell Hashimoto and contributors).
//! The library resolves placeholders and relative chains; this module copies its geometry without recomputing it.

use std::collections::HashMap;
use std::sync::Arc;

use ngterm_ghostty_vt as vt;

mod animation;
pub(crate) use animation::Animation;

/// An image's pixel layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KittyFormat {
    Rgb,
    Rgba,
    Gray,
    GrayAlpha,
}

/// What identifies an image's pixels: a new transmission under the same id gets a new generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KittyImageKey {
    pub id: u32,
    pub generation: u64,
}

/// An image a placement shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImage {
    pub key: KittyImageKey,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    pub format: KittyFormat,
    pub pixels: Arc<[u8]>,
}

/// Where a placement is drawn relative to the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KittyLayer {
    BelowBackground,
    BelowText,
    AboveText,
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SourceRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where a placement lands on the viewport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PlacementRect {
    /// The top-left cell; negative when the placement scrolled partly off the top (or left).
    pub column: i32,
    pub row: i32,
    /// The cells it covers.
    pub columns: u32,
    pub rows: u32,
    /// The drawn size in pixels.
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// The part of the image drawn.
    pub source: SourceRect,
}

/// One resolved rectangle of an image, including each row of a placeholder placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KittyPlacement {
    pub image: KittyImageKey,
    /// The client's id; 0 for internal placements and unnamed placeholder runs (not a unique identity).
    pub placement_id: u32,
    pub layer: KittyLayer,
    pub z: i32,
    /// Resolved pixel offsets, including virtual centering beyond one cell.
    pub x_offset: u32,
    pub y_offset: u32,
    pub rect: PlacementRect,
}

/// The Kitty graphics of a frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KittyView {
    /// The storage's generation (0: never written).
    pub generation: u64,
    /// Equality token for the render state's resolved placement geometry.
    pub placements_generation: u64,
    /// Every stored placed image, once; retained while offscreen for the texture cache.
    pub images: Vec<Arc<KittyImage>>,
    /// Resolved rectangles in iterator order; draw by layer, z and image id.
    pub placements: Vec<KittyPlacement>,
    /// The images the previous frame's view had and this one has not: their textures can go. A view that skipped
    /// frames compares against what it drew instead.
    pub removed: Vec<KittyImageKey>,
}

impl KittyView {
    #[must_use]
    pub fn image(&self, key: KittyImageKey) -> Option<&Arc<KittyImage>> {
        self.images.iter().find(|image| image.key == key)
    }

    fn is_empty(&self) -> bool {
        self.images.is_empty() && self.placements.is_empty()
    }
}

/// The pixels of the placed images, copied on the pane's IO thread after each batch that changed the storage (kept
/// with the terminal, under the VT lock). A capture shares these when available; images caught mid-batch and animation
/// frames advanced by the capture's tick are copied on the paint thread. A later batch can prefetch an animation
/// frame already held by a captured view.
#[derive(Debug, Default)]
pub(crate) struct Prefetch {
    generation: u64,
    images: HashMap<KittyImageKey, Arc<KittyImage>>,
}

impl Prefetch {
    /// Copies the placed images it does not hold yet and forgets the ones no longer placed, when the storage changed.
    pub(crate) fn update(&mut self, graphics: Option<vt::KittyGraphics<'_>>) {
        let Some(graphics) = graphics else {
            *self = Self::default();
            return;
        };
        let generation = graphics.generation();
        if generation == self.generation {
            return;
        }
        let Ok(placements) = graphics.placements() else {
            return;
        };
        self.generation = generation;
        let mut kept = HashMap::new();
        for placement in placements {
            let Some(image) = graphics.image(placement.image_id) else {
                continue;
            };
            let key = KittyImageKey {
                id: image.id,
                generation: image.generation,
            };
            if kept.contains_key(&key) {
                continue;
            }
            // a pending payload has no pixels yet: the batch that completes it changes the storage again
            if let Some(copied) = self.images.remove(&key).or_else(|| copy(&image, key).map(Arc::new)) {
                kept.insert(key, copied);
            }
        }
        self.images = kept;
    }

    fn get(&self, key: KittyImageKey) -> Option<&Arc<KittyImage>> {
        self.images.get(&key)
    }
}

/// Capture pixels while the terminal is locked, from the same successful begin phase as the geometry.
/// Conversion is skipped on placement-generation equality; storage is checked separately (unplaced deletions too).
pub(crate) fn capture(
    graphics: Option<vt::KittyGraphics<'_>>,
    placements_generation: u64,
    mut placements: vt::RenderImages<'_>,
    previous: &KittyView,
    prefetched: &Prefetch,
) -> Result<Option<KittyView>, vt::Error> {
    let Some(graphics) = graphics else {
        return Ok((!previous.is_empty()).then(|| removed_all(previous)));
    };
    let generation = graphics.generation();
    if generation == previous.generation && placements_generation == previous.placements_generation {
        return Ok(None);
    }
    // Keep ngterm's texture policy: placed images remain cached while offscreen. The storage iterator is used only
    // to retain their pixels; every drawable rectangle still comes exclusively from the render-state iterator.
    let images = if generation == previous.generation {
        previous.images.clone()
    } else {
        stored_images(&graphics, previous, prefetched)?
    };
    let converted = if placements_generation == previous.placements_generation {
        previous.placements.clone()
    } else {
        let mut converted = Vec::new();
        while let Some(placement) = placements.next_image()? {
            let key = KittyImageKey {
                id: placement.image_id,
                generation: placement.image_generation,
            };
            if images.iter().any(|image| image.key == key) {
                converted.push(convert(placement, key));
            }
        }
        converted
    };
    let removed = previous
        .images
        .iter()
        .map(|image| image.key)
        .filter(|key| images.iter().all(|image| image.key != *key))
        .collect();
    Ok(Some(KittyView {
        generation,
        placements_generation,
        images,
        placements: converted,
        removed,
    }))
}

fn stored_images(
    graphics: &vt::KittyGraphics<'_>,
    previous: &KittyView,
    prefetched: &Prefetch,
) -> Result<Vec<Arc<KittyImage>>, vt::Error> {
    let mut images: Vec<Arc<KittyImage>> = Vec::new();
    for placement in graphics.placements()? {
        let Some(image) = graphics.image(placement.image_id) else {
            continue;
        };
        let key = KittyImageKey {
            id: image.id,
            generation: image.generation,
        };
        if images.iter().any(|image| image.key == key) {
            continue;
        }
        let copied = previous
            .image(key)
            .or_else(|| prefetched.get(key))
            .cloned()
            .or_else(|| copy(&image, key).map(Arc::new));
        if let Some(copied) = copied {
            images.push(copied);
        }
    }
    Ok(images)
}

fn removed_all(previous: &KittyView) -> KittyView {
    KittyView {
        removed: previous.images.iter().map(|image| image.key).collect(),
        ..KittyView::default()
    }
}

/// The image's pixels copied out of the terminal; `None` while they are pending.
fn copy(image: &vt::KittyImage<'_>, key: KittyImageKey) -> Option<KittyImage> {
    Some(KittyImage {
        key,
        number: image.number,
        width: image.width,
        height: image.height,
        format: match image.format {
            vt::KittyFormat::Rgb => KittyFormat::Rgb,
            vt::KittyFormat::Rgba => KittyFormat::Rgba,
            vt::KittyFormat::Gray => KittyFormat::Gray,
            vt::KittyFormat::GrayAlpha => KittyFormat::GrayAlpha,
        },
        pixels: Arc::from(image.pixels?),
    })
}

fn convert(placement: vt::RenderImage, image: KittyImageKey) -> KittyPlacement {
    KittyPlacement {
        image,
        placement_id: placement.placement_id,
        layer: match placement.layer {
            vt::KittyLayer::BelowBackground => KittyLayer::BelowBackground,
            vt::KittyLayer::BelowText => KittyLayer::BelowText,
            vt::KittyLayer::AboveText => KittyLayer::AboveText,
        },
        z: placement.z,
        x_offset: placement.offset_x,
        y_offset: placement.offset_y,
        rect: PlacementRect {
            column: placement.column,
            row: placement.row,
            columns: placement.columns,
            rows: placement.rows,
            pixel_width: placement.width,
            pixel_height: placement.height,
            source: SourceRect {
                x: placement.source.x,
                y: placement.source.y,
                width: placement.source.width,
                height: placement.source.height,
            },
        },
    }
}

#[cfg(test)]
mod tests;
