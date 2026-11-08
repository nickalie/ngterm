//! The console's Ctrl handler on Windows. It is the process's for its whole life and a console's events reach every
//! process on it, so each scenario runs in a child of this test binary on a pseudo console of its own, which the test
//! closes as a terminal closing its window or tab does. A console window of the child's own would not do: in an
//! interactive session, as on CI's runners, the system hands a new console to the default terminal (Windows Terminal),
//! and the window the child then has, a pseudo console's, ignores a `WM_CLOSE`.

use std::ffi::{OsStr, c_void};
use std::io::Read as _;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use ngterm_host::{MainLoop, MainLoopClosed, MainTask};
use windows_sys::Win32::Foundation::{FALSE, INVALID_HANDLE_VALUE, S_OK, TRUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{
    COORD, CTRL_BREAK_EVENT, CTRL_C_EVENT, ClosePseudoConsole, CreatePseudoConsole, GenerateConsoleCtrlEvent, HPCON,
    SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW,
    TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::core::BOOL;

use crate::state::HeadlessState;

/// Names the child that installs the handler and is sent its console's events.
const CHILD: &str = "HEADLESS_CONSOLE_SIGNAL_CHILD";
/// Names the file the child writes each step it passed to: what it shows on its console ends with the console.
const STEPS: &str = "HEADLESS_CONSOLE_SIGNAL_STEPS";
/// The step after which the child waits for its console's close.
const READY_TO_CLOSE: &str = "waiting for the close";
/// What the system ends a process with when its console's Ctrl handling ends it.
const STATUS_CONTROL_C_EXIT: u32 = 0xC000_013A;
/// How long a posted quit or an event is waited for.
const WAIT: Duration = Duration::from_secs(10);
/// How long the child is given to end.
const CHILD_WAIT: Duration = Duration::from_secs(60);

/// A pseudo console of the test's own and the thread draining what it shows, which the console host would stall on.
struct PseudoConsole {
    handle: HPCON,
    /// Its input, held open until the close: the console host ends with it.
    _input: std::io::PipeWriter,
    shown: mpsc::Receiver<String>,
}

impl PseudoConsole {
    fn new() -> Self {
        let (input_read, input) = std::io::pipe().unwrap();
        let (mut output, output_write) = std::io::pipe().unwrap();
        let mut handle: HPCON = 0;
        let size = COORD { X: 120, Y: 40 };
        // SAFETY: two open pipe ends, which the console host takes copies of, and a place for the handle.
        let created = unsafe {
            CreatePseudoConsole(
                size,
                input_read.as_raw_handle(),
                output_write.as_raw_handle(),
                0,
                &raw mut handle,
            )
        };
        assert_eq!(created, S_OK, "CreatePseudoConsole: {created:#x}");
        let (show, shown) = mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = output.read_to_end(&mut bytes);
            let _ = show.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        Self {
            handle,
            _input: input,
            shown,
        }
    }

    /// Closes it as a terminal closing its window or tab does: its processes get the console's close. What it showed.
    fn close(mut self) -> String {
        self.release();
        self.shown
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| "(its output did not end)".to_owned())
    }

    /// Closes the console unless that was done already.
    fn release(&mut self) {
        let handle = std::mem::take(&mut self.handle);
        if handle != 0 {
            // SAFETY: the handle CreatePseudoConsole made, taken out of `self` so it is closed once.
            unsafe { ClosePseudoConsole(handle) };
        }
    }
}

impl Drop for PseudoConsole {
    fn drop(&mut self) {
        self.release();
    }
}

/// A process attribute list holding one pseudo console.
struct AttributeList(Vec<usize>);

impl AttributeList {
    fn with_pseudo_console(console: HPCON) -> Self {
        let mut size = 0;
        // SAFETY: a size query: no list, only the size written. It fails with the size wanted.
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &raw mut size) };
        let mut buffer = vec![0_usize; size.div_ceil(size_of::<usize>())];
        // SAFETY: a pointer-aligned buffer of at least the size asked for.
        let initialized = unsafe { InitializeProcThreadAttributeList(buffer.as_mut_ptr().cast(), 1, 0, &raw mut size) };
        assert_ne!(initialized, FALSE, "{}", std::io::Error::last_os_error());
        let mut list = Self(buffer);
        // SAFETY: an initialized list with room for one attribute; a pseudo console's value is its handle itself.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                list.as_mut_ptr(),
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                std::ptr::without_provenance::<c_void>(console.cast_unsigned()),
                size_of::<HPCON>(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert_ne!(updated, FALSE, "{}", std::io::Error::last_os_error());
        list
    }

    fn as_mut_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.0.as_mut_ptr().cast()
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: a list InitializeProcThreadAttributeList initialized, deleted once.
        unsafe { DeleteProcThreadAttributeList(self.as_mut_ptr()) };
    }
}

