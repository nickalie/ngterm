//! The pane's pure parts: its terminal's configuration and the gone guard's line at the end of a channel's output.
//! The attach, its threads and how it ends run through the actor (`actor/pane_tests.rs`).

use std::sync::Arc;

use ngterm_core::remote::ssh::SESSION_GONE;
use std::time::{Duration, Instant};

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
fn the_gone_guards_line_is_found_before_the_attach_clients_restore() {
    // under a pty the attach client leaves with RIS and the colour resets (ngterm-session's `cli/attach.rs` RESTORE)
    let restore = "\x1bc\x1b]110\x1b\\\x1b]111\x1b\\\x1b]112\x1b\\";
    let mut tail = Tail::default();
    tail.push(format!("\x1b[2J\x1b[H{SESSION_GONE}\r\n{restore}").as_bytes());
    assert!(tail.is_gone(), "escape sequences after the line are not output");

    let mut bell = Tail::default();
    bell.push(format!("{SESSION_GONE}\r\n\x1b]110\x07\x1b[0m\r\n").as_bytes());
    assert!(bell.is_gone(), "nor are a BEL-ended OSC and a CSI");

    let mut later = Tail::default();
    later.push(format!("{SESSION_GONE}\r\n{restore}$ ").as_bytes());
    assert!(!later.is_gone(), "text after them still is");
}

#[test]
fn a_terminal_installed_after_a_resize_raced_its_build_takes_the_new_grid() {
    let built = GridSize {
        cols: 80,
        rows: 24,
        cell_width: 9,
        cell_height: 18,
    };
    let slot = PaneSlot::new(built, Arc::new(|| {}));
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

#[test]
fn the_views_theme_reaches_the_terminal_in_place_and_every_later_one() {
    use crate::api::{Pane, PaneColors, PaneInputError};

    let grid = GridSize {
        cols: 10,
        rows: 2,
        cell_width: 9,
        cell_height: 18,
    };
    let slot = Arc::new(PaneSlot::new(grid, Arc::new(|| {})));
    let pane = Pane::new("pane".to_owned(), Arc::clone(&slot));
    let colors = PaneColors {
        foreground: 0xffe3_e8ea,
        background: 0xff1b_2226,
        cursor: Some(0xffe3_884f),
        palette: vec![0xff2a_343a, 0xffe5_675c],
    };
    // with no terminal yet the theme is kept for the first one
    pane.set_colors(colors.clone()).unwrap();
    let (first, _feed, _out) = ngterm_term::Pane::stream(&config(grid, Arc::new(|| {}))).unwrap();
    slot.install(Arc::new(first), grid);
    // a frame taken while the terminal's IO thread holds its lock is the previous one, marked stale: wait for a
    // fresh one
    let theme = |slot: &PaneSlot| {
        let terminal = slot.terminal().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = terminal.frame();
            if !snapshot.stale || Instant::now() > deadline {
                return snapshot.frame.colors.clone();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let drawn = theme(&slot);
    assert_eq!(drawn.foreground, ngterm_term::Rgb::new(0xe3, 0xe8, 0xea));
    assert_eq!(drawn.background, ngterm_term::Rgb::new(0x1b, 0x22, 0x26));
    assert_eq!(drawn.cursor, Some(ngterm_term::Rgb::new(0xe3, 0x88, 0x4f)));
    assert_eq!(drawn.palette[1], ngterm_term::Rgb::new(0xe5, 0x67, 0x5c));

    // a change reaches the terminal in place, and a reattach's fresh terminal starts from it
    let light = PaneColors {
        background: 0xfff6_f8f8,
        cursor: None,
        ..colors
    };
    pane.set_colors(light).unwrap();
    assert_eq!(theme(&slot).background, ngterm_term::Rgb::new(0xf6, 0xf8, 0xf8));
    let (again, _feed, _out) = ngterm_term::Pane::stream(&config(grid, Arc::new(|| {}))).unwrap();
    slot.install(Arc::new(again), grid);
    let reattached = theme(&slot);
    assert_eq!(reattached.background, ngterm_term::Rgb::new(0xf6, 0xf8, 0xf8));
    assert_eq!(reattached.cursor, None);

    let too_many = PaneColors {
        foreground: 0,
        background: 0,
        cursor: None,
        palette: vec![0; 257],
    };
    assert!(matches!(pane.set_colors(too_many), Err(PaneInputError::Refused { .. })));
}
