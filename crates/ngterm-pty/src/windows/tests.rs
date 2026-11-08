//! The Windows child and its job on real pseudoconsoles: a handed-over child killed through its process handle, a
//! retired killer, handles without the rights a call needs, a job holding more processes than a first read of its
//! list has room for, and the spawn's and the master's refusals. An actor reads each child's output away, so the
//! console never waits on its pipe.

use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use portable_pty::ChildKiller;
use windows_sys::Win32::Foundation::{DuplicateHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, PROCESS_QUERY_LIMITED_INFORMATION};

use super::*;
use crate::{Actor, ActorConfig, ActorHandle, Command};

/// How long a test waits for a child (a CI runner can be slow to start one).
const PATIENCE: Duration = Duration::from_secs(60);
/// The job access right that reads its information (`JOB_OBJECT_QUERY`, winnt.h).
const JOB_QUERY: u32 = 0x0004;
/// The job access right that terminates its processes (`JOB_OBJECT_TERMINATE`, winnt.h).
const JOB_TERMINATE: u32 = 0x0008;

/// `program` with `args`, without the live ngterm's variables.
fn command(program: &str, args: &[&str]) -> Command {
    std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            key.to_str()
                .is_some_and(|key| key.starts_with("NGTERM_") || key.starts_with("AGTERM_"))
        })
        .fold(Command::new(program).args(args), Command::env_remove)
}

/// An actor that reads the console's output away, answering the cursor query it starts with.
fn drain(master: Master) -> ActorHandle {
    let on_read = Box::new(|bytes: &[u8]| {
        if bytes.windows(4).any(|window| window == b"\x1b[6n") {
            vec![b"\x1b[1;1R".to_vec()]
        } else {
            Vec::new()
        }
    });
    Actor::spawn(ActorConfig::new("test", master, on_read)).expect("an actor")
}

/// A child on a pseudoconsole and the actor reading its output; dropped, its job's close ends its tree.
struct Running {
    child: Child,
    _actor: ActorHandle,
}

fn run(program: &str, args: &[&str]) -> Running {
    let Spawned { master, child } =
        crate::spawn(&command(program, args), Size::cells(24, 80)).expect("a child on a pseudoconsole");
    Running {
        child,
        _actor: drain(master),
    }
}

/// A child that runs until it is killed.
fn lasting() -> Running {
    run("cmd.exe", &["/d", "/c", "ping -n 300 127.0.0.1 >nul"])
}

/// A copy of `handle` with only the `access` rights.
fn restricted(handle: BorrowedHandle<'_>, access: u32) -> OwnedHandle {
    // SAFETY: returns this process's pseudo-handle; no memory is passed.
    let this = unsafe { GetCurrentProcess() };
    let mut copy = std::ptr::null_mut();
    // SAFETY: duplicates a handle this process holds open into this process, written to a live local.
    let copied = unsafe { DuplicateHandle(this, handle.as_raw_handle(), this, &raw mut copy, access, 0, 0) };
    assert_ne!(copied, 0, "{}", io::Error::last_os_error());
    // SAFETY: a fresh handle nothing else owns.
    unsafe { OwnedHandle::from_raw_handle(copy) }
}

/// `child` as another process takes it over: copies of its process handle and its job with every right, or with
/// only the rights given.
fn handed_over(child: &Child, process_access: Option<u32>, job_access: Option<u32>) -> Child {
    let process = child.process().expect("the child's process handle");
    let process = match process_access {
        Some(access) => restricted(process, access),
        None => process.try_clone_to_owned().expect("a copy of the process handle"),
    };
    let job = match job_access {
        Some(access) => restricted(child.job(), access),
        None => child.job().try_clone_to_owned().expect("a copy of the job"),
    };
    Child::from_handoff(process, job).expect("the handed-over child")
}

