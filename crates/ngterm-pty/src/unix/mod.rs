//! The PTY on Unix: `openpty` through portable-pty, the child a session and process group leader of its own (its
//! `setsid`), the master's descriptor duplicated close-on-exec for the actor.
//!
//! herdr: `src/pty/backend/unix.rs` (`spawn_with_portable_pty`) at `HERDR_REV` (`build/pins.env`), licensed under the
//! Apache License 2.0, modified: the child is waited for without reaping first (`waitid` with `WNOWAIT`), so its
//! process group is never signalled after its id could be reused; the group's hangup and kill, the child signalled
//! before its group; on macOS, libnotify's setup finished before the fork.

mod actor;
mod fd;

use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, ExitStatus, native_pty_system};

pub(crate) use actor::{Control, spawn_actor};

use crate::actor::lock;
use crate::{Size, Spawned, pty_error};

/// A PTY's master side, for [`crate::Actor::spawn`].
#[derive(Debug)]
pub struct Master {
    fd: OwnedFd,
}

impl Master {
    /// A master another process handed over (the session server's handoff): the actor sets the flags it needs.
    #[must_use]
    pub fn from_fd(fd: OwnedFd) -> Self {
        Self { fd }
    }
}

impl From<Master> for OwnedFd {
    fn from(master: Master) -> Self {
        master.fd
    }
}

impl AsFd for Master {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

/// The child on a PTY.
#[derive(Debug)]
pub struct Child {
    inner: Box<dyn portable_pty::Child + Send + Sync>,
    pid: u32,
    /// The child was reaped (or is being): its id, and the group's, may be reused, so nothing is signalled.
    reaped: Arc<Mutex<bool>>,
}

impl Child {
    /// The child's process id, which is also its process group's and its session's.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Wait for the child to exit and reap it; once it is reaped, its status again.
    ///
    /// # Errors
    ///
    /// Waiting failed (the child was reaped by someone else).
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if self.was_reaped() {
            return self.inner.wait();
        }
        exited(self.pid, true)?;
        self.reap()
    }

    /// Wait for the child to exit without reaping it: until [`Child::wait`] or [`Child::try_wait`] reaps it, it stays
    /// a zombie holding its id, so its process group can still be signalled ([`Killer`]) for the members that
    /// outlived it.
    ///
    /// # Errors
    ///
    /// As [`Child::wait`].
    pub fn wait_exit(&mut self) -> io::Result<()> {
        if self.was_reaped() {
            return Ok(());
        }
        exited(self.pid, true).map(drop)
    }

    /// The child's exit status if it has exited (and is now reaped, or was before), else `None`.
    ///
    /// # Errors
    ///
    /// As [`Child::wait`].
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.was_reaped() {
            return self.inner.try_wait();
        }
        if exited(self.pid, false)? {
            self.reap().map(Some)
        } else {
            Ok(None)
        }
    }

    /// A child another process started on a PTY this one took over (the session server's update handoff): its id,
    /// which is also its process group's. This process is not its parent, so it cannot wait for it: [`Child::wait`],
    /// [`Child::wait_exit`] and [`Child::try_wait`] fail (`ECHILD`), and its end is learnt from the PTY's end or the
    /// process table. Its [`Killer`] signals the group until [`Killer::retire`]: nothing here holds the id, which the
    /// system may give to another process once the child is gone, so a caller checks the process is still the child
    /// before signalling (best effort) and retires the killer once it is gone.
    #[must_use]
    pub fn adopt(pid: u32) -> Self {
        Self {
            inner: Box::new(Adopted { pid }),
            pid,
            reaped: Arc::new(Mutex::new(false)),
        }
    }

    /// A handle that signals the child's process group from another thread, safe while this one waits.
    #[must_use]
    pub fn killer(&self) -> Killer {
        Killer {
            group: libc::pid_t::try_from(self.pid).unwrap_or(libc::pid_t::MAX),
            reaped: Arc::clone(&self.reaped),
        }
    }

    /// The child was reaped here: its id is released (and may be reused by now), so it is never waited for by id
    /// again; the status std's child kept is the answer.
    fn was_reaped(&self) -> bool {
        *lock(&self.reaped)
    }

    /// The child has exited and is still a zombie: mark it reaped before its id is released, then reap it.
    fn reap(&mut self) -> io::Result<ExitStatus> {
        *lock(&self.reaped) = true;
        self.inner.wait()
    }
}

