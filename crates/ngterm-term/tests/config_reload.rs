//! Config reload reports use the same outbound queue as query replies, on every platform.
use std::time::{Duration, Instant};

use ngterm_term::{ColorScheme, GridSize, Outgoing, Pane, PaneConfig, StreamFeed};

const WAIT: Duration = Duration::from_secs(10);
const SIZE: GridSize = GridSize {
    cols: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
};

fn feed(pane: &Pane, input: &StreamFeed, bytes: &[u8], marker: &str) {
    assert!(input.deliver(&[bytes, marker.as_bytes()].concat()));
    let deadline = Instant::now() + WAIT;
    while !pane.frame().frame.lines().iter().any(|line| line.contains(marker)) {
        assert!(Instant::now() < deadline, "output did not reach {marker}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn config_reload_reports_unchanged_dark_light_and_changed_scheme_once_only_when_enabled() {
    for scheme in [ColorScheme::Dark, ColorScheme::Light] {
        let mut config = PaneConfig::new(SIZE);
        config.color_scheme = scheme;
        let (pane, input, output) = Pane::stream(&config).unwrap();
        pane.config_reloaded(scheme);
        assert_eq!(output.queued(), 0, "mode 2031 off");
        feed(&pane, &input, b"\x1b[?2031h", "enabled");
        let report: &[u8] = if scheme == ColorScheme::Dark {
            b"\x1b[?997;1n"
        } else {
            b"\x1b[?997;2n"
        };
        for _ in 0..2 {
            pane.config_reloaded(scheme);
            assert_eq!(output.next(Some(WAIT)), Ok(Some(Outgoing::Bytes(report.to_vec()))));
        }
        let other = if scheme == ColorScheme::Dark {
            ColorScheme::Light
        } else {
            ColorScheme::Dark
        };
        pane.config_reloaded(other);
        let report: &[u8] = if other == ColorScheme::Dark {
            b"\x1b[?997;1n"
        } else {
            b"\x1b[?997;2n"
        };
        // Queries after the reload see the configured scheme and follow its single report.
        feed(&pane, &input, b"\x1b[?996n\x1b[5n", "queried");
        assert_eq!(
            output.next(Some(WAIT)),
            Ok(Some(Outgoing::Bytes([report, report, b"\x1b[0n"].concat())))
        );
        assert_eq!(output.queued(), 0);
        feed(&pane, &input, b"\x1b[?2031l", "disabled");
        pane.config_reloaded(scheme);
        assert_eq!(output.queued(), 0, "mode 2031 disabled again");
    }
}

#[test]
fn a_config_reload_joins_a_queued_reply_burst_without_stalling_or_overtaking_it() {
    let (pane, input, output) = Pane::stream(&PaneConfig::new(SIZE)).unwrap();
    feed(
        &pane,
        &input,
        &[b"\x1b[?2031h".as_slice(), &b"\x1b[5n".repeat(4096)].concat(),
        "burst",
    );
    pane.config_reloaded(ColorScheme::Dark);
    feed(&pane, &input, b"\x1b[5n", "after");
    assert_eq!(
        output.next(Some(WAIT)),
        Ok(Some(Outgoing::Bytes(
            [b"\x1b[0n".repeat(4096), b"\x1b[?997;1n\x1b[0n".to_vec()].concat()
        )))
    );
}
