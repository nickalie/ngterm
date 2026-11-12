//! Owned geometry read from Ghostty's `render.h` image iterator at `LIBGHOSTTY_VT_REV`.
//! Ghostty resolves fixed, placeholder and relative placements; this is the sole drawing authority.

use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::{Error, Handle, KittyLayer, PixelRect, RenderState, ffi};

/// One resolved Kitty image rectangle. Geometry is already fitted; offsets must not be clamped again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct RenderImage {
    pub image_id: u32,
    pub image_generation: u64,
    pub placement_id: u32,
    pub layer: KittyLayer,
    pub z: i32,
    pub column: i32,
    pub row: i32,
    pub columns: u32,
    pub rows: u32,
    pub offset_x: u32,
    pub offset_y: u32,
    pub width: u32,
    pub height: u32,
    pub source: PixelRect,
}

pub(crate) fn new_iterator() -> Result<Handle<ffi::GhosttyRenderStateImageIteratorImpl>, Error> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: null selects the default allocator; raw is a valid output handle.
    Error::check(unsafe { ffi::ghostty_render_state_image_iterator_new(std::ptr::null(), &raw mut raw) })?;
    Handle::new(raw, ffi::ghostty_render_state_image_iterator_free)
}

pub(crate) fn images(
    state: ffi::GhosttyRenderState,
    iterator: &mut Handle<ffi::GhosttyRenderStateImageIteratorImpl>,
) -> Result<RenderImages<'_>, Error> {
    let mut raw = iterator.ptr();
    // SAFETY: the successful capture and iterator are live, exclusively borrowed for the returned walk.
    Error::check(unsafe {
        ffi::ghostty_render_state_get(
            state,
            ffi::GHOSTTY_RENDER_STATE_DATA_IMAGE_ITERATOR,
            (&raw mut raw).cast(),
        )
    })?;
    Ok(RenderImages {
        raw: iterator.non_null(),
        borrow: PhantomData,
    })
}

/// A walk borrowing its capture, so it cannot survive an update or let a C iterator escape.
#[derive(Debug)]
pub struct RenderImages<'a> {
    raw: NonNull<ffi::GhosttyRenderStateImageIteratorImpl>,
    borrow: PhantomData<&'a mut RenderState>,
}

impl RenderImages<'_> {
    /// Next supported image; unknown protocols are skipped.
    ///
    /// # Errors
    ///
    /// The library's error if any required geometry value cannot be read.
    pub fn next_image(&mut self) -> Result<Option<RenderImage>, Error> {
        // SAFETY: the iterator borrows its successful capture and is exclusively accessed here.
        while unsafe { ffi::ghostty_render_state_image_iterator_next(self.raw.as_ptr()) } {
            let mut protocol = 0i32;
            // SAFETY: the protocol output is a C enum (int); the iterator is on a live placement.
            Error::check(unsafe {
                ffi::ghostty_render_state_image_get(
                    self.raw.as_ptr(),
                    ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_PROTOCOL,
                    (&raw mut protocol).cast(),
                )
            })?;
            // GhosttyImageProtocol: Kitty is 0 (render.h); future protocols need their own pixel lookup.
            if protocol == ffi::GHOSTTY_IMAGE_PROTOCOL_KITTY {
                return self.current().map(Some);
            }
        }
        Ok(None)
    }

    fn current(&self) -> Result<RenderImage, Error> {
        let mut image = RenderImage::default();
        let mut layer = ffi::GHOSTTY_RENDER_STATE_IMAGE_LAYER_ABOVE_TEXT;
        let keys = [
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_ID,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_IMAGE_GENERATION,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_KITTY_PLACEMENT_ID,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_LAYER,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_Z,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_COL,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_VIEWPORT_ROW,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_GRID_COLS,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_GRID_ROWS,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_X,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_CELL_OFFSET_Y,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_WIDTH,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_PIXEL_HEIGHT,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_X,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_Y,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_WIDTH,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_DATA_SOURCE_HEIGHT,
        ];
        let mut values = [
            (&raw mut image.image_id).cast(),
            (&raw mut image.image_generation).cast(),
            (&raw mut image.placement_id).cast(),
            (&raw mut layer).cast(),
            (&raw mut image.z).cast(),
            (&raw mut image.column).cast(),
            (&raw mut image.row).cast(),
            (&raw mut image.columns).cast(),
            (&raw mut image.rows).cast(),
            (&raw mut image.offset_x).cast(),
            (&raw mut image.offset_y).cast(),
            (&raw mut image.width).cast(),
            (&raw mut image.height).cast(),
            (&raw mut image.source.x).cast(),
            (&raw mut image.source.y).cast(),
            (&raw mut image.source.width).cast(),
            (&raw mut image.source.height).cast(),
        ];
        // SAFETY: each key's output has the exact C integer type documented in render.h. All pointers are distinct
        // live fields, and no value or pointer is retained by C after this call.
        Error::check(unsafe {
            ffi::ghostty_render_state_image_get_multi(
                self.raw.as_ptr(),
                keys.len(),
                keys.as_ptr(),
                values.as_mut_ptr(),
                std::ptr::null_mut(),
            )
        })?;
        image.layer = match layer {
            ffi::GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_BACKGROUND => KittyLayer::BelowBackground,
            ffi::GHOSTTY_RENDER_STATE_IMAGE_LAYER_BELOW_TEXT => KittyLayer::BelowText,
            _ => KittyLayer::AboveText,
        };
        Ok(image)
    }
}

#[cfg(test)]
mod tests;
