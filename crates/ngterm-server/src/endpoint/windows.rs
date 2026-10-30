//! The endpoint on Windows: the named pipe the socket path names ([`ngterm_core::paths::control_pipe_name`]),
//! owner-only, closed to remote clients, served by the process that holds the socket's ownership lock.
//!
//! The pipe's creation and its client's checks follow ngterm-session's namespace pipe (`endpoint/windows.rs` there,
//! after herdr's `src/platform/windows.rs` at `HERDR_REV`, Apache-2.0): the same descriptor and server-user check,
//! shared through [`ngterm_core::pipe_security`].

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::windows::io::{AsHandle as _, AsRawHandle as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use interprocess::TryClone as _;
use interprocess::os::windows::named_pipe::{DuplexPipeStream, PipeListenerOptions, pipe_mode::Bytes};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::IO::CancelIoEx;
use windows_sys::Win32::System::Pipes::DisconnectNamedPipe;

use super::watchdog;

pub(super) type Listener = interprocess::os::windows::named_pipe::PipeListener<Bytes, Bytes>;

type Pipe = DuplexPipeStream<Bytes>;

/// What each direction of a pipe buffers, near an `AF_UNIX` socket's default, so a response or a stream's lines are
/// not handed over 512 bytes (interprocess's default) at a time.
const BUFFER_BYTES: u32 = 64 * 1024;

/// One control connection, either end: a byte-mode pipe stream with the part of std's socket surface the server
/// and `ngtermctl` use. Reads and writes take the timeouts set on it (shared with its duplicates, as a socket's
/// options are), a write that times out disconnects the pipe (its peer is stalled, and what it was sent is cut), and
/// [`Connection::shutdown`] ends either end from any thread, for good, as `shutdown(2)` does a socket's.
#[derive(Debug)]
pub struct Connection {
    pipe: Pipe,
    timeouts: Arc<Timeouts>,
}

/// What a connection's duplicates share: its timeouts, whether it was shut down, and how many calls are in flight
/// on it, which a shutdown keeps cancelling until none is left.
#[derive(Debug, Default)]
struct Timeouts {
    read: Mutex<Option<Duration>>,
    write: Mutex<Option<Duration>>,
    shut: AtomicBool,
    calls: AtomicUsize,
}

/// How long [`Connection::shutdown`] keeps cancelling the calls in flight before it gives up on them.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(2);

impl Timeouts {
    fn get(slot: &Mutex<Option<Duration>>) -> Option<Duration> {
        *slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets `slot`; a zero duration is refused, as std refuses it for a socket.
    fn set(slot: &Mutex<Option<Duration>>, timeout: Option<Duration>) -> io::Result<()> {
        if timeout.is_some_and(|timeout| timeout.is_zero()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot set a 0 duration timeout",
            ));
        }
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = timeout;
        Ok(())
    }
}

impl Connection {
    fn new(pipe: Pipe) -> Self {
        Self {
            pipe,
            timeouts: Arc::default(),
        }
    }

    /// Another handle to the same pipe end, sharing its timeouts.
    ///
    /// # Errors
    ///
    /// The handle could not be duplicated.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            pipe: self.pipe.try_clone()?,
            timeouts: Arc::clone(&self.timeouts),
        })
    }

    /// Bounds each later read: past `timeout` it fails with [`io::ErrorKind::TimedOut`] and the connection stays
    /// usable. `None` waits for as long as it takes.
    ///
    /// # Errors
    ///
    /// A zero duration.
    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        Timeouts::set(&self.timeouts.read, timeout)
    }

    /// Bounds each later write: past `timeout` it fails with [`io::ErrorKind::TimedOut`] and the pipe is
    /// disconnected. `None` waits for as long as it takes.
    ///
    /// # Errors
    ///
    /// A zero duration.
    pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        Timeouts::set(&self.timeouts.write, timeout)
    }

    /// The read timeout set on it.
    ///
    /// # Errors
    ///
    /// None; the signature is std's.
    pub fn read_timeout(&self) -> io::Result<Option<Duration>> {
        Ok(Timeouts::get(&self.timeouts.read))
    }

    /// The write timeout set on it.
    ///
    /// # Errors
    ///
    /// None; the signature is std's.
    pub fn write_timeout(&self) -> io::Result<Option<Duration>> {
        Ok(Timeouts::get(&self.timeouts.write))
    }

    /// Ends the connection from any thread, as `shutdown(2)` with [`Shutdown::Both`] does: a server's end is
    /// disconnected (the client reads its end at once; what it had not read is dropped), and every read or write
    /// pending on the pipe, from any duplicate, is cancelled. A client's end has its pending calls cancelled; its
    /// peer reads the end once the last handle to it closes. Either way every later read on it, from any duplicate,
    /// returns 0 and every later write fails with [`io::ErrorKind::BrokenPipe`].
    ///
    /// # Errors
    ///
    /// [`Shutdown::Read`] or [`Shutdown::Write`]: a pipe has no half-close.
    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        if how != Shutdown::Both {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a named pipe has no half-close",
            ));
        }
        // sticky, as a socket's: every later read ends at once and every later write fails, on each duplicate
        self.timeouts.shut.store(true, Ordering::SeqCst);
        self.disconnect();
        // a call that read the flag just before it was set may enter the kernel after the cancel above, which would
        // then miss it; cancelling until no call is in flight catches those
        let deadline = Instant::now() + SHUTDOWN_DRAIN;
        while self.timeouts.calls.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
            self.cancel_pending();
        }
        Ok(())
    }

    /// Runs `call` counted as in flight, or `closed` once the connection is shut down; a call a shutdown cancelled
    /// answers as `closed` does.
    fn call<T>(&self, call: impl FnOnce() -> io::Result<T>, closed: fn() -> io::Result<T>) -> io::Result<T> {
        self.timeouts.calls.fetch_add(1, Ordering::SeqCst);
        let result = if self.timeouts.shut.load(Ordering::SeqCst) {
            closed()
        } else {
            call()
        };
        self.timeouts.calls.fetch_sub(1, Ordering::SeqCst);
        match result {
            Err(_) if self.timeouts.shut.load(Ordering::SeqCst) => closed(),
            result => result,
        }
    }

    fn handle(&self) -> HANDLE {
        self.pipe.as_handle().as_raw_handle()
    }

    fn disconnect(&self) {
        let handle = self.handle();
        // Failures (a client's end, a pipe already disconnected, nothing pending) leave it as closed as it gets.
        if self.pipe.is_server() {
            // SAFETY: the handle is the open pipe end `self.pipe` owns, borrowed for the call.
            unsafe { DisconnectNamedPipe(handle) };
        }
        self.cancel_pending();
    }

    fn cancel_pending(&self) {
        // SAFETY: the handle is the open pipe end `self.pipe` owns, borrowed for the call; a null overlapped pointer
        // cancels every operation pending on the pipe, through any of its duplicates.
        unsafe { CancelIoEx(self.handle(), std::ptr::null()) };
    }
}