/// `text` as a NUL-terminated wide string.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// This process's environment without `NGTERM_*`, with `extra` set: the block `CreateProcessW` takes.
fn environment_block(extra: &[(&str, &OsStr)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in std::env::vars_os() {
        let name_text = name.to_string_lossy();
        if name_text.starts_with("NGTERM_") || extra.iter().any(|(set, _)| name_text.eq_ignore_ascii_case(set)) {
            continue;
        }
        block.extend(name.encode_wide().chain("=".encode_utf16()).chain(value.encode_wide()));
        block.push(0);
    }
    for (name, value) in extra {
        block.extend(name.encode_utf16().chain("=".encode_utf16()).chain(value.encode_wide()));
        block.push(0);
    }
    block.push(0);
    block
}

/// Starts [`console_signal_child`] on `console` with `first` (`Ctrl+C` or `Ctrl+Break`) as the first key it sends and
/// its steps written to `steps`; its process.
fn spawn_child(console: &PseudoConsole, first: &str, steps: &Path) -> Child {
    let exe = std::env::current_exe().unwrap();
    let mut command_line = wide(OsStr::new(&format!(
        "\"{}\" --exact signals::windows_tests::console_signal_child --nocapture --test-threads=1",
        exe.display()
    )));
    let environment = environment_block(&[(CHILD, OsStr::new(first)), (STEPS, steps.as_os_str())]);
    let mut attributes = AttributeList::with_pseudo_console(console.handle);
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = u32::try_from(size_of::<STARTUPINFOEXW>()).unwrap();
    // none of this process's own standard handles: the child's are its console's
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdOutput = INVALID_HANDLE_VALUE;
    startup.StartupInfo.hStdError = INVALID_HANDLE_VALUE;
    startup.lpAttributeList = attributes.as_mut_ptr();
    let mut process = PROCESS_INFORMATION::default();
    // SAFETY: a writable command line and an environment block, both NUL-terminated, and an extended startup info whose
    // attribute list outlives the call.
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            FALSE,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            std::ptr::null(),
            std::ptr::from_ref(&startup).cast::<STARTUPINFOW>(),
            &raw mut process,
        )
    };
    assert_ne!(created, FALSE, "{}", std::io::Error::last_os_error());
    // SAFETY: a thread handle CreateProcessW opened, closed here.
    drop(unsafe { OwnedHandle::from_raw_handle(process.hThread) });
    // SAFETY: the process handle CreateProcessW opened, owned from here on.
    Child(unsafe { OwnedHandle::from_raw_handle(process.hProcess) })
}

/// The child's process, which goes with the test however the test ends: one left running would keep its console and
/// the steps' directory.
struct Child(OwnedHandle);

