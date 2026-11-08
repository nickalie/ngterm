//! The console's Ctrl handler on Windows. It is the process's for its whole life and a console's events reach every
//! process on it, so each scenario runs in a child of this test binary with a console of its own.

use std::io::{Read as _, Write as _};
use std::os::windows::process::CommandExt as _;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use windows_sys::Win32::Foundation::{FALSE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GenerateConsoleCtrlEvent, GetConsoleWindow, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE;
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
use windows_sys::core::BOOL;

use crate::state::HeadlessState;

/// Names the child that installs the handler and is sent its console's events.
const CHILD: &str = "HEADLESS_CONSOLE_SIGNAL_CHILD";
/// What the system ends a process with when its console's Ctrl handling ends it.
const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
/// How long a posted quit or an event is waited for.
const WAIT: Duration = Duration::from_secs(10);

/// Runs [`console_signal_child`] with `first` (`Ctrl+C` or `Ctrl+Break`) as the first key it sends, on a console of
/// its own; its exit status and output.
fn run_child(first: &str) -> (std::process::ExitStatus, String) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "signals::windows_tests::console_signal_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, first)
        .creation_flags(CREATE_NEW_CONSOLE)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NGTERM_") {
            command.env_remove(name);
        }
    }
    let mut child = command.spawn().unwrap();
    let (mut stdout, mut stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).unwrap();
        stderr.read_to_string(&mut text).unwrap();
        text
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("the console signal child did not end");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    (status, reader.join().unwrap())
}

/// The first Ctrl+C posts a quit and a Ctrl+Break after it gets the default action; the console's close then posts a
/// quit and holds the process until the system ends it.
#[test]
fn ctrl_c_and_the_consoles_close_post_an_orderly_quit() {
    let (status, output) = run_child("Ctrl+C");
    for step in [
        "Ctrl+C posted a quit",
        "Ctrl+Break got the default action",
        "the close posted a quit",
    ] {
        assert!(output.contains(step), "{step}: {output}");
    }
    // the system ended the closing console's process, past the handler's wait
    assert_eq!(
        status.code().map(i32::cast_unsigned),
        Some(STATUS_CONTROL_C_EXIT),
        "{output}"
    );
}

/// A first Ctrl+Break quits as a first Ctrl+C does, and a Ctrl+C after it gets the default action.
#[test]
fn a_first_ctrl_break_posts_an_orderly_quit() {
    let (status, output) = run_child("Ctrl+Break");
    for step in ["Ctrl+Break posted a quit", "Ctrl+C got the default action"] {
        assert!(output.contains(step), "{step}: {output}");
    }
    assert!(status.success(), "{status:?}: {output}");
}

/// A main loop that hands each posted task to the test.
struct Handing(Mutex<mpsc::Sender<MainTask<HeadlessState>>>);

impl MainLoop<HeadlessState> for Handing {
    fn post(&self, task: MainTask<HeadlessState>) -> Result<(), MainLoopClosed> {
        self.0.lock().unwrap().send(task).map_err(|_| MainLoopClosed)
    }

    fn post_after(&self, _: Duration, task: MainTask<HeadlessState>) -> Result<(), MainLoopClosed> {
        self.post(task)
    }
}

/// A headless state as the binary builds it, running.
fn running_state() -> HeadlessState {
    let config = ngterm_core::ghostty_config::GhosttyConfig::default();
    let core = ngterm_core::Core::new(
        ngterm_core::CoreConfig {
            app: crate::identity(),
            capabilities: Vec::new(),
        },
        ngterm_core::CoreState::default(),
    );
    let panes = crate::panes::Panes::new(
        crate::panes::PaneEnvironment {
            socket: "unused".into(),
            program_version: "test".into(),
            host: ngterm_term::Host::new(&config, None, None, None),
            config,
            grid: (80, 24),
            hostname: "box".into(),
        },
        Arc::new(|_, _| {}),
    );
    let poster: crate::state::Poster = std::sync::Weak::<Handing>::new();
    HeadlessState::new(core, poster, panes)
}

/// Asserts the next task posted is the quit: run on a running state, it requests the quit.
fn assert_quit_posted(tasks: &mpsc::Receiver<MainTask<HeadlessState>>) {
    let task = tasks.recv_timeout(WAIT).expect("no task posted");
    let mut state = running_state();
    assert!(!state.quit_requested());
    task(&mut state);
    assert!(state.quit_requested());
}

/// The events the handler under test left to the next one, in place of the default action that ends the process.
static DEFAULTED: Mutex<Option<mpsc::Sender<u32>>> = Mutex::new(None);

unsafe extern "system" fn record_default(kind: u32) -> BOOL {
    if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
        return FALSE;
    }
    if let Some(sender) = DEFAULTED.lock().unwrap().as_ref() {
        let _ = sender.send(kind);
    }
    TRUE
}

fn send_to_console(kind: u32) {
    // SAFETY: a plain call; group 0 is every process on this child's own console.
    let sent = unsafe { GenerateConsoleCtrlEvent(kind, 0) };
    assert_ne!(sent, FALSE, "{}", std::io::Error::last_os_error());
}

/// The scenarios above, only in the child that opts in, named by the first key it sends: installing the handler is for
/// this process's whole life.
#[test]
fn console_signal_child() {
    let Ok(first_name) = std::env::var(CHILD) else {
        return;
    };
    let (first, then, then_name) = if first_name == "Ctrl+C" {
        (CTRL_C_EVENT, CTRL_BREAK_EVENT, "Ctrl+Break")
    } else {
        (CTRL_BREAK_EVENT, CTRL_C_EVENT, "Ctrl+C")
    };
    let (defaulted_to, defaulted) = mpsc::channel();
    *DEFAULTED.lock().unwrap() = Some(defaulted_to);
    // handlers run last-installed first: this one stands in for the default action behind the one under test
    // SAFETY: a handler that only sends on a channel, for the whole process.
    assert_ne!(unsafe { SetConsoleCtrlHandler(Some(record_default), TRUE) }, FALSE);
    // a process started with Ctrl+C ignored never gets it: take it back
    // SAFETY: a plain call.
    assert_ne!(unsafe { SetConsoleCtrlHandler(None, FALSE) }, FALSE);
    let (posted, tasks) = mpsc::channel();
    let main_loop: Arc<dyn MainLoop<HeadlessState>> = Arc::new(Handing(Mutex::new(posted)));
    super::quit_on_termination(Arc::downgrade(&main_loop)).unwrap();
    let again = super::quit_on_termination(Arc::downgrade(&main_loop)).unwrap_err();
    assert_eq!(again.to_string(), "the quit handler is installed already");

    send_to_console(first);
    assert_quit_posted(&tasks);
    println!("{first_name} posted a quit");

    send_to_console(then);
    assert_eq!(defaulted.recv_timeout(WAIT), Ok(then));
    assert!(tasks.try_recv().is_err(), "a second interrupt posted again");
    println!("{then_name} got the default action");
    if first == CTRL_BREAK_EVENT {
        return;
    }

    // SAFETY: a plain call.
    let console = unsafe { GetConsoleWindow() };
    assert!(!console.is_null(), "the child has no console window");
    // SAFETY: a message with no pointers, to this process's own console's window.
    assert_ne!(unsafe { PostMessageW(console, WM_CLOSE, 0, 0) }, FALSE);
    assert_quit_posted(&tasks);
    println!("the close posted a quit");
    std::io::stdout().flush().unwrap();
    // the system ends this process from here, past its exit handlers
    ngterm_app::coverage::write_profile();
    std::thread::sleep(super::SHUTDOWN_GRACE * 2);
    panic!("the console's close did not end the process");
}
