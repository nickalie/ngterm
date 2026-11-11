//! Kitty graphics (`kitty_graphics.h`): the images and placements stored on the terminal's active screen.
//!
//! Every handle the library lends here dies on the terminal's next mutating call, so [`KittyGraphics`] borrows the
//! terminal mutably for its whole life: nothing can write the terminal while placements are read or an image's pixels
//! are borrowed.
//!
use std::marker::PhantomData;

use crate::{Error, Handle, HeldTerminal, Terminal, ffi};

/// A stored image's pixel layout (always decoded and decompressed: never PNG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KittyFormat {
    Rgb,
    Rgba,
    Gray,
    GrayAlpha,
}

impl KittyFormat {
    fn from_raw(raw: ffi::GhosttyKittyImageFormat) -> Option<Self> {
        match raw {
            ffi::GHOSTTY_KITTY_IMAGE_FORMAT_RGB => Some(Self::Rgb),
            ffi::GHOSTTY_KITTY_IMAGE_FORMAT_RGBA => Some(Self::Rgba),
            ffi::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY => Some(Self::Gray),
            ffi::GHOSTTY_KITTY_IMAGE_FORMAT_GRAY_ALPHA => Some(Self::GrayAlpha),
            _ => None,
        }
    }
}

/// Where a placement is drawn relative to the text, from its z-index (the protocol's conventions).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum KittyLayer {
    /// `z < i32::MIN / 2`: under the cell backgrounds.
    BelowBackground,
    /// `i32::MIN / 2 <= z < 0`: over the backgrounds, under the text.
    BelowText,
    /// `z >= 0`.
    #[default]
    AboveText,
}

impl KittyLayer {
    #[must_use]
    pub fn of(z: i32) -> Self {
        if z < i32::MIN / 2 {
            Self::BelowBackground
        } else if z < 0 {
            Self::BelowText
        } else {
            Self::AboveText
        }
    }
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// What the library resolves of a placement's drawing: its size and source always, its viewport cell when visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct KittyRenderInfo {
    /// The drawn size in pixels.
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// The cells it covers.
    pub grid_cols: u32,
    pub grid_rows: u32,
    /// The top-left cell on the viewport; negative when it scrolled partly off the top or left. Meaningful only when
    /// `visible`.
    pub viewport_col: i32,
    pub viewport_row: i32,
    /// Whether it lands on the viewport: false off it, for a virtual placement and for a relative placement whose
    /// chain roots at a virtual one (which has no position of its own) or does not resolve.
    pub visible: bool,
    /// The part of the image drawn, clamped to its bounds.
    pub source: PixelRect,
}

/// One placement of an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KittyPlacement {
    pub image_id: u32,
    pub placement_id: u32,
    /// A virtual placement (Unicode placeholders) has no position of its own.
    pub is_virtual: bool,
    pub z: i32,
    /// The pixel offset inside the top-left cell.
    pub x_offset: u32,
    pub y_offset: u32,
    /// The cells asked for (`c=`, `r=`; 0: from the image's size); a virtual placement's grid.
    pub columns: u32,
    pub rows: u32,
    /// `None` when the library resolves nothing (its image is missing).
    pub render: Option<KittyRenderInfo>,
}

/// An image, borrowed from the terminal.
#[derive(Debug, Clone, Copy)]
pub struct KittyImage<'a> {
    pub id: u32,
    pub number: u32,
    pub width: u32,
    pub height: u32,
    pub format: KittyFormat,
    /// Changes whenever the pixels may have (a retransmission, an animation frame); unique process-wide, never zero.
    pub generation: u64,
    /// The pixels in `format`; `None` while the payload is still pending.
    pub pixels: Option<&'a [u8]>,
}

/// The image storage of the terminal's active screen, borrowed with the terminal.
#[derive(Debug)]
pub struct KittyGraphics<'t> {
    raw: ffi::GhosttyKittyGraphics,
    terminal: ffi::GhosttyTerminal,
    borrow: PhantomData<&'t mut Terminal>,
}

