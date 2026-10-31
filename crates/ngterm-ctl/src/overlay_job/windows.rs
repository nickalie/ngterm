//! The overlay job's program on Windows: its whole process tree in a Job object, in a console process group of its
//! own, under the console the helper shares with it.
//!
//! - The program is `%ComSpec% /D /S /C "<command>"` (`cmd.exe` by default, `ngterm_host::command_line`'s shell): the
//!   command is the line cmd parses, as `eval` makes it the line `sh` parses on Unix, and its exit status is the
//!   command's.
//! - It starts suspended, joins a Job object that kills every process in it once the helper's handle to it closes
//!   (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), and only then runs, so no descendant starts outside the job. The job is
//!   the Unix process group's counterpart: the kill (`TerminateJobObject`) and the wait for the tree after a cancel
//!   (its count of active processes) take every process in it, however deep.
//! - The grace stop is a console Ctrl+Break to the program's own process group (`CREATE_NEW_PROCESS_GROUP`,
//!   `GenerateConsoleCtrlEvent`): the Windows counterpart of SIGTERM, which a console program handles or dies of
//!   with `STATUS_CONTROL_C_EXIT`. A Ctrl+C cannot target a group (a new group has it disabled), so Ctrl+Break it is.
//! - The helper ignores Ctrl+C and Ctrl+Break, the terminal's keys being the program's, and takes the console's
//!   close, logoff and shutdown events (`SetConsoleCtrlHandler`) as a cancel, as SIGHUP and SIGTERM are on Unix.
//! - There is no terminal foreground to hand over: a console has no foreground process group and every process
//!   attached to it reads the same input, so the program gets the keys without one, and a console has no hangup to
//!   watch for either (its close is the Ctrl event above).
//! - Exit statuses map onto the Unix ones: `STATUS_CONTROL_C_EXIT` (a Ctrl event the program did not handle) reads
//!   as 130, 128 plus SIGINT, as a shell reports a Ctrl+C'd program, and the kill sets 137, 128 plus SIGKILL.

use std::ffi::OsString;
use std::io;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::os::windows::process::CommandExt as _;
use std::process::Child;
use std::sync::OnceLock;
use std::time::Duration;

use ngterm_core::overlay_capture::{self, OverlayLaunchContext};
use ngterm_host::ShellCommand;
use windows_sys::Win32::Foundation::{FALSE, HANDLE, INVALID_HANDLE_VALUE, TRUE};
use windows_sys::Win32::System::Console::{
    CTRL_BREAK_EVENT, CTRL_C_EVENT, GenerateConsoleCtrlEvent, SetConsoleCtrlHandler,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

use super::Canceler;

/// No terminal to hand over or watch on Windows (the module docs say why): the console is the helper's and the
/// program's alike.
#[derive(Debug, Clone, Copy)]
pub(super) struct Terminal;

/// The program's console process group, which its pid names.
pub(super) type Group = u32;

/// The status a Ctrl event's end reads as: 128 plus SIGINT (`overlay_capture`'s, which a local overlay reads too).
pub(super) const INTERRUPTED_STATUS: i32 = overlay_capture::INTERRUPTED_STATUS;
/// The status the kill sets: 128 plus SIGKILL.
pub(super) const KILLED_STATUS: i32 = 137;
/// How long the console's close waits for the cancel to report before the system ends the helper (it allows 5 s).
const CLOSE_WAIT_CAP: Duration = Duration::from_millis(4_500);

/// The helper's console.
pub(super) fn stdin_terminal() -> Terminal {
    Terminal
}

/// Never: nothing is handed over or watched on Windows.
pub(super) fn is_terminal(_terminal: Terminal) -> bool {
    false
}

/// Nothing to take back: the console has no foreground group.
pub(super) fn take_terminal_back(_terminal: Terminal) {}

/// Never: a console's close arrives as a Ctrl event ([`route_signals`]).
pub(super) fn hung_up(_terminal: Terminal) -> bool {
    false
}

/// The grace stop: a Ctrl+Break to the program's process group. A helper with no console sends nothing, and the
/// kill past the grace ends the program.
pub(super) fn stop(group: Group) {
    // SAFETY: plain integers; a group that is gone, or no console, fails the call, which is ignored.
    unsafe { GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, group) };
}