impl Read for &Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let timeout = Timeouts::get(&self.timeouts.read);
        self.call(
            || watchdog::timed(self.handle(), timeout, || (&self.pipe).read(buf)),
            || Ok(0),
        )
    }
}

impl Write for &Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let timeout = Timeouts::get(&self.timeouts.write);
        let written = self.call(
            || watchdog::timed(self.handle(), timeout, || (&self.pipe).write(buf)),
            || Err(io::ErrorKind::BrokenPipe.into()),
        );
        if written
            .as_ref()
            .is_err_and(|error| error.kind() == io::ErrorKind::TimedOut)
        {
            self.disconnect();
        }
        written
    }

    /// Nothing to do, as for a socket: a written byte is the pipe's.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        (&*self).read(buf)
    }
}

impl Write for Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self).write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        (&*self).flush()
    }
}

fn pipe_path(socket: &Path) -> String {
    format!(r"\\.\pipe\{}", ngterm_core::paths::control_pipe_name(socket))
}

pub(super) fn bind(socket: &Path) -> io::Result<Listener> {
    let path = pipe_path(socket);
    let mut options = PipeListenerOptions::new().path(path.as_str());
    options.security_descriptor = Some(owner_only_security_descriptor()?);
    options.input_buffer_size_hint = BUFFER_BYTES;
    options.output_buffer_size_hint = BUFFER_BYTES;
    options
        .create_duplex::<Bytes>()
        .map_err(|error| name_taken_error(&path, error))
}

pub(super) fn accept(listener: &Listener) -> io::Result<Connection> {
    listener.accept().map(Connection::new)
}

pub(super) fn connect(socket: &Path) -> io::Result<Connection> {
    let path = pipe_path(socket);
    let pipe = Pipe::connect_by_path(path.as_str())?;
    let server_pid = pipe.server_process_id()?;
    if !ngterm_core::pipe_security::runs_as_current_user(server_pid)? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the pipe at {path} belongs to a process of another user (pid {server_pid})"),
        ));
    }
    Ok(Connection::new(pipe))
}

/// A connection to the pipe without the server-user check: the server's own, to its own pipe.
pub(super) fn connect_unchecked(socket: &Path) -> io::Result<Connection> {
    Pipe::connect_by_path(pipe_path(socket).as_str()).map(Connection::new)
}

/// Nothing: a pipe ends with its last handle.
pub(super) fn remove(_socket: &Path) {}

/// Two connected ends of a pipe of their own, under a name no one else uses.
pub(super) fn pair() -> io::Result<(Connection, Connection)> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let name = format!(
        "ngterm-pair-{}-{}-{nanos}.sock",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let socket = std::env::temp_dir().join(name);
    let listener = bind(&socket)?;
    // the instance `bind` created takes the client before the accept: a pipe instance is connectable once created
    let client = connect_unchecked(&socket)?;
    let server = accept(&listener)?;
    Ok((server, client))
}

/// The pipe's descriptor: SYSTEM's and the owner's alone, at the server's integrity level.
fn owner_only_security_descriptor() -> io::Result<SecurityDescriptor> {
    let sddl = U16CString::from_str(ngterm_core::pipe_security::owner_only_pipe_sddl()?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    SecurityDescriptor::deserialize(&sddl)
}

/// A pipe some other process holds makes `FILE_FLAG_FIRST_PIPE_INSTANCE` fail with `ERROR_ACCESS_DENIED`.
fn name_taken_error(path: &str, error: io::Error) -> io::Error {
    if error.kind() != io::ErrorKind::PermissionDenied {
        return error;
    }
    io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("another process holds the pipe {path}: {error}"),
    )
}