impl Child {
    /// Whether it ended, waiting up to `wait` for it.
    fn ended(&self, wait: Duration) -> bool {
        let milliseconds = u32::try_from(wait.as_millis()).unwrap();
        // SAFETY: an open process handle.
        unsafe { WaitForSingleObject(self.0.as_raw_handle(), milliseconds) == WAIT_OBJECT_0 }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        // SAFETY: an open process handle; for a process that ended already the call only fails.
        unsafe { TerminateProcess(self.0.as_raw_handle(), 1) };
        self.ended(WAIT);
    }
}

/// The steps the child wrote so far.
fn steps_in(steps: &Path) -> String {
    std::fs::read_to_string(steps).unwrap_or_default()
}

/// Runs [`console_signal_child`] with `first` (`Ctrl+C` or `Ctrl+Break`) as the first key it sends, on a pseudo
/// console of its own, closed once the child waits for that or ended: its exit code, its steps and what its console
/// showed.
fn run_child(first: &str) -> (u32, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let steps = dir.path().join("steps");
    let console = PseudoConsole::new();
    let process = spawn_child(&console, first, &steps);
    let deadline = Instant::now() + CHILD_WAIT;
    while !steps_in(&steps).contains(READY_TO_CLOSE) && !process.ended(Duration::from_millis(50)) {
        assert!(
            Instant::now() < deadline,
            "the child neither ended nor waited for the close"
        );
    }
    let shown = console.close();
    assert!(
        process.ended(CHILD_WAIT),
        "the console signal child did not end: {}",
        steps_in(&steps)
    );
    let mut code = 0;
    // SAFETY: an ended process's handle and a place for its exit code.
    let read = unsafe { GetExitCodeProcess(process.0.as_raw_handle(), &raw mut code) };
    assert_ne!(read, FALSE, "{}", std::io::Error::last_os_error());
    (code, steps_in(&steps), shown)
}

/// The first Ctrl+C posts a quit and a Ctrl+Break after it gets the default action; the console's close then posts a
/// quit and holds the process until the system ends it.
#[test]
fn ctrl_c_and_the_consoles_close_post_an_orderly_quit() {
    let (code, steps, shown) = run_child("Ctrl+C");
    for step in [
        "Ctrl+C posted a quit",
        "Ctrl+Break got the default action",
        "the close posted a quit",
    ] {
        assert!(steps.contains(step), "{step}: {steps}\nits console showed: {shown}");
    }
    // the system ended the closing console's process, past the handler's wait
    assert_eq!(
        code, STATUS_CONTROL_C_EXIT,
        "{code:#x}: {steps}\nits console showed: {shown}"
    );
}

/// A first Ctrl+Break quits as a first Ctrl+C does, and a Ctrl+C after it gets the default action.
#[test]
fn a_first_ctrl_break_posts_an_orderly_quit() {
    let (code, steps, shown) = run_child("Ctrl+Break");
    for step in ["Ctrl+Break posted a quit", "Ctrl+C got the default action"] {
        assert!(steps.contains(step), "{step}: {steps}\nits console showed: {shown}");
    }
    assert_eq!(code, 0, "{code:#x}: {steps}\nits console showed: {shown}");
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

/// Records that the child passed `step`, for the test to read after the console is gone.
fn record_step(step: &str) {
    let steps = std::env::var_os(STEPS).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(steps)
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{step}\n").as_bytes()).unwrap();
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
    record_step(&format!("{first_name} posted a quit"));

    send_to_console(then);
    assert_eq!(defaulted.recv_timeout(WAIT), Ok(then));
    assert!(tasks.try_recv().is_err(), "a second interrupt posted again");
    record_step(&format!("{then_name} got the default action"));
    if first == CTRL_BREAK_EVENT {
        return;
    }

    record_step(READY_TO_CLOSE);
    assert_quit_posted(&tasks);
    record_step("the close posted a quit");
    // the system ends this process from here, past its exit handlers
    ngterm_app::coverage::write_profile();
    std::thread::sleep(super::SHUTDOWN_GRACE * 2);
    panic!("the console's close did not end the process");
}
