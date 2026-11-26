//! The Windows child and its job on real pseudoconsoles: a handed-over child killed through its process handle, a
//! retired killer, handles without the rights a call needs, a job holding more processes than a first read of its
//! list has room for, the spawn's and the master's refusals, each shell's tree in its job, the suspended start's
//! failures, panes of one package that end alone, and the processes that left a job taken back into it (or, in
//! another job, ended with its kill), but not a process that got a listed one's id. An actor reads each child's
//! output away, so the console never waits on its pipe.

use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use portable_pty::ChildKiller;
use windows_sys::Win32::Foundation::{DuplicateHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE};
use windows_sys::Win32::System::JobObjects::IsProcessInJob;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

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
    run_command(&command(program, args))
}

/// `command` on a pseudoconsole, its output read away.
fn run_command(command: &Command) -> Running {
    let Spawned { master, child } = crate::spawn(command, Size::cells(24, 80)).expect("a child on a pseudoconsole");
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

/// A process killed while it is still creating its own child takes that child with it (`STATUS_PROCESS_IS_TERMINATING`,
/// with or without a job or a console), so the kill waits for a grandchild that has run: a job listing the inner
/// cmd's ping shows the inner cmd got far enough to start it.
#[test]
fn a_handed_over_childs_killer_clone_kills_it() {
    let running = run("cmd.exe", &["/d", "/c", "cmd /d /c ping -n 300 127.0.0.1 >nul"]);
    let mut handed = handed_over(&running.child, None, None);
    let view = running.child.job_view();
    wait_until("the inner cmd started its ping", || {
        view.process_ids().is_some_and(|pids| pids.len() >= 3)
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

/// The job, not a process id, names the tree, so a retired killer still ends it.
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
    let children = FIRST_CAPACITY + 8;
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

/// A program that is not there is refused in plain words: the program as written, then the OS error (no debug text:
/// doubled backslashes, NUL terminators, `Some(...)`).
#[test]
fn a_missing_program_is_refused_naming_it_as_written() {
    let program = r"C:\ngterm-no-such-dir\no-such-program.exe";
    let err = crate::spawn(&command(program, &["-l"]), Size::cells(24, 80)).expect_err("a program that is not there");
    let shown = err.to_string();
    assert_eq!(err.kind(), io::ErrorKind::NotFound, "{shown}");
    assert!(
        shown.starts_with(&format!("{program}: ")) && shown.ends_with("(os error 3)") && shown.lines().count() == 1,
        "{shown}"
    );
    for debug_text in [r"\\", r"\0", "Some(", "CreateProcessW"] {
        assert!(!shown.contains(debug_text), "{debug_text} in {shown}");
    }
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

/// The exit code of the process `process` names, once it exits; `None` if it still runs `PATIENCE` from now.
fn exit_code_of(process: &OwnedHandle) -> Option<u32> {
    let patience = u32::try_from(PATIENCE.as_millis()).expect("a timeout");
    // SAFETY: waits on a process handle this test holds open with `SYNCHRONIZE`.
    if unsafe { WaitForSingleObject(process.as_raw_handle(), patience) } != WAIT_OBJECT_0 {
        return None;
    }
    let mut code = 0_u32;
    // SAFETY: reads the exit code of a process this test holds open into a live local.
    (unsafe { GetExitCodeProcess(process.as_raw_handle(), &raw mut code) } != 0).then_some(code)
}

/// `STATUS_PROCESS_IS_TERMINATING`: under load, Windows 11 (build 26100) sometimes gives this exit code to an escaped
/// ping that the end of its tree terminates, before this change too. The cause is not known
/// (`docs/backlog/an-escaped-ping-ends-with-process-is-terminating.md`).
const PROCESS_IS_TERMINATING: u32 = 0xC000_010A;

/// That `process` ended with `code` (the code its tree's end gives), or with [`PROCESS_IS_TERMINATING`]. The test's
/// pings run for 300 seconds, so one that ends in the wait was ended.
#[track_caller]
fn assert_ended_by_its_job(process: &OwnedHandle, code: u32, what: &str) {
    let ended = exit_code_of(process);
    assert!(
        ended.is_some_and(|ended| ended == code || ended == PROCESS_IS_TERMINATING),
        "{what}: its exit code is {ended:x?}"
    );
}

/// Whether the process `process` names still runs.
fn runs(process: BorrowedHandle<'_>) -> bool {
    // SAFETY: polls a process handle this test holds open with `SYNCHRONIZE`.
    unsafe { WaitForSingleObject(process.as_raw_handle(), 0) == WAIT_TIMEOUT }
}

/// Process `pid` with the rights to wait for it and read its state; `None` once it is gone.
fn watched(pid: u32) -> Option<OwnedHandle> {
    // SAFETY: opens a process by id; no memory is passed.
    let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    // SAFETY: a fresh handle nothing else owns.
    (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// The process running `program` (a file name) that the system lists as started by `pid`, once there is one (a
/// console program's first child can be its console host).
fn child_of(pid: u32, program: &str) -> u32 {
    let mut found = None;
    wait_until("the process started its child", || {
        found = escapees::processes()
            .iter()
            .filter(|entry| entry.parent == pid)
            .find(|entry| watched(entry.pid).is_some_and(|process| image_name(&process).eq_ignore_ascii_case(program)))
            .map(|entry| entry.pid);
        found.is_some()
    });
    found.expect("a child")
}

/// A child joins its job before it runs: it is in the job, and so is what it starts, a level down too. The job's end
/// ends them.
#[test]
fn a_child_and_what_it_starts_are_in_its_job() {
    let Running { mut child, _actor } = run("cmd.exe", &["/d", "/c", "cmd /d /c ping -n 300 127.0.0.1 >nul"]);
    let view = child.job_view();
    wait_until("the inner cmd started its ping in the job", || {
        view.process_ids().is_some_and(|pids| pids.len() >= 3)
    });
    assert!(view.process_ids().expect("the job's processes").contains(&child.pid()));
    assert!(!packaged(child.process().expect("the process handle")));
    child.killer().kill_tree().expect("the kill");
    assert_eq!(child.wait().expect("the exit").exit_code(), 1);
    wait_until("the tree ended", || {
        view.process_ids().is_some_and(|pids| pids.is_empty())
    });
}

/// A child started suspended runs on its own pseudoconsole: what it prints reaches the master. The line is not
/// the one the command names, which the console's title would carry too.
#[test]
fn a_child_prints_to_its_console() {
    let Spawned { master, mut child } = crate::spawn(
        &command("cmd.exe", &["/d", "/c", "echo ngterm-%OS%"]),
        Size::cells(24, 80),
    )
    .expect("a child on a pseudoconsole");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let on_read = {
        let seen = Arc::clone(&seen);
        Box::new(move |bytes: &[u8]| {
            seen.lock().expect("the output").extend_from_slice(bytes);
            if bytes.windows(4).any(|window| window == b"\x1b[6n") {
                vec![b"\x1b[1;1R".to_vec()]
            } else {
                Vec::new()
            }
        })
    };
    let actor = Actor::spawn(ActorConfig::new("test", master, on_read)).expect("an actor");
    assert!(child.wait().expect("the exit").success());
    wait_until("the console printed the line", || {
        String::from_utf8_lossy(&seen.lock().expect("the output")).contains("ngterm-Windows_NT")
    });
    actor.child_exited();
}

/// A program Windows denies (a directory) is refused in plain words.
#[test]
fn a_program_windows_denies_is_refused_naming_it() {
    let program = std::env::var("SystemRoot").expect("the Windows directory");
    let err = crate::spawn(&command(&program, &[]), Size::cells(24, 80)).expect_err("a directory started");
    let shown = err.to_string();
    assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{shown}");
    assert!(
        shown.starts_with(&format!("{program}: ")) && shown.ends_with("(os error 5)"),
        "{shown}"
    );
}

/// Whether `shell` is there to test. A shell that is not there fails the test on CI, whose Windows runners have each
/// shell [`a_ping_that_a_shells_child_starts_is_in_the_job`] names; elsewhere (a machine without that installer's
/// shell, or with it in another folder) its case is left out, and the test says so.
fn installed(shell: &str) -> bool {
    let there = std::path::Path::new(shell).is_file();
    assert!(
        there || std::env::var_os("CI").is_none(),
        "{shell} is not on the CI runner"
    );
    if !there {
        eprintln!("{shell} is not installed: its case is left out");
    }
    there
}

/// Each shell a pane runs (`cmd.exe`, Windows PowerShell, PowerShell 7 from its installer, Git Bash) is in its job
/// before it runs, and so is the ping its child starts: the kill of the tree ends that ping. CI's Windows runners
/// have each of them; elsewhere a shell not in its installer's default folder is left out ([`installed`]).
#[test]
fn a_ping_that_a_shells_child_starts_is_in_the_job() {
    let program_files = std::env::var("ProgramFiles").expect("the Program Files folder");
    let pwsh = format!(r"{program_files}\PowerShell\7\pwsh.exe");
    let bash = format!(r"{program_files}\Git\bin\bash.exe");
    let posh_line = "cmd /d /c ping -n 300 127.0.0.1";
    let shells: [(&str, &[&str]); 4] = [
        ("cmd.exe", &["/d", "/c", "cmd /d /c ping -n 300 127.0.0.1 >nul"]),
        ("powershell.exe", &["-NoLogo", "-NoProfile", "-Command", posh_line]),
        (&pwsh, &["-NoLogo", "-NoProfile", "-Command", posh_line]),
        (&bash, &["-c", "bash -c 'ping -n 300 127.0.0.1'; true"]),
    ];
    // `cmd.exe` and `powershell.exe` come with Windows, found on its PATH
    let found = |(shell, _): &(&str, &[&str])| !std::path::Path::new(shell).is_absolute() || installed(shell);
    for (shell, args) in shells.into_iter().filter(found) {
        let Running { child, _actor } = run(shell, args);
        assert!(
            !packaged(child.process().expect("the process handle")),
            "{shell} is packaged"
        );
        let view = child.job_view();
        let mut ping = None;
        wait_until("the shell's child started a ping in the job", || {
            ping = view
                .process_ids()
                .unwrap_or_default()
                .into_iter()
                .filter_map(watched)
                .find(|process| image_name(process).eq_ignore_ascii_case("ping.exe"));
            ping.is_some()
        });
        child.killer().kill_tree().expect("the kill");
        assert_eq!(
            exit_code_of(&ping.expect("the ping")),
            Some(1),
            "the ping of {shell} outlived the kill"
        );
    }
}

/// A job that stands for a package's, the job Windows puts each process of a packaged app into as `CreateProcessW`
/// creates it: no limits, and the test holds it.
fn package_job() -> OwnedHandle {
    // SAFETY: creates an unnamed job with default security; no memory is passed.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    assert!(!raw.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: a fresh handle nothing else owns.
    unsafe { OwnedHandle::from_raw_handle(raw) }
}

/// [`create`] as Windows does it for a packaged app: the child created is put into its package's job, `package`,
/// before the spawn can assign it to the pane's job.
fn create_in_package(package: &OwnedHandle) -> impl FnOnce(CommandBuilder, Size) -> io::Result<Created> + '_ {
    move |builder, size| {
        let created = create(builder, size)?;
        let process = created.1.as_raw_handle().expect("the child's process handle");
        // SAFETY: both handles are open; no memory is passed.
        let joined = unsafe { AssignProcessToJobObject(package.as_raw_handle(), process) };
        assert_ne!(joined, 0, "{}", io::Error::last_os_error());
        Ok(created)
    }
}

/// A pane of the package `package` (see [`create_in_package`]) whose shell runs until it is killed.
fn package_pane(package: &OwnedHandle) -> Running {
    let builder = command("cmd.exe", &["/d", "/c", "ping -n 300 127.0.0.1 >nul"]).builder();
    let Spawned { master, child } =
        spawn_with(builder, Size::cells(24, 80), create_in_package(package)).expect("a pane of the package");
    Running {
        child,
        _actor: drain(master),
    }
}

/// Whether `child`'s process is in `job`.
fn in_job_of(child: &Child, job: BorrowedHandle<'_>) -> bool {
    let mut inside = 0;
    // SAFETY: both handles are open; the result goes to a live local.
    let read = unsafe {
        IsProcessInJob(
            child.process().expect("the process handle").as_raw_handle(),
            job.as_raw_handle(),
            &raw mut inside,
        )
    };
    assert_ne!(read, 0, "{}", io::Error::last_os_error());
    inside != 0
}

/// Panes of one packaged app (issue 19, PowerShell from the Microsoft Store) are each in their package's job and their
/// own, but not in each other's: the package's job takes the first process before its pane's job does, so it nests
/// in no pane's job. The end of a pane, by its kill or its job's close, ends its own tree only, in any order.
#[test]
fn the_end_of_a_pane_leaves_the_panes_of_its_package_running() {
    let package = package_job();
    let panes: Vec<Running> = (0..3).map(|_| package_pane(&package)).collect();
    for (index, pane) in panes.iter().enumerate() {
        assert!(
            in_job_of(&pane.child, package.as_handle()),
            "pane {index} is not in its package's job"
        );
        for (other, other_pane) in panes.iter().enumerate().filter(|(other, _)| *other != index) {
            assert!(
                !in_job_of(&pane.child, other_pane.child.job()),
                "pane {index} is in the job of pane {other}"
            );
        }
    }
    let [first, second, third] = <[Running; 3]>::try_from(panes)
        .map_err(|_| "three panes")
        .expect("three panes");
    let shells: Vec<OwnedHandle> = [&first, &second, &third]
        .iter()
        .map(|pane| watched(pane.child.pid()).expect("a pane's shell"))
        .collect();
    first.child.killer().kill_tree().expect("the first pane's kill");
    assert_eq!(exit_code_of(&shells[0]), Some(1));
    assert!(runs(shells[1].as_handle()), "the second pane ended with the first");
    assert!(runs(shells[2].as_handle()), "the third pane ended with the first");
    drop(third);
    // a kill on close ends a process with the exit code 0
    assert_eq!(exit_code_of(&shells[2]), Some(0));
    assert!(runs(shells[1].as_handle()), "the second pane ended with the third");
    drop(first);
    second.child.killer().kill_tree().expect("the second pane's kill");
    assert_eq!(exit_code_of(&shells[1]), Some(1));
}

/// What a packaged shell leaves, made without a package: `line` run by a `cmd.exe` outside any job of the test's
/// own, and that `cmd.exe` then put alone into a new kill-on-close job once it started `program`, so what it started
/// is outside the job. The job is not watched ([`escapees::watch`]) unless a test makes it so.
struct Escaped {
    job: Arc<Job>,
    shell: std::process::Child,
}

impl Escaped {
    fn new(line: &str, program: &str) -> Self {
        Self::with(line, program, |_| {}, |handle| Arc::new(Job { handle }))
    }

    /// [`Escaped::new`] that calls `prepare` with the id of `program`'s process before the shell joins the job, which
    /// `hold` then makes the [`Job`] of.
    fn with(line: &str, program: &str, prepare: impl FnOnce(u32), hold: impl FnOnce(OwnedHandle) -> Arc<Job>) -> Self {
        let shell = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", line])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("a shell");
        // what the shell starts is in no job of the test's yet
        prepare(child_of(shell.id(), program));
        let job = kill_on_close_job().expect("a job");
        // SAFETY: both handles are open, the process one in `shell`.
        let joined = unsafe { AssignProcessToJobObject(job.as_raw_handle(), shell.as_raw_handle()) };
        assert_ne!(joined, 0, "{}", io::Error::last_os_error());
        Self { job: hold(job), shell }
    }

    fn killer(&self) -> Killer {
        Killer {
            job: Arc::clone(&self.job),
        }
    }

    fn holds(&self, pid: u32) -> bool {
        job_process_ids(&self.job.handle).is_some_and(|pids| pids.contains(&pid))
    }
}

impl Drop for Escaped {
    fn drop(&mut self) {
        drop(self.shell.kill());
    }
}

/// The kill of a tree takes back first what escaped its job, and ends it too: only what its processes started.
#[test]
fn killing_the_tree_ends_what_escaped_its_job() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    assert!(!escaped.holds(ping), "the ping started in the job");
    let ping = watched(ping).expect("the ping");
    let mut bystander = std::process::Command::new("ping")
        .args(["-n", "300", "127.0.0.1"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("a process outside the tree");
    escaped.killer().kill_tree().expect("the kill");
    assert_ended_by_its_job(&ping, 1, "the ping outlived the kill");
    assert!(
        bystander.try_wait().expect("the bystander's state").is_none(),
        "the kill ended another tree"
    );
    bystander.kill().expect("the bystander's end");
}

/// The close of a job takes back what escaped it first, so the close ends it with the tree.
#[test]
fn the_close_of_a_job_ends_what_escaped_it() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = watched(child_of(escaped.shell.id(), "ping.exe")).expect("the ping");
    drop(escaped);
    // a kill on close ends a process with the exit code 0
    assert_ended_by_its_job(&ping, 0, "the ping outlived its job");
}

/// A watched job (each pane's job is watched) takes back what escaped it while the tree runs: the job lists it then.
#[test]
fn a_watched_job_takes_back_what_escaped_it() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    escapees::watch(&escaped.job);
    wait_until("the job took the ping back", || escaped.holds(ping));
}

/// What an escaped process started before it was taken back escaped too, and is taken back with it.
#[test]
fn what_an_escaped_process_started_is_taken_back_too() {
    let escaped = Escaped::new("cmd /d /c ping -n 300 127.0.0.1 >nul", "cmd.exe");
    let inner = child_of(escaped.shell.id(), "cmd.exe");
    let ping = watched(child_of(inner, "ping.exe")).expect("the ping");
    escaped.killer().kill_tree().expect("the kill");
    assert_ended_by_its_job(&ping, 1, "the ping outlived the kill");
}

/// The id of the process `process` names.
fn pid_of(process: &OwnedHandle) -> u32 {
    // SAFETY: reads the id of a process this test holds open.
    unsafe { GetProcessId(process.as_raw_handle()) }
}

/// What escaped a job and is then put into another job, as Windows puts a packaged program a pane's process starts
/// into its package's: `line`'s `cmd.exe`, the inner one, in a job of its own, after it started a ping. The inner
/// shell joins that job before the outer one joins the pane's, which `hold` makes the [`Job`] of.
fn escaped_into_another_job(
    hold: impl FnOnce(OwnedHandle) -> Arc<Job>,
) -> (Escaped, OwnedHandle, OwnedHandle, OwnedHandle) {
    use windows_sys::Win32::System::Threading::{PROCESS_SET_QUOTA, PROCESS_TERMINATE};
    let package = package_job();
    let mut found = None;
    let prepare = |inner: u32| {
        let ping = watched(child_of(inner, "ping.exe")).expect("the ping");
        // SAFETY: opens a process by id; no memory is passed.
        let raw = unsafe {
            OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                inner,
            )
        };
        assert!(!raw.is_null(), "{}", io::Error::last_os_error());
        // SAFETY: a fresh handle nothing else owns.
        let inner = unsafe { OwnedHandle::from_raw_handle(raw) };
        // SAFETY: both handles are open, the process one with the rights an assignment needs.
        let joined = unsafe { AssignProcessToJobObject(package.as_raw_handle(), inner.as_raw_handle()) };
        assert_ne!(joined, 0, "{}", io::Error::last_os_error());
        found = Some((inner, ping));
    };
    let escaped = Escaped::with("cmd /d /c ping -n 300 127.0.0.1 >nul", "cmd.exe", prepare, hold);
    let (inner, ping) = found.expect("the inner shell and its ping");
    (escaped, package, inner, ping)
}

/// A process of the tree that cannot join its job (it is in another job, as a packaged program a pane's process
/// starts is in its package's) ends with the kill of the tree, and what it started is taken back and ends too.
#[test]
fn a_process_of_the_tree_in_another_job_ends_with_its_kill() {
    let (escaped, _package, inner, ping) = escaped_into_another_job(|handle| Arc::new(Job { handle }));
    let pid = pid_of(&inner);
    escapees::Tree::of(&escaped.job.handle).adopt(&escaped.job.handle, &escapees::processes());
    assert!(!escaped.holds(pid), "the inner shell joined the job");
    assert!(escaped.holds(pid_of(&ping)), "its ping was not taken back");
    escaped.killer().kill_tree().expect("the kill");
    assert_ended_by_its_job(&inner, 1, "the inner shell outlived the kill");
    assert_ended_by_its_job(&ping, 1, "its ping outlived the kill");
}

/// The close of a job takes back what a process of the tree in another job started, and ends it with the tree. That
/// process itself is left to its console, which a pane's close ends.
#[test]
fn the_close_of_a_job_ends_what_a_process_in_another_job_started() {
    let (escaped, _package, inner, ping) = escaped_into_another_job(|handle| Arc::new(Job { handle }));
    drop(escaped);
    // a kill on close ends a process with the exit code 0
    assert_ended_by_its_job(&ping, 0, "the ping outlived its job");
    // the inner shell ends with its ping, or ends here; one that ended already fails harmlessly
    // SAFETY: terminates a process this test holds open with `PROCESS_TERMINATE`.
    unsafe { TerminateProcess(inner.as_raw_handle(), 1) };
}

/// A pane's job is watched whatever its own program is (a `cmd.exe` pane, not a packaged one): what a process of its
/// tree in another job (a Store PowerShell typed in the pane) starts is taken back while that process runs. So it
/// ends with the pane also when that process ended first, and nothing names it as the tree's then.
#[test]
fn a_panes_job_takes_back_what_a_process_in_another_job_started_before_it_ends() {
    let (escaped, _package, inner, ping) = escaped_into_another_job(Job::holding);
    wait_until("the job took the ping back", || escaped.holds(pid_of(&ping)));
    // SAFETY: terminates a process this test holds open with `PROCESS_TERMINATE`.
    let ended = unsafe { TerminateProcess(inner.as_raw_handle(), 1) };
    assert_ne!(ended, 0, "{}", io::Error::last_os_error());
    assert_eq!(exit_code_of(&inner), Some(1));
    drop(escaped);
    // a kill on close ends a process with the exit code 0
    assert_ended_by_its_job(&ping, 0, "the ping outlived its job");
}

/// A process table read before a process ended can name its id as a child of the tree, after another process got
/// that id: that process is left alone, as its creator, read through its own handle, is not of the tree. It is
/// created after the tree's process, so the creation time alone does not tell.
#[test]
fn a_process_that_got_the_id_of_a_listed_child_is_not_taken() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = child_of(escaped.shell.id(), "ping.exe");
    let mut bystander = std::process::Command::new("ping")
        .args(["-n", "300", "127.0.0.1"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("a process outside the tree");
    let mut listed = escapees::processes();
    listed.push(escapees::Entry {
        pid: bystander.id(),
        parent: escaped.shell.id(),
    });
    escapees::Tree::of(&escaped.job.handle).adopt(&escaped.job.handle, &listed);
    assert!(escaped.holds(ping), "the shell's own ping stayed out");
    assert!(!escaped.holds(bystander.id()), "a process of another tree was taken");
    bystander.kill().expect("the bystander's end");
    bystander.wait().expect("the bystander's exit");
}

/// What a process of the tree starts outside the job as the kill ends it is taken back once the tree ended, and
/// ended too: the tree holds that process open, so its id still names it.
#[test]
fn what_a_process_started_as_its_tree_was_killed_is_taken_back() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = watched(child_of(escaped.shell.id(), "ping.exe")).expect("the ping");
    let shell = watched(escaped.shell.id()).expect("the shell");
    let job = &escaped.job.handle;
    // the ping, outside the job and not yet taken back, stands for one started after the first pass
    let mut tree = escapees::Tree::of(job);
    terminate(job).expect("the kill");
    tree.wait_ended(escapees::ENDING);
    assert_eq!(exit_code_of(&shell), Some(1));
    end_tree(job, &mut tree).expect("the second pass");
    assert_ended_by_its_job(&ping, 1, "the ping outlived its tree");
}

/// The error of an assignment of process `pid` to `job`; `None` when the job took it.
fn assignment_error(job: &OwnedHandle, pid: u32) -> Option<u32> {
    // SAFETY: opens a process by id; no memory is passed.
    let raw = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid) };
    assert!(!raw.is_null(), "{}", io::Error::last_os_error());
    // SAFETY: a fresh handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: both handles are open, the process one with the rights an assignment needs.
    let joined = unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) };
    if joined != 0 {
        return None;
    }
    os_error(&io::Error::last_os_error())
}