impl Terminal {
    /// The Kitty image storage; `None` when the library was built without Kitty graphics.
    pub fn kitty_graphics(&mut self) -> Option<KittyGraphics<'_>> {
        // SAFETY: the terminal is live and the result borrows `self` mutably, so no mutating call runs while it lives.
        unsafe { KittyGraphics::of(self.as_raw()) }
    }
}

impl HeldTerminal<'_> {
    /// The Kitty image storage of the terminal running the synchronized-output callback, read as the frame it holds is
    /// captured; `None` when the library was built without Kitty graphics.
    pub fn kitty_graphics(&mut self) -> Option<KittyGraphics<'_>> {
        // SAFETY: the callback's terminal is live for the call, its caller holds what serializes it, and the result
        // borrows `self` (so the callback) mutably: the terminal is not mutated while it lives.
        unsafe { KittyGraphics::of(self.as_raw()) }
    }
}

impl KittyGraphics<'_> {
    /// The storage of `terminal`.
    ///
    /// # Safety
    ///
    /// `terminal` must be live and not mutated while the result lives.
    unsafe fn of(terminal: ffi::GhosttyTerminal) -> Option<Self> {
        let mut raw: ffi::GhosttyKittyGraphics = std::ptr::null_mut();
        // SAFETY: the terminal is live (the caller's contract); `DATA_KITTY_GRAPHICS` writes a handle borrowed until
        // the next mutating call, which the caller rules out while the result lives.
        let result = unsafe {
            ffi::ghostty_terminal_get(
                terminal,
                ffi::GHOSTTY_TERMINAL_DATA_KITTY_GRAPHICS,
                (&raw mut raw).cast(),
            )
        };
        (result == ffi::GHOSTTY_SUCCESS && !raw.is_null()).then_some(Self {
            raw,
            terminal,
            borrow: PhantomData,
        })
    }
}

