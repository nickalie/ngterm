//! The pane's pure parts: its terminal's configuration and the gone guard's line at the end of a channel's output.
//! The attach, its threads and how it ends run through the actor (`actor/pane_tests.rs`).

use std::sync::Arc;

use ngterm_core::remote::ssh::SESSION_GONE;
use std::time::Duration;

use ngterm_term::{GridSize, Outgoing};

use super::{Tail, config};
use crate::api::PaneSlot;

#[test]
fn a_shown_panes_terminal_has_kitty_images_off_and_wakes_its_owner() {
    let size = GridSize {
        cols: 80,
        rows: 24,
        cell_width: 9,
        cell_height: 18,
    };
    let config = config(size, Arc::new(|| {}));
    assert_eq!(
        config.kitty_image_limit, 0,
        "the attach client's probe reports no support"
    );
    assert_eq!(config.size, size);
    assert!(config.wake.is_some());
}

#[test]
fn the_gone_guards_line_is_found_at_the_end_of_the_output_only() {
    let mut tail = Tail::default();
    tail.push(b"some output\r\n");
    assert!(!tail.is_gone());
    tail.push(format!("{SESSION_GONE}\r\n").as_bytes());
    assert!(tail.is_gone(), "the guard's line ends the output");

    let mut split = Tail::default();
    for byte in format!("{}{SESSION_GONE}\n", "x".repeat(4096)).bytes() {
        split.push(&[byte]);
    }
    assert!(split.is_gone(), "found across reads, after long output");

    let mut later = Tail::default();
    later.push(format!("{SESSION_GONE}\r\n$ ").as_bytes());
    assert!(!later.is_gone(), "a line printed earlier is not the attach's end");
}

#[test]
fn a_terminal_installed_after_a_resize_raced_its_build_takes_the_new_grid() {
    let built = GridSize {
        cols: 80,
        rows: 24,
        cell_width: 9,
        cell_height: 18,
    };
    let slot = PaneSlot::new(built);
    let (terminal, _feed, out) = ngterm_term::Pane::stream(&config(built, Arc::new(|| {}))).unwrap();
    // the view's resize lands between the build and the install
    let resized = GridSize { cols: 100, ..built };
    slot.set_size(resized);
    slot.install(Arc::new(terminal), built);
    assert_eq!(
        out.next(Some(Duration::from_secs(1))).unwrap(),
        Some(Outgoing::Resize(resized)),
        "the new terminal follows the grid the view gave"
    );

    // without a race nothing is resized
    let (terminal, _feed, out) = ngterm_term::Pane::stream(&config(resized, Arc::new(|| {}))).unwrap();
    slot.install(Arc::new(terminal), resized);
    assert_eq!(out.next(Some(Duration::from_millis(50))).unwrap(), None);
}