/// What a process of the tree starts as the kill ends it also ends when the job refuses to take it back: a job that
/// ends its tree can refuse an assignment (`ERROR_ACCESS_DENIED`) after the tree ended. A job handle without the
/// right to assign gives the same refusal at each pass.
#[test]
fn what_a_process_started_as_its_tree_was_killed_ends_when_its_job_refuses_it() {
    let escaped = Escaped::new("ping -n 300 127.0.0.1 >nul", "ping.exe");
    let ping = watched(child_of(escaped.shell.id(), "ping.exe")).expect("the ping");
    let shell = watched(escaped.shell.id()).expect("the shell");
    let refusing = restricted(escaped.job.handle.as_handle(), JOB_QUERY | JOB_TERMINATE);
    assert_eq!(
        assignment_error(&refusing, pid_of(&ping)),
        Some(ERROR_ACCESS_DENIED),
        "the job took the ping"
    );
    let mut tree = escapees::Tree::of(&refusing);
    terminate(&refusing).expect("the kill");
    tree.wait_ended(escapees::ENDING);
    assert_eq!(exit_code_of(&shell), Some(1));
    end_tree(&refusing, &mut tree).expect("the second pass");
    assert_ended_by_its_job(&ping, 1, "the refused ping outlived its tree");
}

/// A child started suspended that cannot join its job (a job handle without the right to assign) is terminated
/// before it runs, and the start fails with the assignment's error.
#[test]
fn a_suspended_child_that_cannot_join_its_job_is_terminated() {
    let pair = ConPtySystem::default()
        .openpty(Size::cells(24, 80).pty())
        .expect("a pseudoconsole");
    drop(pair.slave);
    let master: Box<dyn MasterPty> = pair.master;
    let Ok(conpty) = master.downcast::<ConPtyMasterPty>() else {
        panic!("the master is not a ConPTY master");
    };
    let (child, thread) = conpty
        .spawn_command_suspended(command("cmd.exe", &["/d", "/c", "exit 0"]).builder())
        .expect("a suspended child");
    let process = watched(child.process_id().expect("the child's id")).expect("the child");
    let job = kill_on_close_job().expect("a job");
    let query_only = restricted(job.as_handle(), JOB_QUERY);
    let err = join_and_resume(child, &thread, &query_only).expect_err("joined without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    assert_eq!(
        exit_code_of(&process),
        Some(1),
        "the suspended child was not terminated"
    );
}

