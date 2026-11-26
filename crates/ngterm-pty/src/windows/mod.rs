//! The PTY on Windows: a pseudoconsole (`ConPTY`) through portable-pty, the child started suspended, assigned to a
//! job object that kills it and every process it starts once the job's last handle closes, and only then resumed. A
//! packaged app's process (PowerShell from the Microsoft Store) joins its package's job as it is created, so it is
//! never created in a pane's job ([`spawn_with`] says why). What a packaged program starts leaves every job, so
//! [`escapees`] takes it back into the child's.
//!
//! herdr: `src/pty/backend.rs` (`spawn_with_portable_pty`, its Windows form) and `src/platform/windows.rs`
//! (`StatusCommandGuard`'s kill-on-close job) at `HERDR_REV` (`build/pins.env`), licensed under the Apache License
//! 2.0, modified: the child is started suspended and assigned before it runs (`build/patches/portable-pty/0004`); the
//! pipes are taken as owned handles (`0005`), so the actor holds what a handoff passes on; a master is rebuilt from
//! handed-over handles.

mod actor;
mod escapees;

use std::fs::File;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::sync::{Arc, Weak};

use portable_pty::win::conpty::{ConPtyHandoff, ConPtyMasterPty, ConPtySystem};
use portable_pty::{CommandBuilder, ExitStatus, MasterPty, PtySystem};
use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, FALSE, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_PROCESS_ID_LIST,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
    QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessId, INFINITE, ResumeThread, TerminateProcess, WaitForSingleObject,
};

pub(crate) use actor::{Control, spawn_actor};

use crate::{Size, Spawned, pty_error};

/// A PTY's master side, for [`crate::Actor::spawn`]: the pseudoconsole and the two ends of its pipes this process
/// holds.
pub struct Master {
    conpty: Box<ConPtyMasterPty>,
    /// The write end of the console's input pipe.
    input: File,
    /// The read end of the console's output pipe.
    output: File,
}

impl std::fmt::Debug for Master {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Master")
            .field("input", &self.input)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

impl Master {
    /// A master over a session another process handed over (the session server's handoff), of the size the giver
    /// last set; it owns every handle of `handoff`.
    ///
    /// # Errors
    ///
    /// The pipes could not be taken from the rebuilt master.
    pub fn from_handoff(handoff: ConPtyHandoff, size: Size) -> io::Result<Self> {
        Self::new(Box::new(ConPtyMasterPty::from_handoff(handoff, size.pty())))
    }

    fn new(conpty: Box<ConPtyMasterPty>) -> io::Result<Self> {
        let (input, output) = conpty.take_pipes().map_err(|err| pty_error(&err))?;
        Ok(Self {
            conpty,
            input: File::from(input),
            output: File::from(output),
        })
    }
}

/// The child on a pseudoconsole, and the job that holds it and every process it starts.
#[derive(Debug)]
pub struct Child {
    inner: Box<dyn portable_pty::Child + Send + Sync>,
    pid: u32,
    /// Killed on close: the tree ends once this and every [`Killer`] are dropped.
    job: Arc<Job>,
}

/// A child's kill-on-close job. Its close first takes back what escaped it ([`escapees::Tree::adopt`]), so what
/// escaped ends with the tree. A process of the tree that cannot join it (one in another job) does not end with the
/// close: after a handoff, another process holds the job and the tree runs on. [`Killer::kill_tree`] ends it.
#[derive(Debug)]
struct Job {
    handle: OwnedHandle,
}

impl Drop for Job {
    fn drop(&mut self) {
        escapees::Tree::of(&self.handle).adopt(&self.handle, &escapees::processes());
    }
}

impl Job {
    /// The job of a child's tree, watched for what escapes it ([`escapees::watch`]).
    fn holding(handle: OwnedHandle) -> Arc<Self> {
        let job = Arc::new(Self { handle });
        escapees::watch(&job);
        job
    }
}

impl Child {
    /// The child's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Wait for the child to exit.
    ///
    /// # Errors
    ///
    /// Waiting for the process handle failed.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait()
    }

