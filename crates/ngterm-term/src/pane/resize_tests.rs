//! Force the child redraw to finish parsing before the resize call can proceed beyond PTY notification.
//! This catches a PTY-first reorder deterministically, rather than depending on which thread wins a race.

#[path = "../../tests/support/redraw.rs"]
mod redraw;

use super::{Frame, GridSize, Pane, PaneCommand, PaneConfig, PaneEvent};
use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static AFTER_PTY_RESIZE: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

pub(super) fn after_pty_resize() {
    let hook = AFTER_PTY_RESIZE.with(|slot| slot.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

#[test]
fn an_immediate_resize_redraw_is_parsed_before_resize_returns() {
    let pane = Arc::new(redraw::spawn().unwrap());
    redraw::wait_title(&pane, "ready");
    let child = Arc::clone(&pane);
    AFTER_PTY_RESIZE.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            redraw::assert_redrawn(&child);
        }));
    });
    pane.resize(redraw::AFTER).unwrap();
}
