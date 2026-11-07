use std::sync::{Arc, Mutex};

use super::*;
use crate::Terminal;

struct Defaults;
impl Effects for Defaults {}

#[test]
fn default_handlers_leave_terminal_state_and_queries_usable() {
    let mut terminal = Terminal::new(20, 3).unwrap();
    terminal.set_effects(Box::new(Defaults)).unwrap();
    terminal.vt_write(b"\x07\x1b]2;title\x07\x1b]7;file://host/tmp\x07");
    terminal.vt_write(b"\x1b]9;notice\x07\x1b]9;4;1;50\x07\x1b]7501;state=working\x07");
    terminal.vt_write(b"\x1b]52;c;aGk=\x07\x1b]52;c;?\x07");
    terminal.vt_write(b"\x1b[?2026h\x1b[?2026l\x1b[18t\x1b[c\x1b[?996n\x1b[>q");
    assert_eq!(terminal.title(), "title");
    assert_eq!(terminal.pwd(), "file://host/tmp");
    terminal.vt_write(b"\x1bcok");
    assert_eq!(terminal.title(), "");
    assert_eq!(terminal.pwd(), "");
    assert_eq!(terminal.active_row(0).unwrap().cells[0].text, "o");
    assert_eq!(
        format!("{:?}", EffectsSlot::new(Box::new(Defaults))),
        "EffectsSlot { .. }"
    );
}

#[derive(Default)]
struct Observed {
    bytes: Vec<u8>,
    progress: Vec<ProgressReport>,
    status: Vec<ProgramStatus>,
    locations: Vec<ClipboardLocation>,
}

struct Recorder {
    seen: Arc<Mutex<Observed>>,
    write: ClipboardWriteResult,
    read: ClipboardReadReply,
    suppress_size: bool,
}

impl Effects for Recorder {
    fn write_pty(&mut self, data: &[u8]) {
        self.seen.lock().unwrap().bytes.extend_from_slice(data);
    }

    fn progress_report(&mut self, report: ProgressReport) {
        self.seen.lock().unwrap().progress.push(report);
    }

    fn program_status(&mut self, status: ProgramStatus) {
        self.seen.lock().unwrap().status.push(status);
    }

    fn clipboard_write(&mut self, request: &ClipboardWriteRequest<'_>) -> ClipboardWriteResult {
        self.seen.lock().unwrap().locations.push(request.location);
        self.write
    }

    fn clipboard_read(&mut self, _: &ClipboardReadRequest<'_>) -> ClipboardReadReply {
        self.read.clone()
    }

    fn size(&mut self, current: SizeReport) -> Option<SizeReport> {
        (!self.suppress_size).then_some(current)
    }
}

fn recording(write: ClipboardWriteResult, read: ClipboardReadReply) -> (Terminal, Arc<Mutex<Observed>>) {
    let seen = Arc::new(Mutex::new(Observed::default()));
    let mut terminal = Terminal::new(20, 3).unwrap();
    terminal
        .set_effects(Box::new(Recorder {
            seen: Arc::clone(&seen),
            write,
            read,
            suppress_size: false,
        }))
        .unwrap();
    (terminal, seen)
}

#[test]
fn clipboard_write_results_are_reported_to_the_requesting_program() {
    for (result, status) in [
        (ClipboardWriteResult::Success, "DONE"),
        (ClipboardWriteResult::Denied, "EPERM"),
        (ClipboardWriteResult::Unsupported, "ENOSYS"),
        (ClipboardWriteResult::Busy, "EBUSY"),
        (ClipboardWriteResult::InvalidData, "EINVAL"),
        (ClipboardWriteResult::IoError, "EIO"),
    ] {
        let (mut terminal, seen) = recording(result, ClipboardReadReply::Denied);
        terminal.vt_write(b"\x1b]5522;type=write:id=42\x1b\\\x1b]5522;type=wdata\x1b\\");
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.bytes,
            format!("\x1b]5522;type=write:status={status}:id=42\x1b\\").as_bytes()
        );
        assert_eq!(seen.locations, [ClipboardLocation::Standard]);
    }
}

