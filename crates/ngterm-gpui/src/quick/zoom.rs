//! The window host's zoom, followed until the window's maximize matches it.
//!
//! A maximize request reaches the window manager only at the right moment, and the panel cannot see it go astray.
//! weston (libweston's xdg-shell) answers no configure to a maximize change that comes between the client's ack of a
//! configure and the commit that applies it: it compares the request with the committed state, which still holds the
//! old maximize, finds nothing to change, and never looks again. X11's request toggles, and so does Windows' (each
//! picks maximize or restore from the state it reads), so a request sent again before the first one's answer undoes it.
//! The panel therefore sends a request only after a frame was presented (which commits any configure it acked), sends
//! one at a time, and checks again once the window's state changes.

/// What the window does next for the panel's zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// The window's maximize is the zoom's: the request is done.
    Done,
    /// A request is in flight: its answer, a change of the window's state, settles again.
    Wait,
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

    /// The step for the window's current maximize, recording a request the caller is to send.
    pub fn settle(&mut self, maximized: bool) -> Settle {
        if self.sent_from == Some(maximized) {
            return Settle::Wait;
        }
        if maximized == self.wanted {
            self.sent_from = None;
            return Settle::Done;
        }
        self.sent_from = Some(maximized);
        Settle::Request
    }
}

#[cfg(test)]
#[path = "zoom_tests.rs"]
mod tests;
