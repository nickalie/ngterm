//! Inherited VT fixes in Ghostty Terminal.zig, Screen.zig and PageList.zig at the libghostty-vt pin.
use ngterm_ghostty_vt::{Point, PointTag, ScrollbackLimits, StyleFlags, Terminal};

#[test]
fn cub_and_backspace_stop_at_the_left_margin_or_the_screen_edge() {
    for movement in [b"\x1b[100D".as_slice(), b"\x08\x08\x08\x08\x08\x08\x08\x08"] {
        let mut terminal = Terminal::new(10, 10).unwrap();
        terminal.vt_write(b"\x1b[?69h\x1b[4;9s\x1b[1;8H");
        terminal.vt_write(movement);
        assert_eq!(terminal.cursor(), (3, 0));
        terminal.vt_write(b"\x1b[1;3H");
        terminal.vt_write(movement);
        assert_eq!(terminal.cursor(), (0, 0));
    }
}

#[test]
fn absolute_and_relative_axes_respect_origin_and_margins() {
    let mut terminal = Terminal::new(10, 10).unwrap();
    terminal.vt_write(b"\x1b[?69h\x1b[3;8r\x1b[4;9s\x1b[?6h");
    for col in [b"\x1b[1G".as_slice(), b"\x1b[1`"] {
        terminal.vt_write(b"\x1b[2;2H");
        terminal.vt_write(col);
        assert_eq!(terminal.cursor(), (3, 3));
        terminal.vt_write(b"\x1b[1d");
        assert_eq!(terminal.cursor(), (3, 2));
    }
    terminal.vt_write(b"\x1b[2;2H\x1b[2a");
    assert_eq!(terminal.cursor(), (6, 3));
    terminal.vt_write(b"\x1b[2e");
    assert_eq!(terminal.cursor(), (6, 5));
    terminal.vt_write(b"\x1b[100a\x1b[100e");
    assert_eq!(terminal.cursor(), (8, 7));
    terminal.vt_write(b"\x1b[?6l\x1b[3;4H\x1b[2a\x1b[1e");
    assert_eq!(terminal.cursor(), (5, 3));
    terminal.vt_write(b"\x1b[100a\x1b[100e");
    assert_eq!(terminal.cursor(), (9, 9));
}

#[test]
fn scroll_clear_pruning_keeps_the_cursor_style_and_open_link() {
    // Find the page boundary through retention: allocation rounding can change rows per page between builds.
    // Stop one line before natural pruning, leaving the top active row on the page ED22 will prune.
    let mut probe = zero_history_terminal().unwrap();
    let mut lines = 0;
    loop {
        let before = probe.total_rows();
        probe.vt_write(b"x\r\n");
        lines += 1;
        if probe.total_rows() < before {
            break;
        }
        assert!(lines < 1024, "zero history must eventually prune a page");
    }
    let mut terminal = zero_history_terminal().unwrap();
    for _ in 0..lines - 1 {
        terminal.vt_write(b"x\r\n");
    }
    let before = terminal.total_rows();
    terminal.vt_write(b"x\x1b[H\x1b[1m\x1b]8;;https://example.com/scroll-clear\x1b\\\x1b[22Jy");
    assert!(terminal.total_rows() < before, "ED22 must prune the original page");
    let cell = &terminal.active_row(0).unwrap().cells[0];
    assert_eq!(cell.text, "y");
    assert!(cell.style.flags.contains(StyleFlags::BOLD));
    assert_eq!(
        terminal
            .hyperlink_uri(Point::new(PointTag::Active, 0, 0))
            .unwrap()
            .as_deref(),
        Some("https://example.com/scroll-clear")
    );
    terminal.vt_write(b"\x1b]8;;\x1b\\z");
    assert_eq!(
        terminal.hyperlink_uri(Point::new(PointTag::Active, 1, 0)).unwrap(),
        None
    );
}

#[test]
fn a_max_width_page_can_reflow_to_two_columns() {
    let mut terminal = Terminal::new(u16::MAX, 1).unwrap();
    terminal.resize(2, 1, 8, 16).unwrap();
    terminal.vt_write(b"ab");
    assert_eq!((terminal.cols(), terminal.rows()), (2, 1));
    assert_eq!(terminal.active_row(0).unwrap().cells[0].text, "a");
    assert_eq!(terminal.active_row(0).unwrap().cells[1].text, "b");
}

fn zero_history_terminal() -> Result<Terminal, ngterm_ghostty_vt::Error> {
    let mut terminal = Terminal::new(215, 3)?;
    terminal.set_scrollback_limits(ScrollbackLimits {
        max_bytes: None,
        max_lines: Some(0),
    })?;
    Ok(terminal)
}