/// A child started suspended whose main thread cannot be resumed (a thread handle without the right to) is terminated,
/// and the start fails with the resume's error: no suspended process is left.
#[test]
fn a_suspended_child_that_cannot_resume_is_terminated() {
    let (conpty, child, thread) = create(
        command("cmd.exe", &["/d", "/c", "exit 0"]).builder(),
        Size::cells(24, 80),
    )
    .expect("a suspended child");
    let process = watched(child.process_id().expect("the child's id")).expect("the child");
    let job = kill_on_close_job().expect("a job");
    // `SYNCHRONIZE` (winnt.h) without `THREAD_SUSPEND_RESUME`
    let wait_only = restricted(thread.as_handle(), 0x0010_0000);
    let err = join_and_resume(child, &wait_only, &job).expect_err("resumed without the right");
    assert_eq!(os_error(&err), Some(ERROR_ACCESS_DENIED), "{err}");
    assert_eq!(
        exit_code_of(&process),
        Some(1),
        "the suspended child was not terminated"
    );
    drop((conpty, thread));
}

/// Held for the whole run of a test that starts Store PowerShell: libtest runs tests on many threads, and a test that
/// needs the package to start with it ([`store_powershell_panes_end_alone`]) must not see another test's Store
/// PowerShell. Poison-tolerant: a failed Store test does not fail the next.
fn store_test() -> std::sync::MutexGuard<'static, ()> {
    static STORE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    STORE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether `process` runs with a package's identity (an MSIX package, such as PowerShell from the Microsoft Store).