    /// Wait for the child to exit, as on Unix, where it is left unreaped so its group can still be signalled; here
    /// nothing is reaped, and its job ends the tree either way. Its status is [`Child::wait`]'s.
    ///
    /// # Errors
    ///
    /// As [`Child::wait`].
    pub fn wait_exit(&mut self) -> io::Result<()> {
        self.inner.wait().map(drop)
    }

    /// The child's exit status if it has exited, else `None`.
    ///
    /// # Errors
    ///
    /// As [`Child::wait`].
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    /// A handle that ends the child's tree from another thread, safe while this one waits.
    #[must_use]
    pub fn killer(&self) -> Killer {
        Killer {
            job: Arc::clone(&self.job),
        }
    }

    /// A view of the job the child's tree runs in that lists its processes and never keeps it open: the tree still
    /// ends when the child and its killers are gone.
    #[must_use]
    pub fn job_view(&self) -> JobView {
        JobView {
            job: Arc::downgrade(&self.job),
        }
    }

    /// The job object the child's tree runs in, borrowed (a handoff duplicates it into the new owner, whose copy
    /// then keeps the tree alive).
    #[must_use]
    pub fn job(&self) -> BorrowedHandle<'_> {
        self.job.handle.as_handle()
    }

    /// The child's process handle, borrowed (a handoff duplicates it into the new owner, which then waits for the
    /// child and reads its exit code through its copy).
    #[must_use]
    pub fn process(&self) -> Option<BorrowedHandle<'_>> {
        let raw = self.inner.as_raw_handle()?;
        // SAFETY: the handle is the child's process handle, which `inner` keeps open for as long as `self` is borrowed.
        Some(unsafe { BorrowedHandle::borrow_raw(raw) })
    }

    /// A child another process started and handed over (the session server's handoff): its process handle and the
    /// job its tree runs in, both owned from here on. It is waited for through the process handle, as a child
    /// started here is, and its tree ends with the job (kill on close: the tree outlives this one only while another
    /// process holds the job too).
    ///
    /// # Errors
    ///
    /// The process handle names no process (`GetProcessId` failed).
    pub fn from_handoff(process: OwnedHandle, job: OwnedHandle) -> io::Result<Self> {
        // SAFETY: reads the id of the process whose handle `process` owns.
        let pid = unsafe { GetProcessId(process.as_raw_handle()) };
        if pid == 0 {
            return Err(io::Error::last_os_error());
        }
        let job = Job::holding(job);
        Ok(Self {
            inner: Box::new(HandedProcess { process }),
            pid,
            job,
        })
    }
}

/// The portable-pty side of a handed-over child ([`Child::from_handoff`]), over its process handle.
#[derive(Debug)]
struct HandedProcess {
    process: OwnedHandle,
}

impl HandedProcess {
    fn exit_code(&self) -> io::Result<u32> {
        let mut code = 0_u32;
        // SAFETY: reads the exit code of the process whose handle `process` owns into a live local.
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &raw mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(code)
    }
}

impl portable_pty::ChildKiller for HandedProcess {
    fn kill(&mut self) -> io::Result<()> {
        // SAFETY: terminates the process whose handle `process` owns; one already gone fails harmlessly.
        if unsafe { TerminateProcess(self.process.as_raw_handle(), 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        match self.process.try_clone() {
            Ok(process) => Box::new(Self { process }),
            Err(err) => {
                tracing::debug!(%err, "duplicating a handed-over child's process handle failed");
                Box::new(NoKiller)
            }
        }
    }
}

impl portable_pty::Child for HandedProcess {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: polls the process handle `process` owns.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => self.exit_code().map(|code| Some(ExitStatus::with_exit_code(code))),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: waits on the process handle `process` owns.
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        self.exit_code().map(ExitStatus::with_exit_code)
    }

    fn process_id(&self) -> Option<u32> {
        // SAFETY: reads the id of the process whose handle `process` owns.
        let pid = unsafe { GetProcessId(self.process.as_raw_handle()) };
        (pid != 0).then_some(pid)
    }

    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        Some(self.process.as_raw_handle())
    }
}