/// Starts the program suspended, in a process group of its own, puts it in a kill-on-close job, then lets it run.
pub(super) fn spawn(
    context: &OverlayLaunchContext,
    base: &[(OsString, OsString)],
    _terminal: Option<Terminal>,
) -> io::Result<Program> {
    let job = kill_on_close_job()?;
    let shell = ShellCommand::cmd(&context.command, std::env::var_os("ComSpec").as_deref());
    let mut command = std::process::Command::new(&shell.program);
    // `/S`: everything between the first and the last quote is the command line, taken as it is
    command.args(&shell.args);
    if let Some(tail) = &shell.raw_tail {
        command.raw_arg(tail);
    }
    command
        .env_clear()
        .envs(base.iter().map(|(name, value)| (name, value)))
        .envs(&context.environment)
        .current_dir(&context.cwd)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
    let mut child = command.spawn()?;
    let started = join(&job, &child).and_then(|()| resume(child.id()));
    if let Err(error) = started {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let group = child.id();
    Ok(Program { child, group, job })
}

/// A job that kills every process in it once its last handle closes.
fn kill_on_close_job() -> io::Result<OwnedHandle> {
    // SAFETY: null attributes and name make an unnamed job with the default security.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is the job handle just created, owned by nothing else.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let size = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).map_err(io::Error::other)?;
    // SAFETY: the job handle is open; `limits` is a live struct of the class and the size passed.
    let set = unsafe {
        SetInformationJobObject(
            handle(&job),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size,
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

fn handle(owned: &OwnedHandle) -> HANDLE {
    owned.as_raw_handle()
}

fn join(job: &OwnedHandle, child: &Child) -> io::Result<()> {
    // SAFETY: both handles are open for the call: the job's and the child's process handle.
    if unsafe { AssignProcessToJobObject(handle(job), child.as_raw_handle()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Resumes the threads of `pid`, a process created suspended: its one main thread. std keeps no thread handle, so
/// the thread is found by a snapshot of the system's threads.
fn resume(pid: u32) -> io::Result<()> {
    // SAFETY: a snapshot of every thread; `pid` is ignored for a thread snapshot.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is the snapshot just created, owned by nothing else.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = THREADENTRY32 {
        dwSize: u32::try_from(size_of::<THREADENTRY32>()).map_err(io::Error::other)?,
        ..THREADENTRY32::default()
    };
    let mut resumed = 0_usize;
    // SAFETY: the snapshot is open and `entry` a live struct whose `dwSize` is set.
    let mut more = unsafe { Thread32First(handle(&snapshot), &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid && resume_thread(entry.th32ThreadID) {
            resumed += 1;
        }
        // SAFETY: as above.
        more = unsafe { Thread32Next(handle(&snapshot), &raw mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other("the program's thread could not be resumed"));
    }
    Ok(())
}

fn resume_thread(id: u32) -> bool {
    // SAFETY: plain arguments; a thread that is gone fails the call.
    let raw = unsafe { OpenThread(THREAD_SUSPEND_RESUME, FALSE, id) };
    if raw.is_null() {
        return false;
    }
    // SAFETY: `raw` is the thread handle just opened, owned by nothing else.
    let thread = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: the thread handle is open with the access `ResumeThread` needs.
    unsafe { ResumeThread(handle(&thread)) != u32::MAX }
}

/// The running program: its child, its process group and the job its tree runs in. Dropping it closes the job,
/// which kills whatever of the tree is left.
#[derive(Debug)]
pub(super) struct Program {
    child: Child,
    group: Group,
    job: OwnedHandle,
}

impl Program {
    pub(super) fn group(&self) -> Group {
        self.group
    }

    /// The program's status once it ended, `None` while it runs.
    pub(super) fn reap(&mut self) -> Option<i32> {
        let code = self.child.try_wait().ok()??.code()?;
        Some(overlay_capture::status_of_exit_code(u32::from_ne_bytes(
            code.to_ne_bytes(),
        )))
    }

    /// Whether any process of the job is still there.
    pub(super) fn alive(&self) -> bool {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        let Ok(size) = u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>()) else {
            return false;
        };
        // SAFETY: the job handle is open; `accounting` is a live struct of the class and the size passed.
        let read = unsafe {
            QueryInformationJobObject(
                handle(&self.job),
                JobObjectBasicAccountingInformation,
                (&raw mut accounting).cast(),
                size,
                std::ptr::null_mut(),
            )
        };
        read != 0 && accounting.ActiveProcesses > 0
    }

    /// Ends every process of the job, past the grace.
    pub(super) fn kill(&self) {
        // SAFETY: the job handle is open; a job already empty is not an error.
        unsafe { TerminateJobObject(handle(&self.job), KILLED_STATUS.unsigned_abs()) };
    }
}

/// The run's canceler, for the console's Ctrl handler, which takes no context.
static CANCELER: OnceLock<Canceler> = OnceLock::new();

/// Ignores Ctrl+C and Ctrl+Break and turns the console's close, logoff and shutdown into a cancel.
pub(super) fn route_signals(canceler: Canceler) {
    if CANCELER.set(canceler).is_ok() {
        // SAFETY: `on_console_event` is a valid handler for the process's whole life.
        unsafe { SetConsoleCtrlHandler(Some(on_console_event), TRUE) };
    }
}

/// The console's Ctrl handler, on a thread of the system's. The close, logoff and shutdown events end the helper
/// once the handler returns, so it cancels and gives the run its grace to report, within the system's own limit:
/// the run ending first ends the process, and with it this wait.
unsafe extern "system" fn on_console_event(event: u32) -> windows_sys::core::BOOL {
    if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
        return TRUE;
    }
    if let Some(canceler) = CANCELER.get() {
        canceler.cancel();
        let wait = (canceler.0.grace + Duration::from_millis(500)).min(CLOSE_WAIT_CAP);
        std::thread::sleep(wait);
    }
    TRUE
}
