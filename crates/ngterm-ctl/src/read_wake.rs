//! Ending reads another thread blocks in, on the app's socket or on the viewer's input. Shutting the socket down tells
//! the app this end hung up, and on Linux it also ends a read blocked on the socket; on macOS it does not reliably: a
//! `shutdown` racing a `recv` another thread is blocked in on the same socket can leave that `recv` asleep for good,
//! the shutdown having succeeded (about once in 200 tries on macOS 26 on x86-64, with a plain `socketpair`). So on Unix a
//! reader waits in `poll` on its source and on a wake pipe, which [`ReadWake::wake`] makes readable for good. Windows
//! reads its source directly: closing the app's pipe ends a read blocked on it there.

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

/// What a [`ReadWake`] reads: anything with a descriptor `poll` can wait on, on Unix.
#[cfg(unix)]
pub trait Source: Read + std::os::fd::AsFd {}

#[cfg(unix)]
impl<T: Read + std::os::fd::AsFd> Source for T {}

/// What a [`ReadWake`] reads: anything readable, on Windows.
#[cfg(windows)]
pub trait Source: Read {}

#[cfg(windows)]
impl<T: Read> Source for T {}

/// A wake every read made through it ends at, the later ones too.
#[derive(Debug)]
pub struct ReadWake {
    woken: AtomicBool,
    /// The wake pipe: never read, so one byte keeps it readable.
    #[cfg(unix)]
    pipe: (io::PipeReader, io::PipeWriter),
}

