//! The window host's zoom, followed until the window's maximize matches it.
//!
//! A maximize request reaches the window manager only at the right moment, and the panel cannot see it go astray.
//! weston (libweston's xdg-shell) answers no configure to a maximize change that comes between the client's ack of a
//! configure and the commit that applies it: it compares the request with the committed state, which still holds the
//! old maximize, finds nothing to change, and never looks again. GPUI acks a configure as it arrives and commits it with
//! the next frame it draws, after that frame's callbacks ran. X11's request toggles, and so does Windows' (each picks
//! maximize or restore from the state it reads), so a request sent again before the first one's answer undoes it.
//! The panel therefore settles over two frames: the first commits every configure acked before it, and the second
//! sends a request only when the maximize still reads as that frame committed it; otherwise a configure came in
//! between, acked and not yet committed, and the settle waits for the next two frames. It sends one request at a time
//! and settles again once the window's state changes.
//!
//! macOS has no maximize for the panel: `-[NSWindow zoom:]` does nothing on a window that is not resizable, as the
//! panel is. Its request there gives the panel its screen's visible frame, as upstream's panel takes it
//! (`QuickTerminalController.targetFrame`), and the unzoom gives back the frame it had ([`Restore`]). The kept frame
//! is the panel's maximize there, not the window's size against its screen, which stops matching once the screen's
//! visible frame changes under a zoomed panel; the request settles on it as the others do.

/// What the window does next for the panel's zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The window's maximize is the zoom's: the request is done.
    Done,
    /// A request is in flight: its answer, a change of the window's state, settles again.
    Wait,
    /// A configure changed the maximize since the last frame and is not committed yet: settle after the next one.
    Again,
    /// Ask the window manager to maximize or restore the window.
    Request,
}

/// The zoom the panel wants of its window and the request in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoomRequest {
    wanted: bool,
    /// The window's maximize when the last request went out, while it has not changed since.
    sent_from: Option<bool>,
}

impl ZoomRequest {
    #[must_use]
    pub fn new(wanted: bool) -> Self {
        Self {
            wanted,
            sent_from: None,
        }
    }

    /// The panel wants `wanted` now; a request in flight stays in flight.
    pub fn want(&mut self, wanted: bool) {
        self.wanted = wanted;
    }

    /// The step for the window's current maximize, `committed` the maximize as the last drawn frame committed it,
    /// recording a request the caller is to send.
    pub fn settle(&mut self, committed: bool, maximized: bool) -> Settle {
        if self.sent_from == Some(maximized) {
            return Settle::Wait;
        }
        if maximized == self.wanted {
            self.sent_from = None;
            return Settle::Done;
        }
        if committed != maximized {
            return Settle::Again;
        }
        self.sent_from = Some(maximized);
        Settle::Request
    }
}

/// A window's frame in its platform's coordinates: x, y, width, height.
#[cfg(any(target_os = "macos", test))]
pub type Rect = [f64; 4];

/// The frame the macOS panel had before its zoom filled the screen.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Restore(Option<Rect>);

#[cfg(any(target_os = "macos", test))]
impl Restore {
    /// Whether a zoom of the panel's filled its screen and no unzoom gave the frame back since.
    #[must_use]
    pub fn filled(&self) -> bool {
        self.0.is_some()
    }

    /// The frame a request gives the window: the kept frame for the unzoom of a zoom that [`filled`](Self::filled)
    /// the screen, whatever size the window has by now; else the screen's `visible` frame, `current` kept.
    pub fn target(&mut self, current: Rect, visible: Rect) -> Rect {
        self.0.take().unwrap_or_else(|| {
            self.0 = Some(current);
            visible
        })
    }
}

#[cfg(test)]
#[path = "zoom_tests.rs"]
mod tests;
