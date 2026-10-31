//! The frames upstream's `hud.sh` writes for the same bodies (its first tick, captured from the script), byte for byte.

use super::*;

const E: &str = "\x1b[E";
const HOME: &str = "\x1b[0m\x1b[H\x1b[J";
const RESET: &str = "\x1b[0m";

fn first(body: &str) -> String {
    Painter::new().tick(body).output.expect("a first tick paints")
}

#[test]
fn a_plain_line_is_centered_in_the_grid() {
    assert_eq!(
        first("20 5 0 0 1 - 0\nhello\n"),
        format!("{HOME}{E}{E}\x1b[7Chello{RESET}")
    );
}

#[test]
fn the_detail_after_the_separator_is_dimmed_in_the_text_color() {
    assert_eq!(
        first("30 7 0 0 1 38;2;1;2;3 0\nworking hard\n\nstep 2 of 3\nmore\n"),
        format!(
            "{HOME}\x1b[38;2;1;2;3m{E}\x1b[9Cworking hard{E}{E}\x1b[9C\x1b[2mstep 2 of 3{E}\x1b[13C\x1b[2mmore{RESET}"
        )
    );
}

#[test]
fn the_spinner_glyph_leads_the_first_line_and_advances_each_tick() {
    let body = "24 5 1 0 1 - 0 a b c\nspin\nsecond\n";
    let mut painter = Painter::new();
    let frame = |glyph: &str| format!("{HOME}{E}\x1b[9C{glyph} spin{E}\x1b[9Csecond{RESET}");
    assert_eq!(painter.tick(body).output, Some(frame("a")));
    assert_eq!(painter.tick(body).output, Some(frame("b")));
    assert_eq!(painter.tick(body).output, Some(frame("c")));
    assert_eq!(painter.tick(body).output, Some(frame("a")), "the frames wrap");
}

#[test]
fn a_spinner_without_frames_reserves_no_glyph() {
    assert_eq!(first("20 5 1 0 1 - 0\nhello\n"), first("20 5 0 0 1 - 0\nhello\n"));
}

#[test]
fn a_no_break_space_is_a_frame_not_a_separator() {
    assert_eq!(
        first("20 3 1 0 1 - 0 \u{00A0}\nhi\n"),
        format!("{HOME}{E}\x1b[8C\u{00A0} hi{RESET}")
    );
}

#[test]
fn markdown_rows_print_verbatim_at_one_offset() {
    assert_eq!(
        first("40 6 1 0 1 - 12 x y\n\x1b[1mBold\x1b[0m title\n\n  - item\n"),
        format!("{HOME}{E}\x1b[14Cx \x1b[1mBold\x1b[0m title{E}{E}\x1b[14C  - item{RESET}")
    );
}

#[test]
fn widths_count_code_points() {
    assert_eq!(
        first("20 4 0 0 1 - 0\nпривет мир\n"),
        format!("{HOME}{E}\x1b[5Cпривет мир{RESET}")
    );
}

#[test]
fn a_malformed_header_keeps_the_built_in_box() {
    assert_eq!(first("garbage x\nhi\n"), format!("{HOME}{E}{E}\x1b[19Chi{RESET}"));
    assert_eq!(
        first("20 5 0 0 1 3a;b 0\nx\n"),
        format!("{HOME}{E}{E}\x1b[9Cx{RESET}"),
        "a color that is not SGR parameters is dropped"
    );
    assert_eq!(
        first(""),
        format!("{HOME}{E}{E}{RESET}"),
        "an empty body paints the empty box"
    );
}

#[test]
fn a_last_line_without_a_newline_is_read_and_a_wide_one_starts_at_the_edge() {
    assert_eq!(
        first("10 3 1 0 1 - 0\ntoo long a line here"),
        format!("{HOME}{E}too long a line here{RESET}")
    );
}

#[test]
fn an_unchanged_frame_is_written_once_until_a_resize() {
    let body = "20 5 0 0 0.25 - 0\nhello\n";
    let mut painter = Painter::new();
    let tick = painter.tick(body);
    assert!(tick.output.is_some());
    assert_eq!(tick.interval, Duration::from_millis(250));
    assert_eq!(painter.tick(body).output, None);
    painter.forget();
    assert!(painter.tick(body).output.is_some(), "a resize paints again");
    assert!(
        painter.tick("20 5 0 0 0.25 - 0\nbye\n").output.is_some(),
        "an update paints"
    );
}

#[test]
fn the_interval_falls_back_and_never_spins() {
    let interval = |body: &str| Painter::new().tick(body).interval;
    assert_eq!(interval("20 5 0 0 x - 0\n"), DEFAULT_INTERVAL);
    assert_eq!(interval("20 5 0 0 0 - 0\n"), MIN_INTERVAL);
    assert_eq!(interval("20 5 0 0 0.08 - 0\n"), Duration::from_millis(80));
}

#[test]
fn the_body_the_app_writes_paints_its_message() {
    let spec = crate::hud::HudSpec {
        message: "deploying".to_owned(),
        detail: Some("step 1".to_owned()),
        spinner: Some(crate::hud::HudSpinner::Bar),
        ..crate::hud::HudSpec::new("deploying")
    };
    let grid = crate::hud::Grid { columns: 30, rows: 6 };
    let tick = Painter::new().tick(&crate::hud::rendered_body(&spec, grid, 1));
    assert_eq!(tick.interval, Duration::from_millis(100));
    let output = tick.output.unwrap();
    assert!(
        output.contains("| deploying") && output.contains("\x1b[2mstep 1"),
        "{output:?}"
    );
}