impl KittyGraphics<'_> {
    /// Advance each running animation by at most one frame and return its next absolute deadline on the caller's
    /// forward-only millisecond clock. `None` means no animation needs another tick.
    ///
    /// The mutable borrow prevents a tick while image pixels borrowed from this storage are still in use.
    /// See Ghostty `src/terminal/c/kitty_graphics.zig::animation_tick` at the libghostty-vt pin.
    ///
    /// # Errors
    ///
    /// The library's result when it cannot tick the storage.
    pub fn animation_tick(&mut self, now_ms: u64) -> Result<Option<u64>, Error> {
        let mut next_ms = 0;
        // SAFETY: the storage is live and exclusively borrowed; `next_ms` is a valid uint64_t out-pointer.
        let result = unsafe { ffi::ghostty_kitty_graphics_animation_tick(self.raw, now_ms, &raw mut next_ms) };
        if result == ffi::GHOSTTY_NO_VALUE {
            return Ok(None);
        }
        Error::check(result)?;
        Ok(Some(next_ms))
    }

    /// The storage's generation: unchanged means the same placements and image data (geometry may still have moved
    /// with scrolling); zero for a storage never written.
    #[must_use]
    pub fn generation(&self) -> u64 {
        let mut generation: u64 = 0;
        // SAFETY: the storage is live (the terminal is borrowed); `DATA_GENERATION` writes a `uint64_t`.
        unsafe {
            ffi::ghostty_kitty_graphics_get(
                self.raw,
                ffi::GHOSTTY_KITTY_GRAPHICS_DATA_GENERATION,
                (&raw mut generation).cast(),
            )
        };
        generation
    }

    /// Every placement, with where it lands on the viewport.
    ///
    /// # Errors
    ///
    /// The library's result when the placements cannot be read.
    pub fn placements(&self) -> Result<Vec<KittyPlacement>, Error> {
        let mut iterator: ffi::GhosttyKittyGraphicsPlacementIterator = std::ptr::null_mut();
        // SAFETY: a null allocator selects the default one; `iterator` is a valid out-pointer.
        Error::check(unsafe {
            ffi::ghostty_kitty_graphics_placement_iterator_new(std::ptr::null(), &raw mut iterator)
        })?;
        let iterator = Handle::new(iterator, ffi::ghostty_kitty_graphics_placement_iterator_free)?;
        let mut filled = iterator.ptr();
        // SAFETY: the storage is live; `DATA_PLACEMENT_ITERATOR` fills the pre-allocated iterator through a pointer to
        // its handle.
        Error::check(unsafe {
            ffi::ghostty_kitty_graphics_get(
                self.raw,
                ffi::GHOSTTY_KITTY_GRAPHICS_DATA_PLACEMENT_ITERATOR,
                (&raw mut filled).cast(),
            )
        })?;
        let mut placements = Vec::new();
        // SAFETY: the iterator is live and filled from storage that cannot change while `self` borrows the terminal.
        while unsafe { ffi::ghostty_kitty_graphics_placement_next(iterator.ptr()) } {
            placements.push(self.placement(iterator.ptr()));
        }
        Ok(placements)
    }

    /// The image `id`; `None` when there is none.
    #[must_use]
    pub fn image(&self, id: u32) -> Option<KittyImage<'_>> {
        // SAFETY: the storage is live; the returned handle is borrowed until the next mutating terminal call, which
        // `self`'s borrow rules out for the image's lifetime.
        let image = unsafe { ffi::ghostty_kitty_graphics_image(self.raw, id) };
        if image.is_null() {
            return None;
        }
        let u32_of = |data| image_value::<u32>(image, data).unwrap_or(0);
        let format = image_value::<ffi::GhosttyKittyImageFormat>(image, ffi::GHOSTTY_KITTY_IMAGE_DATA_FORMAT)
            .and_then(KittyFormat::from_raw)?;
        let mut pointer: *const u8 = std::ptr::null();
        // SAFETY: the image handle is live; `DATA_DATA_PTR` writes a `const uint8_t *`, or reports no value while the
        // payload is pending (leaving null).
        unsafe {
            ffi::ghostty_kitty_graphics_image_get(
                image,
                ffi::GHOSTTY_KITTY_IMAGE_DATA_DATA_PTR,
                (&raw mut pointer).cast(),
            )
        };
        let len = image_value::<usize>(image, ffi::GHOSTTY_KITTY_IMAGE_DATA_DATA_LEN).unwrap_or(0);
        Some(KittyImage {
            id: u32_of(ffi::GHOSTTY_KITTY_IMAGE_DATA_ID),
            number: u32_of(ffi::GHOSTTY_KITTY_IMAGE_DATA_NUMBER),
            width: u32_of(ffi::GHOSTTY_KITTY_IMAGE_DATA_WIDTH),
            height: u32_of(ffi::GHOSTTY_KITTY_IMAGE_DATA_HEIGHT),
            format,
            generation: image_value::<u64>(image, ffi::GHOSTTY_KITTY_IMAGE_DATA_GENERATION).unwrap_or(0),
            // SAFETY: `DATA_PTR` points at `DATA_LEN` bytes owned by the storage, valid while the terminal is not
            // mutated, which the borrow of `self` (and so of the terminal) guarantees for `'_`.
            pixels: (!pointer.is_null()).then(|| unsafe { crate::raw_slice(pointer, len) }),
        })
    }

    /// The placement the iterator is on.
    fn placement(&self, iterator: ffi::GhosttyKittyGraphicsPlacementIterator) -> KittyPlacement {
        let u32_of = |data| placement_value::<u32>(iterator, data).unwrap_or(0);
        let image_id = u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_IMAGE_ID);
        KittyPlacement {
            image_id,
            placement_id: u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_PLACEMENT_ID),
            is_virtual: placement_value::<bool>(iterator, ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_IS_VIRTUAL)
                .unwrap_or(false),
            z: placement_value::<i32>(iterator, ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_Z).unwrap_or(0),
            x_offset: u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_X_OFFSET),
            y_offset: u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_Y_OFFSET),
            columns: u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_COLUMNS),
            rows: u32_of(ffi::GHOSTTY_KITTY_GRAPHICS_PLACEMENT_DATA_ROWS),
            render: self.render_info(iterator, image_id),
        }
    }

    fn render_info(
        &self,
        iterator: ffi::GhosttyKittyGraphicsPlacementIterator,
        image_id: u32,
    ) -> Option<KittyRenderInfo> {
        // SAFETY: as in `image`.
        let image = unsafe { ffi::ghostty_kitty_graphics_image(self.raw, image_id) };
        if image.is_null() {
            return None;
        }
        let mut info = ffi::GhosttyKittyGraphicsPlacementRenderInfo {
            size: std::mem::size_of::<ffi::GhosttyKittyGraphicsPlacementRenderInfo>(),
            pixel_width: 0,
            pixel_height: 0,
            grid_cols: 0,
            grid_rows: 0,
            viewport_col: 0,
            viewport_row: 0,
            viewport_visible: false,
            source_x: 0,
            source_y: 0,
            source_width: 0,
            source_height: 0,
        };
        // SAFETY: the iterator is positioned on a placement of `image`; the terminal is live and borrowed; `info` is a
        // sized out-struct.
        let result =
            unsafe { ffi::ghostty_kitty_graphics_placement_render_info(iterator, image, self.terminal, &raw mut info) };
        (result == ffi::GHOSTTY_SUCCESS).then_some(KittyRenderInfo {
            pixel_width: info.pixel_width,
            pixel_height: info.pixel_height,
            grid_cols: info.grid_cols,
            grid_rows: info.grid_rows,
            viewport_col: info.viewport_col,
            viewport_row: info.viewport_row,
            visible: info.viewport_visible,
            source: PixelRect {
                x: info.source_x,
                y: info.source_y,
                width: info.source_width,
                height: info.source_height,
            },
        })
    }
}