fn packaged(process: BorrowedHandle<'_>) -> bool {
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
    let mut length = 0_u32;
    // SAFETY: with a length of zero and no buffer the call only reports the name's length into a live local.
    let status = unsafe { GetPackageFamilyName(process.as_raw_handle(), &raw mut length, std::ptr::null_mut()) };
    status == ERROR_INSUFFICIENT_BUFFER
}

/// The Store PowerShell's app execution alias.
fn store_powershell() -> String {
    let local = std::env::var("LOCALAPPDATA").expect("the local app data folder");
    format!(r"{local}\Microsoft\WindowsApps\pwsh.exe")
}

/// A Store PowerShell pane that starts a detached ping and a ping on its console, and the two pings once its job
/// took both back.
fn store_powershell_pane() -> (Running, Vec<OwnedHandle>) {
    let line = "Start-Process -WindowStyle Hidden ping -ArgumentList '-n','300','127.0.0.1'; ping -n 300 127.0.0.1";
    let pane = run(&store_powershell(), &["-NoLogo", "-NoProfile", "-Command", line]);
    assert!(packaged(pane.child.process().expect("the process handle")));
    let pings = pings_of(&pane.child);
    (pane, pings)
}

/// The packaged PowerShell processes that run now.
fn store_powershells() -> Vec<u32> {
    escapees::processes()
        .into_iter()
        .filter(|entry| {
            watched(entry.pid).is_some_and(|process| {
                image_name(&process).eq_ignore_ascii_case("pwsh.exe") && packaged(process.as_handle())
            })
        })
        .map(|entry| entry.pid)
        .collect()
}

