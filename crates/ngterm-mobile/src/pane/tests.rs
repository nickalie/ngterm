//! The pane's pure parts: its terminal's configuration and the gone guard's line at the end of a channel's output.
//! The attach, its threads and how it ends run through the actor (`actor/pane_tests.rs`).

use std::sync::Arc;

use ngterm_core::remote::ssh::SESSION_GONE;
use ngterm_term::GridSize;

use super::{Tail, config};

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
