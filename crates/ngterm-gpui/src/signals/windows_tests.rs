//! Console handling in a child: its process-wide registration cannot claim another test's session-end handler.

use std::process::Command;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use windows_sys::Win32::UI::WindowsAndMessaging::WM_ENDSESSION;

use super::*;

const CHILD: &str = "W_GPUI_CONSOLE_CHILD";

#[test]
fn console_events_post_quit_and_leave_the_second_interrupt_to_windows() {
    for first in ["c", "break"] {
        let mut child = Command::new(std::env::current_exe().unwrap());
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("NGTERM_") {
                child.env_remove(key);
            }
        }
        let output = child
            .args([
                "--exact",
                "signals::windows::tests::console_child",
                "--ignored",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, first)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{first}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct Queue(mpsc::Sender<MainTask<AppState>>);

impl MainLoop<AppState> for Queue {
    fn post(&self, task: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        self.0.send(task).map_err(|_| MainLoopClosed)
    }

    fn post_after(&self, _: Duration, _: MainTask<AppState>) -> Result<(), MainLoopClosed> {
        Err(MainLoopClosed)
    }
}

fn assert_quit(tasks: &mpsc::Receiver<MainTask<AppState>>) {
    let core = ngterm_core::Core::new(
        ngterm_core::CoreConfig {
            app: ngterm_proto::AppIdentity::from_recorded("test", None),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState::default(),
    );
    let mut state = AppState::new(core);
    assert!(!state.take_frontend_work().quit);
    tasks.recv_timeout(Duration::from_secs(2)).unwrap()(&mut state);
    assert!(
        state.take_frontend_work().quit,
        "the queued task must request an orderly quit"
    );
    assert!(tasks.try_recv().is_err(), "one quit per event");
}

fn console(kind: u32) -> i32 {
    // SAFETY: the handler only posts and waits; calling directly cannot invoke Windows's default termination.
    unsafe { on_console_event(kind) }
}

#[test]
#[ignore = "run by console_events_post_quit_and_leave_the_second_interrupt_to_windows"]
fn console_child() {
    let Ok(first) = std::env::var(CHILD) else {
        return;
    };
    request_quit("no main loop yet");
    let (sender, tasks) = mpsc::channel();
    let queue: Arc<dyn MainLoop<AppState>> = Arc::new(Queue(sender));
    quit_on_termination(Arc::downgrade(&queue)).unwrap();
    assert_eq!(
        quit_on_termination(Arc::downgrade(&queue)).unwrap_err().to_string(),
        "the quit handlers are installed already"
    );
    assert_eq!(console(u32::MAX), FALSE);
    assert!(tasks.try_recv().is_err());
    let (first, second) = if first == "c" {
        (CTRL_C_EVENT, CTRL_BREAK_EVENT)
    } else {
        (CTRL_BREAK_EVENT, CTRL_C_EVENT)
    };
    assert_eq!(console(first), TRUE);
    assert_quit(&tasks);
    assert_eq!(console(second), FALSE);
    assert!(
        tasks.try_recv().is_err(),
        "the second interrupt must use the default handler"
    );
    if first == CTRL_C_EVENT {
        for kind in [CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT] {
            let began = Instant::now();
            assert_eq!(console(kind), TRUE);
            assert!(began.elapsed() >= SHUTDOWN_GRACE);
            assert_quit(&tasks);
        }
        let began = Instant::now();
        crate::system_window::tests::send(WM_ENDSESSION, 1, 0);
        assert!(began.elapsed() >= SHUTDOWN_GRACE);
        assert_quit(&tasks);
    }
    // SAFETY: unregistering exactly this child's installed handler before exit.
    assert_ne!(unsafe { SetConsoleCtrlHandler(Some(on_console_event), FALSE) }, 0);
}