/// PowerShell from the Microsoft Store (a packaged app) as a pane's program, the issue's setup: two panes start, the
/// second while the first runs. What either starts leaves its job and is taken back: a ping on the console and a
/// detached one end with the pane, by its kill and by its job's close. Needs that PowerShell (`Microsoft.PowerShell`
/// from the Store or its msixbundle), which CI does not have.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn a_store_powershell_pane_starts_and_its_tree_ends_with_it() {
    let _store = store_test();
    let (first, first_pings) = store_powershell_pane();
    let (second, second_pings) = store_powershell_pane();
    second.child.killer().kill_tree().expect("the kill");
    for ping in second_pings {
        assert_eq!(exit_code_of(&ping), Some(1), "a ping outlived the second pane's kill");
    }
    drop(first);
    for ping in first_pings {
        // a kill on close ends a process with the exit code 0
        assert_eq!(exit_code_of(&ping), Some(0), "a ping outlived the first pane's close");
    }
}

/// Store PowerShell panes, and one started outside ngterm (`outside_first`: before the panes, else after them):
/// the end of a pane, by its kill or its job's close, leaves each other pane, what it started, and the outside one
/// running. Windows puts each process of the package into its package's job as it creates it: started in a job list,
/// the first one would put that job inside its pane's job, and the pane's end would end every later one.
fn store_powershell_panes_end_alone(outside_first: bool) {
    assert_eq!(store_powershells(), [], "a Store PowerShell runs before the test");
    let start_outside = || {
        std::process::Command::new(store_powershell())
            .args(["-NoLogo", "-NoProfile", "-Command", "Start-Sleep 300"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("a Store PowerShell outside ngterm")
    };
    let mut early = outside_first.then(start_outside);
    let (first, _) = store_powershell_pane();
    let (second, second_pings) = store_powershell_pane();
    let (third, _) = store_powershell_pane();
    let mut outside = early.take().unwrap_or_else(start_outside);
    let shells: Vec<OwnedHandle> = [&first, &second, &third]
        .iter()
        .map(|pane| watched(pane.child.pid()).expect("a pane's shell"))
        .collect();
    first.child.killer().kill_tree().expect("the first pane's kill");
    assert_eq!(exit_code_of(&shells[0]), Some(1));
    // what the kill would end with it ends in milliseconds
    std::thread::sleep(Duration::from_secs(1));
    assert!(runs(shells[1].as_handle()), "the second pane ended with the first");
    assert!(runs(shells[2].as_handle()), "the third pane ended with the first");
    for ping in &second_pings {
        assert!(
            runs(ping.as_handle()),
            "a ping of the second pane ended with the first pane"
        );
    }
    assert!(
        outside.try_wait().expect("the outside one's state").is_none(),
        "the outside one ended with the first pane"
    );
    drop(third);
    assert_eq!(exit_code_of(&shells[2]), Some(0));
    std::thread::sleep(Duration::from_secs(1));
    assert!(runs(shells[1].as_handle()), "the second pane ended with the third");
    assert!(
        outside.try_wait().expect("the outside one's state").is_none(),
        "the outside one ended with the third pane"
    );
    drop((first, second));
    assert_eq!(exit_code_of(&shells[1]), Some(0));
    for ping in &second_pings {
        assert_eq!(exit_code_of(ping), Some(0), "a ping outlived the second pane's close");
    }
    assert!(
        outside.try_wait().expect("the outside one's state").is_none(),
        "the outside one ended with the panes"
    );
    outside.kill().expect("the outside one's end");
    outside.wait().expect("the outside one's exit");
}

/// [`store_powershell_panes_end_alone`] when the panes start the package (issue 19's QA, Q1): no Store PowerShell
/// runs before the first pane.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn store_powershell_panes_that_start_the_package_end_alone() {
    let _store = store_test();
    store_powershell_panes_end_alone(false);
}

/// [`store_powershell_panes_end_alone`] when a Store PowerShell outside ngterm started the package first.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn store_powershell_panes_after_one_outside_end_alone() {
    let _store = store_test();
    store_powershell_panes_end_alone(true);
}

