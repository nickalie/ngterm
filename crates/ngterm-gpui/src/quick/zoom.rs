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

#[cfg(test)]
#[path = "zoom_tests.rs"]
mod tests;
