//! A child redraws immediately on the OS resize notification, without waiting for test input.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Frame, GridSize, Pane, PaneCommand, PaneConfig, PaneEvent};

pub const BEFORE: GridSize = GridSize {
    cols: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
};
pub const AFTER: GridSize = GridSize {
    cols: 100,
    rows: 30,
    ..BEFORE
};

// The alternate screen never reflows, so parsing at 80 columns cannot be repaired by a later VT resize.
// XY occupies columns 99 and 100; the final cursor is also outside the old 80x24 grid.
pub const UNIX: &str = r#"
import os, signal
signal.signal(signal.SIGWINCH, lambda *_: os.write(1, b"\x1b[2J\x1b[11;99HXY\x1b[29;100H\x1b]2;redrawn\x07"))
os.write(1, b"\x1b[?1049h\x1b[2J\x1b[H\x1b]2;ready\x07")
while True:
    signal.pause()
"#;
pub const WINDOWS: &str = concat!(
    "$e=[char]27; $b=[char]7; ",
    "[Console]::Write($e+'[?1049h'+$e+'[2J'+$e+'[H'+$e+']2;ready'+$b); ",
    "while ([Console]::WindowWidth -ne 100 -or [Console]::WindowHeight -ne 30) { ",
    "Start-Sleep -Milliseconds 1 }; ",
    "[Console]::Write($e+'[2J'+$e+'[11;99HXY'+$e+'[29;100H'+$e+']2;redrawn'+$b); ",
    "Start-Sleep -Seconds 60"
);

pub fn spawn() -> Result<Pane, Box<dyn std::error::Error>> {
    let mut command = if cfg!(windows) {
        PaneCommand::new("powershell.exe").args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", WINDOWS])
    } else {
        PaneCommand::new("python3").args(["-c", UNIX])
    };
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("NGTERM_") || name.starts_with("AGTERM_") {
            command = command.env_remove(key);
        }
    }
    Ok(Pane::spawn(&command, &PaneConfig::new(BEFORE))?)
}

pub fn wait_title(pane: &Pane, title: &str) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if pane
            .take_events()
            .iter()
            .any(|event| matches!(event, PaneEvent::Title(got) if got == title))
        {
            return;
        }
        assert!(Instant::now() < deadline, "no {title} title from the redraw child");
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn assert_redrawn(pane: &Pane) -> Arc<Frame> {
    wait_title(pane, "redrawn");
    let deadline = Instant::now() + Duration::from_secs(60);
    let frame = loop {
        let snapshot = pane.frame();
        if !snapshot.stale {
            break snapshot.frame;
        }
        assert!(Instant::now() < deadline, "the redraw frame stayed stale");
        std::thread::yield_now();
    };
    assert_eq!((frame.cols, frame.rows.len()), (100, 30));
    assert_eq!(frame.lines()[10], format!("{}XY", " ".repeat(98)));
    assert_eq!(
        frame.cursor.position.map(|cursor| (cursor.column, cursor.row)),
        Some((99, 28))
    );
    frame
}
