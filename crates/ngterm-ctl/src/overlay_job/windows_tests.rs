//! The overlay job's Windows platform, below what `overlay_job`'s own tests reach through a run: the console that is
//! never handed over, the resume of a program with no thread left, and the console's Ctrl handler.

use std::sync::mpsc;
use std::time::Instant;

use ngterm_core::overlay_jobs::OverlayJobFrame;
use ngterm_core::pane_env::PaneEnv;
use windows_sys::Win32::System::Console::CTRL_CLOSE_EVENT;

use super::super::{JobRunner, MAX_FRAME_BYTES};
use super::*;
use crate::client;

#[test]
fn the_console_is_never_handed_over_or_watched() {
    let console = stdin_terminal();
    assert!(!is_terminal(console));
    take_terminal_back(console);
    assert!(!hung_up(console));
}

/// A process that has exited has no thread to resume. Its handle stays open in `child`, so its pid is not reused.
#[test]
fn a_program_with_no_thread_left_cannot_be_resumed() {
    let mut child = std::process::Command::new("cmd.exe")
        .args(["/D", "/C", "exit 0"])
        .spawn()
        .unwrap();
    assert!(child.wait().unwrap().success());
    let error = resume(child.id()).unwrap_err();
    assert_eq!(error.to_string(), "the program's thread could not be resumed");
}

/// The app's end of a run: every frame the helper reports, as it arrives, until the connection ends.
fn origin_frames(origin: client::Connection) -> mpsc::Receiver<OverlayJobFrame> {
    let (sender, frames) = mpsc::channel();
    std::thread::spawn(move || {
        while let Some(line) = client::read_socket_line(&origin, MAX_FRAME_BYTES) {
            if sender.send(OverlayJobFrame::parse(&line).unwrap()).is_err() {
                return;
            }
        }
    });
    frames
}

/// The console's Ctrl+C and Ctrl+Break are the program's keys and leave the run alone; its close cancels the run and
/// holds the helper for the grace and half a second, so the run can report before the system ends the helper. The
/// handler is the first canceler's: routing the signals again changes nothing. A child keeps the process-wide
/// handler and canceler out of the shared test harness.
#[test]
fn the_consoles_close_cancels_the_run_and_its_keys_do_not() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "overlay_job::sys::tests::the_console_signal_probe",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
        .env("CTL_WINDOWS_SIGNAL_PROBE", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("the console signal probe ran"),
        "{output:?}"
    );
}

/// The scenario above, only in the child that opts in: installing the handler is for this process's whole life.
#[test]
fn the_console_signal_probe() {
    if std::env::var_os("CTL_WINDOWS_SIGNAL_PROBE").is_none() {
        return;
    }
    let grace = Duration::from_millis(200);
    let (origin, helper) = ngterm_server::endpoint::pair().unwrap();
    let frames = origin_frames(origin);
    // as run-job makes it, with the helper's console
    let runner = JobRunner::new(helper, Some(stdin_terminal()), grace);
    let (_, other_helper) = ngterm_server::endpoint::pair().unwrap();
    let other = JobRunner::new(other_helper, None, grace);
    route_signals(runner.canceler());
    route_signals(other.canceler());
    let cwd = std::env::temp_dir().to_string_lossy().into_owned();
    // cmd and ping both outlive a Ctrl+Break, so only the kill past the grace ends the job
    let context = OverlayLaunchContext::new("ping -n 30 127.0.0.1 >NUL", &cwd, PaneEnv::new());
    let base: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let status = std::thread::scope(|scope| {
        let running = scope.spawn(|| runner.run(&context, &base));
        let timeout = Duration::from_secs(10);
        assert_eq!(frames.recv_timeout(timeout).unwrap(), OverlayJobFrame::Started);
        for key in [CTRL_C_EVENT, CTRL_BREAK_EVENT] {
            // SAFETY: the handler is called as the system calls it, with an event code.
            assert_eq!(unsafe { on_console_event(key) }, TRUE);
        }
        // past the grace a cancel would have taken: the run goes on
        assert!(frames.recv_timeout(grace * 2).is_err(), "a key cancelled the run");
        assert!(!running.is_finished());
        let closing = Instant::now();
        // SAFETY: as above.
        assert_eq!(unsafe { on_console_event(CTRL_CLOSE_EVENT) }, TRUE);
        assert!(
            closing.elapsed() >= grace + Duration::from_millis(500),
            "the close did not wait"
        );
        running.join().unwrap()
    });
    assert_eq!(status, KILLED_STATUS);
    drop(runner);
    assert_eq!(frames.recv().unwrap(), OverlayJobFrame::Canceled);
    assert!(frames.recv().is_err(), "a frame after the report");
    println!("the console signal probe ran");
}