/// A killer that kills nothing: what a handed-over child's killer clone is when its handle cannot be duplicated (the
/// child's tree still ends with its job).
#[derive(Debug)]
struct NoKiller;

impl portable_pty::ChildKiller for NoKiller {
    fn kill(&mut self) -> io::Result<()> {
        Err(io::Error::other("no handle to kill the handed-over child through"))
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Self)
    }
}

/// Ends a child's tree: [`Child::killer`]. Holds the child's job open, so the tree outlives a dropped [`Child`]
/// while a killer is left.
#[derive(Debug, Clone)]
pub struct Killer {
    job: Arc<Job>,
}

impl Killer {
    /// End the child's tree: Windows has no hangup to send, so this is [`Killer::kill_tree`].
    ///
    /// # Errors
    ///
    /// As [`Killer::kill_tree`].
    pub fn hang_up(&self) -> io::Result<()> {
        self.kill_tree()
    }

    /// Terminate every process of the child's job, the child too (exit code 1), after it takes back what escaped
    /// the job ([`escapees::Tree::adopt`]), and each process of the tree that could not join the job. Once the
    /// terminated tree ended (it waits a second at most), it takes back and terminates what a process of the tree
    /// started outside the job as it ended.
    ///
    /// # Errors
    ///
    /// `TerminateJobObject` failed.
    pub fn kill_tree(&self) -> io::Result<()> {
        let job = &self.job.handle;
        let mut tree = escapees::Tree::of(job);
        end_tree(job, &mut tree)?;
        tree.wait_ended(escapees::ENDING);
        end_tree(job, &mut tree)
    }

    /// Nothing on Windows: the job, not a process id, names the child's tree for as long as a handle holds it (Unix's
    /// [`Killer::retire`] stops signalling a group whose id may be reused). The ids [`Killer::kill_tree`] reads from
    /// the process table only find what escaped the job, and each is checked through a handle before it is taken.
    pub fn retire(&self) {}
}

/// One pass of [`Killer::kill_tree`]: take back what escaped `job` into it, terminate the job, then terminate each
/// process of `tree` that did not join it. A process in another job does not join, and a job that ends its tree can
/// refuse an assignment (`ERROR_ACCESS_DENIED`) after the tree ended.
fn end_tree(job: &OwnedHandle, tree: &mut escapees::Tree) -> io::Result<()> {
    tree.adopt(job, &escapees::processes());
    terminate(job)?;
    tree.terminate_outside();
    Ok(())
}