/// A Store PowerShell that a pane's program starts (`pwsh` typed in a `cmd.exe` pane) while no other one runs, so it
/// starts the package: Windows puts it into its package's job, and it cannot join the pane's. A Store PowerShell pane
/// opened after it runs on when that pane ends. The pane's end, by its kill (`kill`) or its close, ends that
/// PowerShell (the close through its console) and the ping it started through `Start-Process`.
fn a_pane_whose_program_started_the_package_ends(kill: bool) {
    let _store = store_test();
    assert_eq!(store_powershells(), [], "a Store PowerShell runs before the test");
    let script = std::env::temp_dir().join(format!("ngterm-pty-store-{}.ps1", std::process::id()));
    std::fs::write(
        &script,
        "Start-Process -WindowStyle Hidden ping -ArgumentList '-n','300','127.0.0.1'; Start-Sleep 300",
    )
    .expect("the script");
    // `/s`: cmd.exe takes off the outer quotes and keeps the inner ones, so a path with a space stays one argument
    let line = format!(
        r#"""{}" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "{}"""#,
        store_powershell(),
        script.display()
    );
    let first = run_command(&command("cmd.exe", &["/d", "/s", "/c"]).raw_arg(line));
    let mut started = Vec::new();
    wait_until("the cmd pane's program started a Store PowerShell", || {
        started = store_powershells();
        !started.is_empty()
    });
    let pid = started[0];
    let started = watched(pid).expect("the cmd pane's PowerShell");
    let ping = watched(child_of(pid, "ping.exe")).expect("its ping");
    let mut inside = 0;
    // SAFETY: both handles are open; the result goes to a live local.
    let read = unsafe {
        IsProcessInJob(
            started.as_raw_handle(),
            first.child.job().as_raw_handle(),
            &raw mut inside,
        )
    };
    assert_ne!(read, 0, "{}", io::Error::last_os_error());
    assert_eq!(inside, 0, "its PowerShell is in the cmd pane's job");
    let (second, second_pings) = store_powershell_pane();
    let shell = watched(second.child.pid()).expect("the second pane's shell");
    if kill {
        first.child.killer().kill_tree().expect("the first pane's kill");
        assert_eq!(
            exit_code_of(&started),
            Some(1),
            "its PowerShell outlived the pane's kill"
        );
        assert_eq!(exit_code_of(&ping), Some(1), "its ping outlived the pane's kill");
    } else {
        drop(first);
        assert!(
            exit_code_of(&started).is_some(),
            "its PowerShell outlived the pane's close"
        );
        assert_eq!(exit_code_of(&ping), Some(0), "its ping outlived the pane's close");
    }
    // what the end would end with it ends in milliseconds
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        runs(shell.as_handle()),
        "the Store PowerShell pane ended with the cmd pane"
    );
    for ping in &second_pings {
        assert!(
            runs(ping.as_handle()),
            "a ping of the Store PowerShell pane ended with the cmd pane"
        );
    }
    drop(second);
    assert_eq!(exit_code_of(&shell), Some(0));
    drop(std::fs::remove_file(script));
}