fn os_error(err: &io::Error) -> Option<u32> {
    err.raw_os_error().and_then(|code| u32::try_from(code).ok())
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_masters_debug_shows_its_pipes() {
    let Spawned { master, mut child } = crate::spawn(&command("cmd.exe", &["/d", "/c", "exit 0"]), Size::cells(24, 80))
        .expect("a child on a pseudoconsole");
    let shown = format!("{master:?}");
    assert!(
        shown.starts_with("Master { input: ") && shown.contains(", output: ") && shown.ends_with(", .. }"),
        "{shown}"
    );
    let actor = drain(master);
    assert!(child.wait().expect("the child's exit").success());
    actor.child_exited();
}

/// A handed-over child is killed through its process handle (exit code 1), and its id read through it; killing it
/// again, gone, fails.
#[test]
fn a_handed_over_child_is_killed_through_its_process_handle() {
    let running = lasting();
    let mut handed = handed_over(&running.child, None, None);
    assert_eq!(handed.inner.process_id(), Some(running.child.pid()));
    handed.inner.kill().expect("the kill");
    assert_eq!(handed.wait().expect("the exit").exit_code(), 1);
    assert_eq!(
        handed.try_wait().expect("the status").map(|status| status.exit_code()),
        Some(1)
    );
    let again = handed.inner.kill().expect_err("killed twice");
    assert_eq!(os_error(&again), Some(ERROR_ACCESS_DENIED), "{again}");
}

#[test]
fn a_handed_over_childs_killer_clone_kills_it() {
    let running = lasting();
    let mut handed = handed_over(&running.child, None, None);
    let view = running.child.job_view();
    wait_until("cmd started its ping", || {
        view.process_ids().is_some_and(|pids| pids.len() >= 2)
    });
    let mut killer = handed.inner.clone_killer();
    killer.kill().expect("the kill");
    assert_eq!(handed.wait().expect("the exit").exit_code(), 1);
    wait_until("the child left its job", || {
        view.process_ids()
            .is_some_and(|pids| !pids.contains(&running.child.pid()))
    });
    assert!(
        !view.process_ids().expect("the job's processes").is_empty(),
        "the child's kill ended its tree"
    );
}

/// A handed-over child hands its process handle on again: the next owner's copy names the same process.
#[test]
fn a_handed_over_child_hands_its_process_handle_on() {
    let running = lasting();
    let handed = handed_over(&running.child, None, None);
    let again = handed_over(&handed, None, None);
    assert_eq!(again.pid(), running.child.pid());
}

#[test]
fn a_killer_without_a_handle_kills_nothing() {
    let refused = "no handle to kill the handed-over child through";
    let mut killer = NoKiller;
    assert_eq!(killer.kill().expect_err("killed").to_string(), refused);
    assert_eq!(killer.clone_killer().kill().expect_err("killed").to_string(), refused);
}

#[test]
fn a_handle_that_names_no_process_is_refused() {
    let job = kill_on_close_job().expect("a job");
    let not_a_process = job.try_clone().expect("a copy of the job");
    let err = Child::from_handoff(not_a_process, job).expect_err("a job taken for a process");
    assert_eq!(os_error(&err), Some(ERROR_INVALID_HANDLE), "{err}");
}

#[test]
fn a_process_handle_without_the_right_to_wait_fails_the_wait() {
    let running = lasting();
    let mut handed = handed_over(&running.child, Some(PROCESS_QUERY_LIMITED_INFORMATION), None);
    assert_eq!(handed.pid(), running.child.pid());
    let err = handed.try_wait().expect_err("polled without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    let err = handed.wait().expect_err("waited without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
}

/// A job handle that may only be queried lists the tree but cannot end it; one that may only terminate ends it but
/// lists nothing.
#[test]
fn a_job_handle_without_a_right_fails_what_needs_it() {
    let mut running = lasting();
    let pid = running.child.pid();
    let query_only = handed_over(&running.child, None, Some(JOB_QUERY));
    let err = query_only
        .killer()
        .kill_tree()
        .expect_err("terminated without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    let pids = query_only.job_view().process_ids().expect("the job's processes");
    assert!(pids.contains(&pid), "{pids:?}");
    assert!(
        running.child.try_wait().expect("the status").is_none(),
        "the child ended"
    );

    let terminate_only = handed_over(&running.child, None, Some(JOB_TERMINATE));
    assert_eq!(terminate_only.job_view().process_ids(), None);
    terminate_only.killer().hang_up().expect("the hangup");
    assert_eq!(running.child.wait().expect("the exit").exit_code(), 1);
}

/// Windows never reuses an id under a killer, so a retired one still ends the tree.
#[test]
fn a_retired_killer_still_ends_the_tree() {
    let mut running = lasting();
    let killer = running.child.killer();
    killer.retire();
    killer.kill_tree().expect("the kill");
    running.child.wait_exit().expect("the exit");
    assert_eq!(
        running
            .child
            .try_wait()
            .expect("the status")
            .map(|status| status.exit_code()),
        Some(1)
    );
}

#[test]
fn a_job_view_lists_more_processes_than_a_first_read_has_room_for() {
    let children = JobView::FIRST_CAPACITY + 8;
    let line =
        format!("(for /l %i in (1,1,{children}) do @start /b ping -n 300 127.0.0.1 >nul) & ping -n 300 127.0.0.1 >nul");
    let running = run("cmd.exe", &["/d", "/c", &line]);
    let view = running.child.job_view();
    // cmd, the pings it starts and the one it waits for
    let all = children + 2;
    wait_until("the job holds every ping", || {
        view.process_ids().is_some_and(|pids| pids.len() >= all)
    });
    let mut pids = view.process_ids().expect("the job's processes");
    assert!(pids.contains(&running.child.pid()), "{pids:?}");
    pids.sort_unstable();
    pids.dedup();
    assert!(pids.len() >= all, "listed twice: {pids:?}");
}

/// A pseudoconsole of no size is refused with portable-pty's error (`E_INVALIDARG`).
#[test]
fn a_console_of_no_size_is_refused() {
    let no_size = crate::spawn(&command("cmd.exe", &["/d", "/c", "exit 0"]), Size::cells(0, 0));
    let err = no_size.expect_err("a pseudoconsole of no size");
    assert_eq!(err.to_string(), "failed to create psuedo console: HRESULT -2147024809");
}

/// A program that is not there is refused with portable-pty's error, the OS error last.
#[test]
fn a_missing_program_is_refused() {
    let program = r"C:\ngterm-no-such-dir\no-such-program.exe";
    let err = crate::spawn(&command(program, &[]), Size::cells(24, 80)).expect_err("a program that is not there");
    let shown = err.to_string();
    assert!(
        shown.starts_with("CreateProcessW ")
            && shown.contains("no-such-program.exe")
            && shown.ends_with("(os error 3)"),
        "{shown}"
    );
}

/// A console whose pipes were taken already makes no master.
#[test]
fn a_console_without_its_pipes_makes_no_master() {
    let pair = ConPtySystem::default()
        .openpty(Size::cells(24, 80).pty())
        .expect("a pseudoconsole");
    drop(pair.slave);
    let master: Box<dyn MasterPty> = pair.master;
    let conpty = master
        .downcast::<ConPtyMasterPty>()
        .map_err(|_| "another kind of master")
        .expect("a ConPTY master");
    drop(conpty.take_pipes().expect("the pipes"));
    let err = Master::new(conpty).expect_err("a master without pipes");
    assert_eq!(err.to_string(), "writer already taken");
}

/// Dimensions past the signed COORD range are refused without wrapping the requested size.
#[test]
fn an_oversized_console_is_refused_with_the_requested_size() {
    for (rows, cols) in [(24, 32_768), (32_768, 80), (40_000, 40_000), (u16::MAX, u16::MAX)] {
        let err = ConPtySystem::default()
            .openpty(Size::cells(rows, cols).pty())
            .err()
            .expect("a pseudoconsole past i16");
        assert_eq!(
            err.to_string(),
            format!("pseudoconsole size {cols}x{rows} exceeds the maximum 32767x32767")
        );
    }
}