/// Terminate every process of `job` (exit code 1).
fn terminate(job: &OwnedHandle) -> io::Result<()> {
    // SAFETY: terminates the processes of an open job handle; no memory is passed.
    if unsafe { TerminateJobObject(job.as_raw_handle(), 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The processes of a child's job, read without holding the job open ([`Child::job_view`]): what stands in for a
/// pseudoconsole's processes, MSYS programs included, whose parent (an MSYS fork) is gone by the time they run.
#[derive(Debug, Clone)]
pub struct JobView {
    job: Weak<Job>,
}

impl JobView {
    /// The ids of the processes the job holds now (those of jobs nested in it included); `None` once the job closed
    /// (the tree ended) or cannot be queried.
    #[must_use]
    pub fn process_ids(&self) -> Option<Vec<u32>> {
        job_process_ids(&self.job.upgrade()?.handle)
    }
}

/// How many process ids a first read of a job's list makes room for.
const FIRST_CAPACITY: usize = 32;

/// The ids of the processes `job` holds now (those of jobs nested in it included); `None` when it cannot be queried.
fn job_process_ids(job: &OwnedHandle) -> Option<Vec<u32>> {
    let header = std::mem::offset_of!(JOBOBJECT_BASIC_PROCESS_ID_LIST, ProcessIdList) / size_of::<usize>();
    let mut capacity = FIRST_CAPACITY;
    // a process the tree starts between the reads may outgrow the room once more
    for _ in 0..3 {
        let mut buffer = vec![0_usize; header + capacity];
        let bytes = u32::try_from(buffer.len() * size_of::<usize>()).ok()?;
        // SAFETY: `buffer` is writable for `bytes` bytes and aligned for the list (its fields are at most
        // `usize`-aligned); the job handle is open while `job` is borrowed.
        let read = unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectBasicProcessIdList,
                buffer.as_mut_ptr().cast(),
                bytes,
                std::ptr::null_mut(),
            )
        };
        // SAFETY: as above; the error code is read right after the call.
        let more = read == 0 && unsafe { GetLastError() } == ERROR_MORE_DATA;
        if read == 0 && !more {
            return None;
        }
        // SAFETY: the buffer starts with the list's header, which the call filled.
        let list = unsafe { buffer.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>().read() };
        let assigned = usize::try_from(list.NumberOfAssignedProcesses).ok()?;
        let listed = usize::try_from(list.NumberOfProcessIdsInList).ok()?;
        if !more && listed >= assigned {
            return buffer[header..header + listed.min(capacity)]
                .iter()
                .map(|pid| u32::try_from(*pid).ok())
                .collect();
        }
        capacity = assigned.max(capacity) + FIRST_CAPACITY;
    }
    None
}

/// A job object whose last handle's close kills every process in it.
fn kill_on_close_job() -> io::Result<OwnedHandle> {
    // SAFETY: creates an unnamed job with default security; no memory is passed.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a fresh handle nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let len = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
        .map_err(|_| io::Error::other("the job limits do not fit a u32"))?;
    // SAFETY: passes a live local of the class's struct type and its exact size, for a job handle owned here.
    let set = unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            len,
        )
    };
    if set == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(job)
}

/// Lets the children this process starts take Ctrl+C. A process started in a process group of its own
/// (`CREATE_NEW_PROCESS_GROUP`: the session server's detached start, a CI runner's steps) ignores Ctrl+C, and a child
/// inherits that, so a pane's program would never see the Ctrl+C its console makes of a typed `^C`. The setting is
/// this process's own, and Ctrl+C reaches it only through a console it shares: none for the app and the detached
/// server, none of the pseudoconsoles.
fn let_children_take_ctrl_c() {
    // SAFETY: with no handler the call only sets whether this process ignores Ctrl+C.
    unsafe { SetConsoleCtrlHandler(None, FALSE) };
}

pub(crate) fn spawn(builder: CommandBuilder, size: Size) -> io::Result<Spawned> {
    spawn_with(builder, size, create)
}

/// A pseudoconsole, the child started suspended on it, and the child's main thread.
type Created = (
    Box<ConPtyMasterPty>,
    Box<dyn portable_pty::Child + Send + Sync>,
    OwnedHandle,
);

/// [`spawn`] through `create`, which starts the child suspended: the child joins a new kill-on-close job, and only
/// then runs. Never a job list (`PROC_THREAD_ATTRIBUTE_JOB_LIST`): when the first process of a packaged app (issue
/// 19: PowerShell from the Microsoft Store) starts with one, Windows puts its package's job inside the listed job,
/// and each later process of the package is in that job from its creation (another pane's shell, the quick
/// terminal's, one started outside ngterm): the pane's end would end them all. Started suspended and then assigned,
/// the process is in its package's job only, and the pane's job nests in that one, so the pane's end ends its own
/// tree only.
fn spawn_with(
    builder: CommandBuilder,
    size: Size,
    create: impl FnOnce(CommandBuilder, Size) -> io::Result<Created>,
) -> io::Result<Spawned> {
    let_children_take_ctrl_c();
    let job = kill_on_close_job()?;
    let (conpty, child, thread) = create(builder, size)?;
    let child = join_and_resume(child, &thread, &job)?;
    let pid = child
        .process_id()
        .ok_or_else(|| io::Error::other("the PTY child has no process id"))?;
    // on an error from here the job's close ends the child
    let job = Job::holding(job);
    let master = Master::new(conpty)?;
    Ok(Spawned {
        master,
        child: Child { inner: child, pid, job },
    })
}