/// Whether `pid` has exited (with `block`, wait until it has), leaving it a zombie (`WNOWAIT`).
fn exited(pid: u32, block: bool) -> io::Result<bool> {
    let id = libc::id_t::from(pid);
    let options = libc::WEXITED | libc::WNOWAIT | if block { 0 } else { libc::WNOHANG };
    // SAFETY: `siginfo_t` is plain data the kernel fills; all zeroes is its "no child changed state" value.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: `waitid` writes one `siginfo_t` into the live local passed.
        if unsafe { libc::waitid(libc::P_PID, id, &raw mut info, options) } == 0 {
            break;
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
    // SAFETY: `si_pid` reads the field `waitid` filled (or left zero under WNOHANG when nothing exited).
    Ok(block || unsafe { info.si_pid() } != 0)
}

/// Signals a child's process group: [`Child::killer`].
#[derive(Debug, Clone)]
pub struct Killer {
    group: libc::pid_t,
    reaped: Arc<Mutex<bool>>,
}

impl Killer {
    /// `SIGHUP` to the child's process group, as a terminal closing does.
    ///
    /// Once the child is reaped ([`Child::wait`], [`Child::try_wait`]) its group is never signalled, since the id may
    /// be reused by then, so members that outlived the child (a background job) live on: hang up or kill the tree
    /// before reaping it ([`Child::wait_exit`] waits for its exit without). On Windows the job ends the tree either
    /// way.
    ///
    /// # Errors
    ///
    /// The signal could not be sent (a group already gone is not an error, nor is a reaped child, which is not
    /// signalled).
    pub fn hang_up(&self) -> io::Result<()> {
        self.signal(libc::SIGHUP)
    }

    /// `SIGKILL` to the child's process group: the child and every process of its group end, unless the child was
    /// already reaped, when nothing is signalled (see [`Killer::hang_up`]).
    ///
    /// # Errors
    ///
    /// As [`Killer::hang_up`].
    pub fn kill_tree(&self) -> io::Result<()> {
        self.signal(libc::SIGKILL)
    }

    /// Signal nothing from now on, from this killer or any other of the child's: its group's id may name another
    /// group by now (an adopted child that ended, [`Child::adopt`]).
    pub fn retire(&self) {
        *lock(&self.reaped) = true;
    }

    /// The child first, by its id, then its whole group. The group's signal is not one act: the kernel signals the
    /// members one after another (macOS from a list it took beforehand, with nothing holding off their exits
    /// meanwhile), so a child signalled after its children can see one of them end first and exit as it would have
    /// anyway: a shell waiting on its `sleep` reports the `sleep` killed and exits 0 before its own `SIGKILL` comes,
    /// which is then dropped, as it is for any process already exiting. Signalled first, the child has its signal
    /// before any of its group ends.
    fn signal(&self, signal: libc::c_int) -> io::Result<()> {
        // held while signalling: the child cannot be reaped (and its id reused) in between
        let reaped = lock(&self.reaped);
        if *reaped {
            return Ok(());
        }
        // SAFETY: plain `kill(2)` on the unreaped child (its id is the group's); no memory is passed. Its result is
        // not needed: a child that already exited takes no signal, and the group's signal below reports a failure.
        unsafe { libc::kill(self.group, signal) };
        // SAFETY: plain `killpg(2)` on the group the unreaped child leads; no memory is passed.
        if unsafe { libc::killpg(self.group, signal) } != 0 {
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::ESRCH) => {}
                #[cfg(target_os = "macos")]
                Some(libc::EPERM) => signal_members(self.group, signal),
                _ => return Err(err),
            }
        }
        drop(reaped);
        Ok(())
    }
}

