//! Frames a page hands to the consumer, and the acknowledgement protocol that gives its buffers back.
//!
//! `WebKit` renders a frame into one of its buffers, hands it over ([`Frame`]) and renders the next one only after
//! the consumer says the frame was shown ([`crate::Host::frame_presented`]), which marks the buffer rendered. A
//! dmabuf the consumer shows stays in use until a newer one replaces it on screen, so a buffer is released one frame
//! after it was presented; an SHM frame is copied out when it arrives, so its buffer goes back at once. `WebKit`
//! replaces its buffer set under the host (2 buffers, recreated on load, grown to 3): an acknowledgement for a buffer
//! that left the set is dropped, and the consumer forgets an imported buffer only when it leaves the set
//! ([`crate::Event::BuffersRemoved`]). [`FrameFlow`] is that protocol without the engine.

use std::os::fd::OwnedFd;
use std::sync::Arc;

use crate::spec::ViewId;
use serde::{Deserialize, Serialize};

/// A buffer of a page's current set; ids are never reused, so an id names one buffer for good.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BufferId(pub(crate) u64);

crate::raw_ids!(BufferId);

/// A rectangle in buffer pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// One memory plane of a dmabuf.
#[derive(Debug)]
pub struct Plane {
    /// The host's own duplicate of the buffer's fd, valid while this value lives.
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
}

/// A GPU buffer to import (`WPEBufferDMABuf`): every memory plane, since a compressed buffer keeps several in one
/// object (Intel CCS: main, aux and clear color, a non-disjoint image).
#[derive(Debug)]
pub struct DmaBuf {
    pub width: u32,
    pub height: u32,
    /// The DRM fourcc ([`crate::fourcc`]).
    pub fourcc: u32,
    pub modifier: u64,
    pub planes: Vec<Plane>,
}

/// Damaged pixels of an SHM frame, copied out of the engine's buffer: premultiplied ARGB, 32 bits per pixel in native
/// byte order (`WPE_PIXEL_FORMAT_ARGB8888`, B G R A in memory on a little-endian machine), rows `rect.width * 4`
/// bytes apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShmRegion {
    pub rect: Rect,
    pub pixels: Vec<u8>,
}

/// What a frame carries.
#[derive(Debug, Clone)]
pub enum FrameContent {
    /// The whole buffer, shared by every frame drawn into it; the consumer imports it once per [`BufferId`].
    DmaBuf(Arc<DmaBuf>),
    /// The damaged regions only: the consumer keeps the rest from earlier frames.
    Shm(Vec<ShmRegion>),
}

/// A frame of page `view`.
#[derive(Debug, Clone)]
pub struct Frame {
    pub view: ViewId,
    pub buffer: BufferId,
    /// The buffer's size in pixels.
    pub width: u32,
    pub height: u32,
    /// What changed since the previous frame; the whole buffer when the engine names nothing.
    pub damage: Vec<Rect>,
    pub content: FrameContent,
}

/// Clips damage to the frame bounds.
#[must_use]
pub fn clip(rect: Rect, width: u32, height: u32) -> Option<Rect> {
    let right = rect.x.saturating_add(rect.width).min(width);
    let bottom = rect.y.saturating_add(rect.height).min(height);
    (rect.x < right && rect.y < bottom).then(|| Rect {
        x: rect.x,
        y: rect.y,
        width: right - rect.x,
        height: bottom - rect.y,
    })
}