#[test]
fn clipboard_read_refusals_keep_their_protocol_status() {
    for (reply, status) in [
        (ClipboardReadReply::Denied, "EPERM"),
        (ClipboardReadReply::Unsupported, "ENOSYS"),
        (ClipboardReadReply::Busy, "EBUSY"),
        (ClipboardReadReply::IoError, "EIO"),
    ] {
        let (mut terminal, seen) = recording(ClipboardWriteResult::Denied, reply);
        terminal.vt_write(b"\x1b]5522;type=read:id=x;dGV4dC9wbGFpbg==\x1b\\");
        assert_eq!(
            seen.lock().unwrap().bytes,
            format!("\x1b]5522;type=read:status={status}:id=x\x1b\\").as_bytes()
        );
    }
}

#[test]
fn clipboard_selection_location_reaches_the_handler() {
    let (mut terminal, seen) = recording(ClipboardWriteResult::Success, ClipboardReadReply::Denied);
    terminal.vt_write(b"\x1b]52;s;aGk=\x07");
    assert_eq!(seen.lock().unwrap().locations, [ClipboardLocation::Selection]);
}

#[test]
fn progress_error_and_pause_keep_their_percentages() {
    let (mut terminal, seen) = recording(ClipboardWriteResult::Denied, ClipboardReadReply::Denied);
    terminal.vt_write(b"\x1b]9;4;2;100\x07\x1b]9;4;4;0\x07");
    assert_eq!(
        seen.lock().unwrap().progress,
        [
            ProgressReport {
                state: ProgressState::Error,
                progress: Some(100)
            },
            ProgressReport {
                state: ProgressState::Pause,
                progress: Some(0)
            },
        ]
    );
}

#[test]
fn program_status_variants_reach_the_handler_with_their_text() {
    let (mut terminal, seen) = recording(ClipboardWriteResult::Denied, ClipboardReadReply::Denied);
    for (state, expected) in [
        ("idle", ProgramStatusState::Idle),
        ("working", ProgramStatusState::Working),
        ("done", ProgramStatusState::Done),
        ("error", ProgramStatusState::Error),
        ("clear", ProgramStatusState::Clear),
    ] {
        terminal.vt_write(format!("\x1b]7501;state={state}:id=job:app=test\x07").as_bytes());
        let mut seen = seen.lock().unwrap();
        assert_eq!(
            seen.status.pop().unwrap(),
            ProgramStatus {
                state: expected,
                kind: ProgramStatusKind::None,
                progress: None,
                id: "job".to_owned(),
                app: "test".to_owned(),
                title: String::new(),
                message: String::new(),
            }
        );
    }
    for (kind, expected) in [
        ("question", ProgramStatusKind::Question),
        ("auth", ProgramStatusKind::Auth),
    ] {
        terminal.vt_write(format!("\x1b]7501;state=blocked:kind={kind}:progress=73:title=SGk=\x07").as_bytes());
        let status = seen.lock().unwrap().status.pop().unwrap();
        assert_eq!(status.kind, expected);
        assert_eq!(status.progress, Some(73));
        assert_eq!(status.title, "Hi");
    }
}

#[test]
fn a_size_handler_can_suppress_geometry_reports() {
    let seen = Arc::new(Mutex::new(Observed::default()));
    let mut terminal = Terminal::new(20, 3).unwrap();
    terminal
        .set_effects(Box::new(Recorder {
            seen: Arc::clone(&seen),
            write: ClipboardWriteResult::Denied,
            read: ClipboardReadReply::Denied,
            suppress_size: true,
        }))
        .unwrap();
    terminal.vt_write(b"\x1b[18t\x1b[14t");
    assert!(seen.lock().unwrap().bytes.is_empty());
}