/// The portable-pty side of an adopted child ([`Child::adopt`]): it can be neither waited for nor reaped here.
#[derive(Debug)]
struct Adopted {
    pid: u32,
}

fn not_a_child() -> io::Error {
    io::Error::from_raw_os_error(libc::ECHILD)
}

impl portable_pty::ChildKiller for Adopted {
    fn kill(&mut self) -> io::Result<()> {
        Err(not_a_child())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Self { pid: self.pid })
    }
}

impl portable_pty::Child for Adopted {
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        Err(not_a_child())
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        Err(not_a_child())
    }

    fn process_id(&self) -> Option<u32> {
        Some(self.pid)
    }
}

/// `signal` to each member of `group` in turn: macOS answers `killpg` with `EPERM` once the group's leader exited
/// (an unreaped zombie), though members that outlived it are left. A member that cannot be signalled, or ends
/// meanwhile, is passed over.
#[cfg(target_os = "macos")]
fn signal_members(group: libc::pid_t, signal: libc::c_int) {
    /// More processes than a session's group holds.
    const MOST: usize = 4096;
    let mut pids: Vec<libc::pid_t> = vec![0; MOST];
    let bytes = libc::c_int::try_from(MOST * size_of::<libc::pid_t>()).unwrap_or(libc::c_int::MAX);
    // SAFETY: libproc writes at most `bytes` bytes of process ids into the buffer, which holds that many.
    let listed = unsafe { libc::proc_listpgrppids(group, pids.as_mut_ptr().cast(), bytes) };
    for &pid in pids.iter().take(usize::try_from(listed).unwrap_or(0)) {
        if pid > 0 {
            // SAFETY: plain `kill(2)` on a member of the group, which the unreaped leader keeps from being reused;
            // no memory is passed.
            unsafe { libc::kill(pid, signal) };
        }
    }
}

/// Finish libnotify's one-time setup in this process before it forks (macOS). Every forked child runs libnotify's
/// fork handler (`_notify_fork_child`, from `fork` itself, before the exec), which reads that setup's once-gate: a
/// fork that catches another thread in the middle of the process's first libnotify use (`getpwuid`, which
/// portable-pty calls for a command without `HOME`, `localtime`, `confstr`) leaves the child a gate held by a thread
/// it does not have, and the child dies of `SIGILL` (`os_once_t is corrupt`, crashed on the child side of the fork,
/// pre-exec). The spawn still succeeds, as the child closed std's error pipe without a word, and the PTY ends with
/// nothing read. Once the setup is done it never runs again, and this call waits for one in progress, so no fork
/// after it can catch one.
#[cfg(target_os = "macos")]
fn settle_libnotify() {
    unsafe extern "C" {
        /// `<notify.h>`; the token is only looked up, and every call starts with libnotify's setup.
        fn notify_is_valid_token(token: libc::c_int) -> bool;
    }
    // SAFETY: a lookup of a token number in libnotify's table, which libSystem holds; no memory is passed.
    unsafe { notify_is_valid_token(0) };
}

/// XNU's internal "redrive the open" errno (`bsd/sys/errno.h`, kernel-only), which its `/dev/ptmx` open returns to
/// user space once the kernel's own retries of a lost clone race run out.
#[cfg(target_os = "macos")]
const EREDRIVEOPEN: i32 = -6;

/// How many times an `openpty` that lost XNU's ptmx clone race is tried again before its error is returned.
#[cfg(target_os = "macos")]
const CLONE_RACE_RETRIES: usize = 3;