/// One `ghostty_kitty_graphics_image_get` datum of type `T`; `None` when the library has no value.
fn image_value<T: Copy + Default>(
    image: ffi::GhosttyKittyGraphicsImage,
    data: ffi::GhosttyKittyGraphicsImageData,
) -> Option<T> {
    let mut value = T::default();
    // SAFETY: the image handle is live (callers hold the terminal borrowed); callers pass the output type `data`
    // documents as `T`.
    let result = unsafe { ffi::ghostty_kitty_graphics_image_get(image, data, (&raw mut value).cast()) };
    (result == ffi::GHOSTTY_SUCCESS).then_some(value)
}

/// One `ghostty_kitty_graphics_placement_get` datum of type `T`; `None` when the library has no value.
fn placement_value<T: Copy + Default>(
    iterator: ffi::GhosttyKittyGraphicsPlacementIterator,
    data: ffi::GhosttyKittyGraphicsPlacementData,
) -> Option<T> {
    let mut value = T::default();
    // SAFETY: the iterator is live and positioned; callers pass the output type `data` documents as `T`.
    let result = unsafe { ffi::ghostty_kitty_graphics_placement_get(iterator, data, (&raw mut value).cast()) };
    (result == ffi::GHOSTTY_SUCCESS).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANIMATION: &[u8] = concat!(
        "\x1b_Ga=T,f=32,i=1,s=1,v=1,c=1,r=1;/wAA/w==\x1b\\",
        "\x1b_Ga=f,f=32,i=1,s=1,v=1,z=40;AAD//w==\x1b\\",
        "\x1b_Ga=a,i=1,r=1,z=30\x1b\\",
        "\x1b_Ga=a,i=1,s=3\x1b\\",
    )
    .as_bytes();

    fn snapshot(graphics: &KittyGraphics<'_>) -> (u64, Vec<u8>) {
        let image = graphics.image(1).unwrap();
        (image.generation, image.pixels.unwrap().to_vec())
    }

    fn assert_stopped(graphics: &mut KittyGraphics<'_>) {
        assert_eq!(graphics.animation_tick(1100), Ok(None));
        let mut untouched = 42;
        // SAFETY: the live exclusively borrowed storage and valid output match the C fixture's contract.
        let result = unsafe { ffi::ghostty_kitty_graphics_animation_tick(graphics.raw, 1100, &raw mut untouched) };
        assert_eq!(result, ffi::GHOSTTY_NO_VALUE);
        assert_eq!(untouched, 42);
    }

    #[test]
    fn animation_tick_advances_the_current_frame_on_absolute_deadlines() {
        let mut terminal = terminal();
        terminal.vt_write(ANIMATION);
        let mut graphics = terminal.kitty_graphics().unwrap();
        let red = snapshot(&graphics);
        assert_eq!(red.1, [255, 0, 0, 255]);
        assert_eq!(graphics.animation_tick(1000), Ok(Some(1030)));
        assert_eq!(snapshot(&graphics), red);
        assert_eq!(graphics.animation_tick(1010), Ok(Some(1030)));
        assert_eq!(snapshot(&graphics), red);
        assert_eq!(graphics.animation_tick(1030), Ok(Some(1070)));
        let blue = snapshot(&graphics);
        assert!(blue.0 > red.0);
        assert_eq!(blue.1, [0, 0, 255, 255]);
        assert_eq!(graphics.animation_tick(1070), Ok(Some(1100)));
        let red_again = snapshot(&graphics);
        assert!(red_again.0 > blue.0);
        assert_eq!(red_again.1, red.1);
        terminal.vt_write(b"\x1b_Ga=a,i=1,s=1\x1b\\");
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_stopped(&mut graphics);
        assert_eq!(snapshot(&graphics), red_again);
    }

    #[test]
    fn animation_tick_late_advances_only_one_frame() {
        let mut terminal = terminal();
        terminal.vt_write(ANIMATION);
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_eq!(graphics.animation_tick(1000), Ok(Some(1030)));
        assert_eq!(graphics.animation_tick(2000), Ok(Some(2040)));
        assert_eq!(snapshot(&graphics).1, [0, 0, 255, 255]);
    }

    #[test]
    fn animation_tick_without_playback_has_no_deadline_and_null_output_is_invalid() {
        let mut terminal = terminal();
        assert_eq!(terminal.kitty_graphics().unwrap().animation_tick(0), Ok(None));
        terminal.vt_write(b"\x1b_Ga=T,f=32,i=1,s=1,v=1,c=1,r=1;/wAA/w==\x1b\\");
        let mut graphics = terminal.kitty_graphics().unwrap();
        assert_eq!(graphics.animation_tick(1000), Ok(None));
        // SAFETY: a null output is explicitly accepted as an invalid argument by the C API.
        let result = unsafe { ffi::ghostty_kitty_graphics_animation_tick(graphics.raw, 0, std::ptr::null_mut()) };
        assert_eq!(result, ffi::GHOSTTY_INVALID_VALUE);
    }

    /// A 2x1 RGBA image sent directly (`f=32`), in two chunks, then placed.
    const CHUNKED: &[u8] = b"\x1b_Ga=T,f=32,s=2,v=1,i=7,m=1;AQIDBAUG\x1b\\\x1b_Gm=0;Bwg=\x1b\\";

    fn terminal() -> Terminal {
        let mut terminal = Terminal::new(20, 5).unwrap();
        terminal.resize(20, 5, 10, 20).unwrap();
        terminal.set_kitty_image_storage_limit(10_000_000).unwrap();
        terminal
    }

    #[test]
    fn cancelled_or_aborted_kitty_commands_store_nothing_and_send_no_reply() {
        use std::sync::{Arc, Mutex};
        struct Answers(Arc<Mutex<Vec<u8>>>);
        impl crate::Effects for Answers {
            fn write_pty(&mut self, data: &[u8]) {
                self.0.lock().unwrap().extend_from_slice(data);
            }
        }
        for exit in [0x18, 0x1a, 0x9b] {
            let mut terminal = terminal();
            let replies = Arc::new(Mutex::new(Vec::new()));
            terminal.set_effects(Box::new(Answers(replies.clone()))).unwrap();
            terminal.vt_write(b"\x1b_Ga=t,f=24,s=1,v=1,i=7;AAAA");
            terminal.vt_write(&[exit]);
            assert!(terminal.kitty_graphics().unwrap().image(7).is_none());
            assert!(replies.lock().unwrap().is_empty());
            terminal.vt_write(b"\x1b_Ga=t,f=24,s=1,v=1,i=7;AAAA\x1b\\");
            assert!(terminal.kitty_graphics().unwrap().image(7).is_some());
            assert_eq!(*replies.lock().unwrap(), b"\x1b_Gi=7;OK\x1b\\");
        }
    }

    #[test]
    fn a_chunked_transmission_stores_an_image_and_places_it() {
        let mut terminal = terminal();
        assert_eq!(terminal.kitty_graphics().unwrap().generation(), 0);
        terminal.vt_write(CHUNKED);
        let graphics = terminal.kitty_graphics().unwrap();
        assert_ne!(graphics.generation(), 0);
        let image = graphics.image(7).unwrap();
        assert_eq!((image.id, image.width, image.height), (7, 2, 1));
        assert_eq!(image.format, KittyFormat::Rgba);
        assert_eq!(image.pixels, Some(&[1, 2, 3, 4, 5, 6, 7, 8][..]));
        assert_ne!(image.generation, 0);
        let placements = graphics.placements().unwrap();
        assert_eq!(placements.len(), 1);
        let placement = placements[0];
        assert_eq!(placement.image_id, 7);
        assert!(!placement.is_virtual);
        let render = placement.render.unwrap();
        assert!(render.visible);
        assert_eq!((render.viewport_col, render.viewport_row), (0, 0));
        assert_eq!((render.grid_cols, render.grid_rows), (1, 1));
        assert_eq!(
            render.source,
            PixelRect {
                x: 0,
                y: 0,
                width: 2,
                height: 1
            }
        );
        assert!(graphics.image(8).is_none());
    }

    #[test]
    fn a_deletion_removes_the_placement() {
        let mut terminal = terminal();
        terminal.vt_write(CHUNKED);
        let before = terminal.kitty_graphics().unwrap().generation();
        terminal.vt_write(b"\x1b_Ga=d,d=I,i=7\x1b\\");
        let graphics = terminal.kitty_graphics().unwrap();
        assert!(graphics.generation() > before);
        assert!(graphics.placements().unwrap().is_empty());
        assert!(graphics.image(7).is_none());
    }

    #[test]
    fn a_virtual_placement_has_its_grid_but_no_position() {
        let mut terminal = terminal();
        terminal.vt_write(b"\x1b_Ga=T,U=1,q=2,f=32,s=2,v=1,i=9,p=4,c=3,r=2;AQIDBAUGBwg=\x1b\\");
        let graphics = terminal.kitty_graphics().unwrap();
        let placements = graphics.placements().unwrap();
        assert_eq!(placements.len(), 1);
        let placement = placements[0];
        assert!(placement.is_virtual);
        assert_eq!((placement.image_id, placement.placement_id), (9, 4));
        assert_eq!((placement.columns, placement.rows), (3, 2));
        let render = placement.render.unwrap();
        assert!(!render.visible);
        assert_eq!((render.grid_cols, render.grid_rows), (3, 2));
    }

    #[test]
    fn layers_follow_the_z_index() {
        assert_eq!(KittyLayer::of(i32::MIN), KittyLayer::BelowBackground);
        assert_eq!(KittyLayer::of(-1), KittyLayer::BelowText);
        assert_eq!(KittyLayer::of(0), KittyLayer::AboveText);
    }

    #[test]
    fn storage_off_stores_nothing() {
        let mut terminal = Terminal::new(20, 5).unwrap();
        terminal.set_kitty_image_storage_limit(0).unwrap();
        terminal.vt_write(CHUNKED);
        if let Some(graphics) = terminal.kitty_graphics() {
            assert!(graphics.image(7).is_none());
            assert!(graphics.placements().unwrap().is_empty());
        }
    }

    #[derive(Clone, Default)]
    struct Answers(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl crate::Effects for Answers {
        fn write_pty(&mut self, bytes: &[u8]) {
            self.0.lock().unwrap().extend_from_slice(bytes);
        }
    }

    fn file_payload(data: &[u8]) -> (tempfile::TempDir, String) {
        use base64::Engine as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.data");
        std::fs::write(&path, data).unwrap();
        let path = path.canonicalize().unwrap();
        let path = path.to_str().unwrap();
        // Ghostty rejects raw Windows device namespace paths; use the ordinary local drive spelling.
        #[cfg(windows)]
        let path = path.strip_prefix(r"\\?\").unwrap_or(path);
        (dir, base64::engine::general_purpose::STANDARD.encode(path))
    }

    /// Ghostty `graphics_exec.zig`: a non-direct transmission begins a new load, including its reply policy.
    #[test]
    fn a_file_transmission_abandons_a_chunked_image_and_uses_its_own_quiet_flag() {
        let (_dir, payload) = file_payload(&[1, 2, 3]);
        for (old_quiet, fresh_quiet) in [(2, 0), (0, 2)] {
            let mut terminal = terminal();
            terminal.set_kitty_image_media(true, None, false).unwrap();
            let answers = Answers::default();
            terminal.set_effects(Box::new(answers.clone())).unwrap();
            terminal.vt_write(format!("\x1b_Ga=t,f=24,s=1,v=2,i=1,m=1,q={old_quiet};AAAA\x1b\\").as_bytes());
            assert!(answers.0.lock().unwrap().is_empty());
            terminal.vt_write(format!("\x1b_Ga=t,t=f,f=24,s=1,v=1,i=2,q={fresh_quiet};{payload}\x1b\\").as_bytes());
            let graphics = terminal.kitty_graphics().unwrap();
            assert!(graphics.image(1).is_none());
            assert_eq!(graphics.image(2).unwrap().pixels, Some([1, 2, 3].as_slice()));
            assert_eq!(
                answers.0.lock().unwrap().as_slice(),
                if fresh_quiet == 0 {
                    b"\x1b_Gi=2;OK\x1b\\".as_slice()
                } else {
                    b""
                }
            );
        }
    }

    #[test]
    fn a_file_transmission_abandons_a_chunked_frame_and_keeps_only_the_new_frame() {
        let (_dir, payload) = file_payload(&[0, 255, 0, 0, 255, 0]);
        for (old_quiet, fresh_quiet) in [(2, 0), (0, 2)] {
            let mut terminal = terminal();
            terminal.set_kitty_image_media(true, None, false).unwrap();
            terminal.vt_write(b"\x1b_Ga=t,f=24,s=2,v=1,i=1,q=2;////////\x1b\\");
            let answers = Answers::default();
            terminal.set_effects(Box::new(answers.clone())).unwrap();
            terminal.vt_write(format!("\x1b_Ga=f,i=1,f=24,s=2,v=1,m=1,q={old_quiet};/wAA\x1b\\").as_bytes());
            assert!(answers.0.lock().unwrap().is_empty());
            terminal.vt_write(format!("\x1b_Ga=f,t=f,i=1,f=24,s=2,v=1,q={fresh_quiet};{payload}\x1b\\").as_bytes());
            assert_eq!(
                answers.0.lock().unwrap().as_slice(),
                if fresh_quiet == 0 {
                    b"\x1b_Gi=1,r=2;OK\x1b\\".as_slice()
                } else {
                    b""
                }
            );
            // Only the new green frame was appended; selecting a nonexistent third frame leaves it unchanged.
            terminal.vt_write(b"\x1b_Ga=a,i=1,c=2,s=1\x1b\\");
            assert_eq!(
                terminal.kitty_graphics().unwrap().image(1).unwrap().pixels,
                Some([0, 255, 0, 255, 0, 255, 0, 255].as_slice())
            );
            terminal.vt_write(b"\x1b_Ga=a,i=1,c=3,s=1\x1b\\");
            assert_eq!(
                terminal.kitty_graphics().unwrap().image(1).unwrap().pixels,
                Some([0, 255, 0, 255, 0, 255, 0, 255].as_slice())
            );
        }
    }

    #[test]
    fn erasing_history_removes_its_placements_and_relative_children() {
        let mut terminal = terminal();
        for id in 1..=4 {
            terminal.vt_write(format!("\x1b_Ga=t,f=24,s=1,v=1,i={id},q=2;AAAA\x1b\\").as_bytes());
        }
        terminal.vt_write(b"\x1b_Ga=p,i=1,p=1,c=1,r=1,C=1,q=2\x1b\\");
        terminal.vt_write(b"\x1b_Ga=p,i=3,p=1,P=1,Q=1,c=1,r=1,C=1,q=2\x1b\\");
        terminal.vt_write(b"\x1b[1S\x1b_Ga=p,i=2,p=1,c=1,r=1,C=1,q=2\x1b\\");
        terminal.vt_write(b"\x1b_Ga=p,i=4,p=1,U=1,c=1,r=1,q=2\x1b\\");
        assert_eq!(terminal.kitty_graphics().unwrap().placements().unwrap().len(), 4);
        terminal.vt_write(b"\x1b[3J");
        let graphics = terminal.kitty_graphics().unwrap();
        let mut ids: Vec<_> = graphics.placements().unwrap().iter().map(|p| p.image_id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [2, 4]);
        for id in 1..=4 {
            assert!(
                graphics.image(id).is_some(),
                "erase removes placements, retaining image {id}"
            );
        }
    }

    /// Ghostty `graphics_image.zig`: Darwin sandbox-style shm names have no leading slash.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn shared_memory_without_a_leading_slash_loads_and_is_unlinked() {
        use base64::Engine as _;
        use std::os::fd::{FromRawFd as _, OwnedFd};
        struct SharedName(std::ffi::CString);
        impl Drop for SharedName {
            fn drop(&mut self) {
                // SAFETY: a valid NUL-terminated name; ENOENT is expected after a successful load.
                unsafe { libc::shm_unlink(self.0.as_ptr()) };
            }
        }
        let prefix = if cfg!(target_os = "macos") {
            "ngterm.test/shm-"
        } else {
            "ngterm-test-shm-"
        };
        let name = std::ffi::CString::new(format!("{prefix}{}", std::process::id())).unwrap();
        // SAFETY: name is NUL-terminated; O_EXCL prevents modifying another process's object.
        let fd = unsafe { libc::shm_open(name.as_ptr(), libc::O_CREAT | libc::O_EXCL | libc::O_RDWR, 0o600) };
        assert!(fd >= 0, "shm_open: {}", std::io::Error::last_os_error());
        let name = SharedName(name);
        // SAFETY: a newly opened descriptor, owned exactly once from this point on.
        let _fd = unsafe { OwnedFd::from_raw_fd(fd) };
        // SAFETY: the live descriptor is writable and owned by this test.
        assert_eq!(unsafe { libc::ftruncate(fd, 3) }, 0);
        // SAFETY: the descriptor holds three bytes; mapping avoids write(2), unsupported by macOS shm.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                3,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };
        assert_ne!(ptr, libc::MAP_FAILED);
        // SAFETY: both buffers hold three bytes, do not overlap, and the mapping is writable.
        unsafe { std::ptr::copy_nonoverlapping([1u8, 2, 3].as_ptr(), ptr.cast(), 3) };
        // SAFETY: releases exactly the live mapping returned above, after its last use.
        assert_eq!(unsafe { libc::munmap(ptr, 3) }, 0);
        let mut terminal = terminal();
        terminal.set_kitty_image_media(false, None, true).unwrap();
        let payload = base64::engine::general_purpose::STANDARD.encode(name.0.as_bytes());
        terminal.vt_write(format!("\x1b_Ga=t,t=s,f=24,s=1,v=1,i=31;{payload}\x1b\\").as_bytes());
        assert_eq!(
            terminal.kitty_graphics().unwrap().image(31).unwrap().pixels,
            Some([1, 2, 3].as_slice())
        );
        // SAFETY: only probes the test's own name; no creation flags.
        let reopened = unsafe { libc::shm_open(name.0.as_ptr(), libc::O_RDWR, 0) };
        if reopened >= 0 {
            // SAFETY: close the unexpected descriptor before reporting the unlink failure.
            unsafe { libc::close(reopened) };
        }
        assert_eq!(reopened, -1);
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ENOENT));
    }
}