/// [`a_pane_whose_program_started_the_package_ends`] by its kill.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn a_pane_whose_program_started_the_package_ends_its_tree_alone_by_its_kill() {
    a_pane_whose_program_started_the_package_ends(true);
}

/// [`a_pane_whose_program_started_the_package_ends`] by its close.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn a_pane_whose_program_started_the_package_ends_its_tree_alone_by_its_close() {
    a_pane_whose_program_started_the_package_ends(false);
}

/// A Store PowerShell that a `cmd.exe` pane's program starts, which starts a detached ping and ends before the pane:
/// the pane's job took the ping back while that PowerShell ran, so the pane's close ends it. Once the PowerShell
/// ended, nothing names the ping as the tree's.
#[test]
#[ignore = "needs PowerShell from the Microsoft Store"]
fn a_pane_ends_what_its_store_powershell_started_after_that_powershell_ended() {
    let _store = store_test();
    assert_eq!(store_powershells(), [], "a Store PowerShell runs before the test");
    let script = std::env::temp_dir().join(format!("ngterm-pty-store-ended-{}.ps1", std::process::id()));
    std::fs::write(
        &script,
        "Start-Process -WindowStyle Hidden ping -ArgumentList '-n','300','127.0.0.1'; Start-Sleep 2",
    )
    .expect("the script");
    // `/s`: cmd.exe takes off the outer quotes and keeps the inner ones; its own ping keeps the pane open
    let line = format!(
        r#"""{}" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "{}" & ping -n 300 127.0.0.1 >nul""#,
        store_powershell(),
        script.display()
    );
    let pane = run_command(&command("cmd.exe", &["/d", "/s", "/c"]).raw_arg(line));
    let mut started = Vec::new();
    wait_until("the cmd pane's program started a Store PowerShell", || {
        started = store_powershells();
        !started.is_empty()
    });
    let powershell = watched(started[0]).expect("the cmd pane's PowerShell");
    let ping = watched(child_of(started[0], "ping.exe")).expect("its ping");
    assert!(exit_code_of(&powershell).is_some(), "its PowerShell did not end");
    assert!(runs(ping.as_handle()), "its ping ended with its PowerShell");
    drop(pane);
    assert_eq!(exit_code_of(&ping), Some(0), "its ping outlived the pane's close");
    drop(std::fs::remove_file(script));
}