/// Open a PTY pair with `open`, again on macOS when the open lost XNU's `/dev/ptmx` clone race to another open or
/// close of a PTY, in any process (XNU's `bsd/kern/tty_ptmx.c` and `bsd/kern/tty_dev.c`, `apple-oss-distributions/xnu`):
///
/// - `ptmx_clone` hands an opener the first free minor under the devfs lock, but the slot is only taken later, in
///   `ptmx_get_ioctl` after an allocation made without the lock. Two opens can be handed one minor; the one that
///   comes second finds the slot taken and `ptcopen` returns `EREDRIVEOPEN`. `vn_open_auth` redrives the open at
///   most 10 times, then returns that kernel-only errno (`-6`) from `open("/dev/ptmx")`.
/// - The same window lets an opener handed the vector's end meet a close that freed a slot meanwhile: the vector is
///   not grown, the minor is out of range (`ptmx_get_ioctl failed because minor number ... was out of range`) and
///   `ptcopen` returns `ENXIO`.
/// - `ptmx_free_ioctl`, the last close of a pair, frees the minor's slot before it removes that minor's
///   `/dev/ttysNNN` node: a new pair handed the minor meanwhile opens its replica through the old node as it is
///   removed, and the open fails with `ENXIO`.
///
/// The kernel's state is consistent again once the racing open or close is through, so a fresh `openpty` gets a
/// minor of its own. Only these two errnos are retried, a bounded number of times; anything else, and a race lost
/// more often than that, is returned as it is.
fn retry_clone_race<T, E>(mut open: impl FnMut() -> Result<T, E>) -> io::Result<T>
where
    E: std::fmt::Display + AsRef<dyn std::error::Error + Send + Sync + 'static>,
{
    #[cfg(target_os = "macos")]
    for _ in 0..CLONE_RACE_RETRIES {
        match open() {
            Err(err) if lost_clone_race(err.as_ref()) => {
                tracing::debug!("openpty lost the ptmx clone race ({err:#}); opening again");
            }
            opened => return opened.map_err(|err| pty_error(&err)),
        }
    }
    open().map_err(|err| pty_error(&err))
}