/// Start `builder` suspended on a new pseudoconsole of `size`.
fn create(builder: CommandBuilder, size: Size) -> io::Result<Created> {
    let program = program_as_written(&builder);
    let pair = ConPtySystem::default()
        .openpty(size.pty())
        .map_err(|err| pty_error(&err))?;
    // the master spawns the child: the slave behind `PtyPair::slave` cannot be downcast
    drop(pair.slave);
    let master: Box<dyn MasterPty> = pair.master;
    let conpty = master
        .downcast::<ConPtyMasterPty>()
        .map_err(|_| io::Error::other("the PTY master is not a ConPTY master"))?;
    let (child, thread) = conpty.spawn_command_suspended(builder).map_err(|err| {
        let cause: &(dyn std::error::Error + 'static) = &*err;
        start_error(cause, &program).unwrap_or_else(|| pty_error(&err))
    })?;
    Ok((conpty, child, thread))
}

/// Assign a child started suspended to `job`, then resume its main thread, `thread`; a child that cannot join or
/// resume is terminated.
fn join_and_resume(
    child: Box<dyn portable_pty::Child + Send + Sync>,
    thread: &OwnedHandle,
    job: &OwnedHandle,
) -> io::Result<Box<dyn portable_pty::Child + Send + Sync>> {
    // SAFETY: both handles are open, the process one in `child`; a missing one joins nothing.
    let joined = child
        .as_raw_handle()
        .is_some_and(|process| unsafe { AssignProcessToJobObject(job.as_raw_handle(), process) } != 0);
    // SAFETY: resumes the main thread of a child started suspended, whose handle `thread` owns.
    if joined && unsafe { ResumeThread(thread.as_raw_handle()) } != u32::MAX {
        return Ok(child);
    }
    let err = io::Error::last_os_error();
    // portable-pty's kill ignores a failed `TerminateProcess`: the process handle tells whether the child ends
    match child.as_raw_handle() {
        // SAFETY: terminates the child through the process handle that `child` keeps open; no memory is passed.
        Some(process) if unsafe { TerminateProcess(process, 1) } != 0 => {}
        Some(_) => {
            tracing::warn!(err = %io::Error::last_os_error(), "a child that could not join its job could not be terminated");
        }
        None => tracing::warn!("a child that could not join its job has no handle to terminate it through"),
    }
    Err(err)
}

/// The program a builder starts, as its caller wrote it (before portable-pty's PATH search): its first argument, or
/// the command interpreter of a builder for the default program (portable-pty's `%ComSpec%`, else `cmd.exe`).
fn program_as_written(builder: &CommandBuilder) -> std::ffi::OsString {
    match builder.get_argv().first() {
        Some(program) if !builder.is_default_prog() => program.clone(),
        _ => builder.get_env("ComSpec").unwrap_or("cmd.exe".as_ref()).to_owned(),
    }
}

/// A failed start in plain words: the program as written, then the OS error `CreateProcessW` gave (portable-pty keeps
/// it, `build/patches/portable-pty/0009-createprocess-error-keeps-its-io-error.patch`), not its debug-quoted command
/// line. `None` for any other failure (a malformed environment or argument), which keeps portable-pty's own message.
fn start_error(err: &(dyn std::error::Error + 'static), program: &std::ffi::OsStr) -> Option<io::Error> {
    let os = os_error(err)?;
    Some(io::Error::new(os.kind(), format!("{}: {os}", program.display())))
}

/// The OS error in `err`'s chain of causes, if one is there.
fn os_error<'a>(err: &'a (dyn std::error::Error + 'static)) -> Option<&'a io::Error> {
    std::iter::successors(Some(err), |cause| cause.source()).find_map(|cause| cause.downcast_ref::<io::Error>())
}

#[cfg(test)]
mod tests;