/// The two pings a Store PowerShell pane's line starts, once its job took both back.
fn pings_of(child: &Child) -> Vec<OwnedHandle> {
    let view = child.job_view();
    let mut pings = Vec::new();
    wait_until("the job took both pings back", || {
        pings = view
            .process_ids()
            .unwrap_or_default()
            .into_iter()
            .filter_map(watched)
            .filter(|process| image_name(process).eq_ignore_ascii_case("ping.exe"))
            .collect();
        pings.len() == 2
    });
    pings
}

/// The file name of the program `process` runs.
fn image_name(process: &OwnedHandle) -> String {
    use windows_sys::Win32::System::Threading::{PROCESS_NAME_WIN32, QueryFullProcessImageNameW};
    let mut path = [0_u16; 1024];
    let mut length = u32::try_from(path.len()).expect("a length");
    // SAFETY: `path` is writable for `length` units; the handle is open with `PROCESS_QUERY_LIMITED_INFORMATION`.
    let read = unsafe {
        QueryFullProcessImageNameW(
            process.as_raw_handle(),
            PROCESS_NAME_WIN32,
            path.as_mut_ptr(),
            &raw mut length,
        )
    };
    if read == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&path[..usize::try_from(length).expect("a length")]);
    path.rsplit('\\').next().unwrap_or_default().to_owned()
}