/// Whether an `openpty` failed with one of the errnos of XNU's ptmx clone race ([`retry_clone_race`]).
#[cfg(target_os = "macos")]
fn lost_clone_race(err: &(dyn std::error::Error + 'static)) -> bool {
    matches!(openpty_errno(err), Some(libc::ENXIO | EREDRIVEOPEN))
}

/// The errno of a failed `openpty`: the first `io::Error` in its chain that carries one (portable-pty's keeps it,
/// `build/patches/portable-pty/0007-openpty-error-keeps-its-errno.patch`).
#[cfg(any(target_os = "macos", test))]
fn openpty_errno(err: &(dyn std::error::Error + 'static)) -> Option<i32> {
    std::iter::successors(Some(err), |err| err.source())
        .filter_map(|err| err.downcast_ref::<io::Error>())
        .find_map(io::Error::raw_os_error)
}

pub(crate) fn spawn(builder: CommandBuilder, size: Size) -> io::Result<Spawned> {
    #[cfg(target_os = "macos")]
    settle_libnotify();
    let pair = retry_clone_race(|| native_pty_system().openpty(size.pty()))?;
    let raw = pair
        .master
        .as_raw_fd()
        .ok_or_else(|| io::Error::other("the PTY master has no descriptor"))?;
    // SAFETY: `raw` is the master's descriptor, which `pair` keeps open for this borrow.
    let fd = unsafe { BorrowedFd::borrow_raw(raw) }.try_clone_to_owned()?;
    let child = pair.slave.spawn_command(builder).map_err(|err| pty_error(&err))?;
    // portable-pty's own master and slave close: the duplicate is the one master descriptor left in this process
    drop(pair);
    let pid = child
        .process_id()
        .ok_or_else(|| io::Error::other("the PTY child has no process id"))?;
    Ok(Spawned {
        master: Master { fd },
        child: Child {
            inner: child,
            pid,
            reaped: Arc::new(Mutex::new(false)),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;

    use super::*;

    /// An `openpty` failure shaped as portable-pty's: the `io::Error` under its context.
    #[derive(Debug)]
    struct OpenFailed(io::Error);

    impl std::fmt::Display for OpenFailed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("failed to openpty")
        }
    }

    impl std::error::Error for OpenFailed {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    type Failure = Box<dyn std::error::Error + Send + Sync>;

    /// `retry_clone_race` over failures with these errnos, then a success; the calls it made, and its result.
    fn open_failing(errnos: &[i32]) -> (usize, io::Result<()>) {
        let mut calls = 0;
        let result = retry_clone_race(|| {
            let errno = errnos.get(calls).copied();
            calls += 1;
            errno.map_or(Ok(()), |errno| {
                Err(Failure::from(OpenFailed(io::Error::from_raw_os_error(errno))))
            })
        });
        (calls, result)
    }

    #[test]
    fn an_open_that_fails_otherwise_is_not_retried() {
        let (calls, result) = open_failing(&[libc::EACCES, libc::EACCES]);
        assert_eq!(calls, 1);
        assert_eq!(result.unwrap_err().to_string(), "failed to openpty");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_open_that_lost_the_clone_race_is_retried() {
        let (calls, result) = open_failing(&[libc::ENXIO, EREDRIVEOPEN]);
        assert_eq!(calls, 3);
        result.unwrap();
        // a race lost more often than the bound returns the last loss
        let lost = [EREDRIVEOPEN; CLONE_RACE_RETRIES + 1];
        let (calls, result) = open_failing(&lost);
        assert_eq!(calls, CLONE_RACE_RETRIES + 1);
        assert_eq!(result.unwrap_err().to_string(), "failed to openpty");
    }

    /// portable-pty's own `openpty` failure carries its errno to [`openpty_errno`], which the clone race's retry reads:
    /// a child process of this test spends its descriptor limit and opens a PTY, which fails with `EMFILE`.
    #[test]
    fn portable_ptys_openpty_failure_keeps_its_errno() {
        const CHILD: &str = "NGTERM_PTY_TEST_NO_DESCRIPTORS";
        const NAME: &str = "unix::tests::portable_ptys_openpty_failure_keeps_its_errno";
        if std::env::var_os(CHILD).is_some() {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: reads and sets this child process's descriptor limit through a valid `rlimit`.
            unsafe {
                assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit), 0);
                let spent = libc::rlimit { rlim_cur: 0, ..limit };
                assert_eq!(libc::setrlimit(libc::RLIMIT_NOFILE, &raw const spent), 0);
            }
            let opened = native_pty_system().openpty(Size::cells(24, 80).pty());
            // SAFETY: as above, the limit read before.
            unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) };
            let err = opened.err().expect("an open past the descriptor limit fails");
            assert_eq!(openpty_errno(err.as_ref()), Some(libc::EMFILE), "{err:?}");
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([NAME, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// This process's descriptors on a PTY.
    #[cfg(target_os = "linux")]
    fn pty_descriptors() -> usize {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| std::fs::read_link(entry.path()).ok())
            .filter(|target| target.starts_with("/dev/pts/") || target.as_os_str() == "/dev/ptmx")
            .count()
    }

    /// The only unit test that opens a PTY (the actor's run on socket pairs), so the count is this test's own.
    #[test]
    fn spawning_leaves_one_close_on_exec_master_descriptor_in_this_process() {
        #[cfg(target_os = "linux")]
        let before = pty_descriptors();
        let mut builder = CommandBuilder::new("/bin/sh");
        builder.args(["-c", "exit 0"]);
        let Spawned { master, mut child } = spawn(builder, Size::cells(24, 80)).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(pty_descriptors(), before + 1);
        child.wait().unwrap();
        // SAFETY: reads the descriptor flags of the master owned here.
        let flags = unsafe { libc::fcntl(master.as_fd().as_raw_fd(), libc::F_GETFD) };
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        drop(master);
        #[cfg(target_os = "linux")]
        assert_eq!(pty_descriptors(), before);
    }
}