impl ReadWake {
    /// # Errors
    ///
    /// The wake pipe could not be made.
    #[cfg(unix)]
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            woken: AtomicBool::new(false),
            pipe: io::pipe()?,
        })
    }

    /// # Errors
    ///
    /// Never on Windows, which needs no wake pipe.
    #[cfg(windows)]
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the Unix form can fail, and callers share one signature"
    )]
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            woken: AtomicBool::new(false),
        })
    }

    /// Ends the read waiting now, and every later one, with `Ok(0)`. The pipe's reader lives as long as `self`, so the
    /// write never raises SIGPIPE.
    pub fn wake(&self) {
        self.woken.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        {
            use std::io::Write as _;
            let _ = (&self.pipe.1).write_all(&[1]);
        }
    }

    /// Whether [`ReadWake::wake`] was called, which tells a read it ended from its source's end.
    pub fn is_woken(&self) -> bool {
        self.woken.load(Ordering::SeqCst)
    }

    /// Reads `source` once it is readable, or `Ok(0)` once woken; the wake wins a tie.
    ///
    /// # Errors
    ///
    /// `poll` or the read failed (an interrupted `poll` waits again).
    #[cfg(unix)]
    pub fn read(&self, mut source: impl Source, buffer: &mut [u8]) -> io::Result<usize> {
        use std::os::fd::AsRawFd as _;
        let mut pollers = [
            libc::pollfd {
                fd: source.as_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: self.pipe.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            // SAFETY: `pollers` is a live array of two initialized `pollfd`s, and the count passed is its length.
            if unsafe { libc::poll(pollers.as_mut_ptr(), 2, -1) } >= 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
        if pollers[1].revents != 0 {
            return Ok(0);
        }
        source.read(buffer)
    }

    /// Reads `source`, or `Ok(0)` once woken: on Windows, closing the source is what ends a read already waiting.
    ///
    /// # Errors
    ///
    /// The read failed.
    #[cfg(windows)]
    pub fn read(&self, mut source: impl Source, buffer: &mut [u8]) -> io::Result<usize> {
        if self.is_woken() {
            return Ok(0);
        }
        source.read(buffer)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    use super::*;

    #[test]
    fn a_read_returns_what_its_source_has() {
        let (ours, theirs) = ngterm_server::endpoint::pair().unwrap();
        crate::client::send_all(&theirs, b"hi").unwrap();
        let wake = ReadWake::new().unwrap();
        let mut buffer = [0_u8; 8];
        let count = wake.read(&ours, &mut buffer).unwrap();
        assert_eq!(&buffer[..count], b"hi");
    }

    /// The race macOS loses: the socket shut down while another thread is blocked reading it, which the wake ends
    /// whichever comes first.
    #[cfg(unix)]
    #[test]
    fn a_wake_after_a_shutdown_ends_a_blocked_read_every_time() {
        for _ in 0..500 {
            let (ours, _theirs) = ngterm_server::endpoint::pair().unwrap();
            let wake = ReadWake::new().unwrap();
            let count = thread::scope(|scope| {
                scope.spawn(|| {
                    let _ = ours.shutdown(std::net::Shutdown::Both);
                    wake.wake();
                });
                wake.read(&ours, &mut [0_u8; 8])
            });
            assert_eq!(count.unwrap(), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_wake_ends_every_later_read_too() {
        let (ours, _theirs) = ngterm_server::endpoint::pair().unwrap();
        let wake = ReadWake::new().unwrap();
        let blocked = thread::scope(|scope| {
            let reading = scope.spawn(|| wake.read(&ours, &mut [0_u8; 8]).unwrap());
            thread::sleep(Duration::from_millis(50));
            wake.wake();
            reading.join().unwrap()
        });
        assert_eq!(blocked, 0);
        assert_eq!(wake.read(&ours, &mut [0_u8; 8]).unwrap(), 0);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn an_interrupted_poll_retries_until_the_source_is_readable() {
        if std::env::var_os("READ_WAKE_INTERRUPT_CHILD").is_some() {
            interrupted_poll();
            return;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "read_wake::tests::an_interrupted_poll_retries_until_the_source_is_readable",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
            .env("READ_WAKE_INTERRUPT_CHILD", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    static SIGNALS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    extern "C" fn signal_received(_: libc::c_int) {
        SIGNALS.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn waiting_in_poll(tid: libc::pid_t) -> bool {
        std::fs::read_to_string(format!("/proc/self/task/{tid}/syscall"))
            .unwrap()
            .split_whitespace()
            .next()
            .is_some_and(|number| number == libc::SYS_poll.to_string())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "reader did not reach the expected state"
            );
            thread::yield_now();
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn interrupted_poll() {
        // SAFETY: all-zero bytes are a valid initial sigaction before setting its handler and mask.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = signal_received as *const () as usize;
        // SAFETY: action's mask is live and writable.
        let emptied = unsafe { libc::sigemptyset(&raw mut action.sa_mask) };
        assert_eq!(emptied, 0);
        // SAFETY: action is initialized, the handler only updates a lock-free atomic, and this process runs one test.
        let installed = unsafe { libc::sigaction(libc::SIGUSR1, &raw const action, std::ptr::null_mut()) };
        assert_eq!(installed, 0);
        let wake = ReadWake::new().unwrap();
        let (ours, theirs) = ngterm_server::endpoint::pair().unwrap();
        // SAFETY: returns the pthread identifier of this live thread.
        let pthread = unsafe { libc::pthread_self() };
        // SAFETY: returns the kernel identifier of this live thread.
        let tid = unsafe { libc::gettid() };
        let mut buffer = [0_u8; 8];
        let count = thread::scope(|scope| {
            let signaler = scope.spawn(|| {
                let result = std::panic::catch_unwind(|| {
                    wait_until(|| waiting_in_poll(tid));
                    // SAFETY: the scoped reader thread remains live until this sender joins.
                    assert_eq!(unsafe { libc::pthread_kill(pthread, libc::SIGUSR1) }, 0);
                    wait_until(|| SIGNALS.load(Ordering::SeqCst) > 0 && waiting_in_poll(tid));
                    crate::client::send_all(&theirs, b"ready").unwrap();
                });
                if result.is_err() {
                    wake.wake();
                }
                result.unwrap();
            });
            let count = wake.read(&ours, &mut buffer).unwrap();
            signaler.join().unwrap();
            count
        });
        assert_eq!(&buffer[..count], b"ready");
        assert!(!wake.is_woken());
        assert_eq!(SIGNALS.load(Ordering::SeqCst), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_read_errors_are_returned_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let source = std::fs::File::open(directory.path()).unwrap();
        let error = ReadWake::new().unwrap().read(source, &mut [0_u8; 1]).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EISDIR));
    }
}
