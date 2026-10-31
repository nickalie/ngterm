//! A painted pane on every OS: what the painter draws reaches the terminal, an update repaints in place, a resize keeps
//! the frame, and the body file's removal ends the pane with status 0. Each wait follows the pane's own wake, never a
//! poll.

use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use crate::{ExitStatus, GridSize, Pane, PaneConfig, PaneEvent};

const SIZE: GridSize = GridSize {
    cols: 20,
    rows: 4,
    cell_width: 8,
    cell_height: 16,
};
/// How long one wake may take before the test fails, not a pacing delay.
const WAKE_LIMIT: Duration = Duration::from_secs(20);

/// A painted pane over `file`, and the wakes it sends.
fn painted(file: &Path) -> (Pane, Receiver<()>) {
    let (woke, wakes) = mpsc::channel();
    let woke = std::sync::Mutex::new(woke);
    let mut config = PaneConfig::new(SIZE);
    config.wake = Some(Arc::new(move || {
        let _ = woke.lock().unwrap().send(());
    }));
    (Pane::paint(file.to_path_buf(), &config).unwrap(), wakes)
}

/// Writes `body` as the core does: a staging file renamed over the body.
fn write_body(file: &Path, body: &str) {
    let staging = file.with_extension("txt.tmp");
    std::fs::write(&staging, body).unwrap();
    std::fs::rename(&staging, file).unwrap();
}

/// Waits for wakes until the screen's lines are `expected`, collecting the events on the way.
fn until_screen(pane: &Pane, wakes: &Receiver<()>, expected: &[&str], events: &mut Vec<PaneEvent>) {
    loop {
        let lines = pane.frame().frame.lines();
        events.extend(pane.take_events());
        if lines.iter().map(|line| line.trim_end()).eq(expected.iter().copied()) {
            return;
        }
        wakes
            .recv_timeout(WAKE_LIMIT)
            .unwrap_or_else(|_| panic!("no wake while the screen shows {lines:?}, waiting for {expected:?}"));
    }
}

/// Waits for wakes until the pane reports its end.
fn until_exited(pane: &Pane, wakes: &Receiver<()>) -> ExitStatus {
    loop {
        let _ = pane.frame();
        if let Some(status) = pane.take_events().into_iter().find_map(|event| match event {
            PaneEvent::Exited(status) => Some(status),
            _ => None,
        }) {
            return status;
        }
        wakes.recv_timeout(WAKE_LIMIT).expect("the pane ends");
    }
}

#[test]
fn a_painted_pane_shows_its_body_repaints_an_update_and_ends_with_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("body.txt");
    write_body(&file, "20 4 0 0 0.05 - 0\nready\n");
    let (pane, wakes) = painted(&file);
    assert_eq!(pane.pid(), None, "no child");
    let mut events = Vec::new();
    until_screen(&pane, &wakes, &["", "       ready", "", ""], &mut events);
    assert!(pane.write(b"ignored").is_ok(), "input is dropped, not refused");

    write_body(&file, "20 4 1 0 0.05 - 0 * *\nsecond\n\ndone\n");
    until_screen(&pane, &wakes, &["      * second", "", "        done", ""], &mut events);

    let wide = GridSize { cols: 30, ..SIZE };
    pane.resize(wide).unwrap();
    write_body(&file, "30 4 0 0 0.05 - 0\nwider\n");
    until_screen(&pane, &wakes, &["", "            wider", "", ""], &mut events);
    assert!(
        !events.iter().any(|event| matches!(event, PaneEvent::Exited(_))),
        "the painter runs while its file is there: {events:?}"
    );

    std::fs::remove_file(&file).unwrap();
    let status = until_exited(&pane, &wakes);
    assert_eq!((status.code, status.signal), (0, None));
}

#[test]
fn a_painted_pane_without_its_file_ends_at_once_and_one_dropped_stops_painting() {
    let dir = tempfile::tempdir().unwrap();
    let (gone, wakes) = painted(&dir.path().join("missing.txt"));
    assert_eq!(until_exited(&gone, &wakes).code, 0);

    let file = dir.path().join("body.txt");
    write_body(&file, "20 4 0 0 0.05 - 0\nkept\n");
    let (pane, wakes) = painted(&file);
    until_screen(&pane, &wakes, &["", "        kept", "", ""], &mut Vec::new());
    // dropping joins the pane's worker; the painter's thread leaves without reporting an end
    drop(pane);
    assert!(file.exists(), "the pane never removes the body");
}
