//! The endpoint on Windows: the named pipe the socket path names ([`ngterm_core::paths::control_pipe_name`]),
//! owner-only, closed to remote clients, served by the process that holds the socket's ownership lock.
//!
//! The pipe's creation and its client's checks follow ngterm-session's namespace pipe (`endpoint/windows.rs` there,
//! after herdr's `src/platform/windows.rs` at `HERDR_REV`, Apache-2.0): the same descriptor and server-user check,
//! shared through [`ngterm_core::pipe_security`].
//!
//! The listener and the server's ends are ngterm's own, not interprocess's: its `PipeListener::accept` (2.4.6)
//! disconnects an instance whose client already closed (`ERROR_NO_DATA`), which drops a request written before the
//! accept, and its streams made from a handle reopen it (`ReOpenFile`), which turns a server end into a new client of
//! the pipe's listening instance. Clients still connect through interprocess and keep only the handle.

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use interprocess::os::windows::named_pipe::{DuplexPipeStream, pipe_mode::Bytes};
use interprocess::os::windows::security_descriptor::{AsSecurityDescriptor as _, SecurityDescriptor};
use widestring::U16CString;
use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES,
};
use windows_sys::Win32::System::Threading::CreateEventW;
use windows_sys::core::BOOL;

use super::watchdog;

/// The pipe's listening instance, and what makes the next one.
pub(super) struct Listener {
    instance: Mutex<OwnedHandle>,
    path: U16CString,
    descriptor: SecurityDescriptor,
}

/// One end of a connection: an overlapped byte-mode pipe handle, read and written through overlapped calls so a
/// read in flight never holds up a write on a duplicate, and so a cancel from another thread ends either.
#[derive(Debug)]
struct Pipe {
    handle: OwnedHandle,
    server: bool,
}

impl Pipe {
    fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            handle: self.handle.try_clone()?,
            server: self.server,
        })
    }

    /// Reads what the pipe holds, within `timeout`; a peer gone (or disconnected) reads as the end, as a socket's
    /// does.
    fn read(&self, buf: &mut [u8], timeout: Option<Duration>) -> io::Result<usize> {
        let handle = self.handle.as_raw_handle();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let ptr = buf.as_mut_ptr();
        // SAFETY: `buf` outlives the call, which `overlapped` waits for, and `len` is within it.
        let read = overlapped(handle, timeout, |ov| unsafe {
            ReadFile(handle, ptr, len, std::ptr::null_mut(), ov)
        });
        match read {
            Ok(read) => usize::try_from(read).map_err(io::Error::other),
            Err(error) if is_peer_gone(&error) => Ok(0),
            Err(error) => Err(error),
        }
    }

    fn write(&self, buf: &[u8], timeout: Option<Duration>) -> io::Result<usize> {
        let handle = self.handle.as_raw_handle();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let ptr = buf.as_ptr();
        // SAFETY: `buf` outlives the call, which `overlapped` waits for, and `len` is within it.
        let written = overlapped(handle, timeout, |ov| unsafe {
            WriteFile(handle, ptr, len, std::ptr::null_mut(), ov)
        })?;
        usize::try_from(written).map_err(io::Error::other)
    }
}

fn is_peer_gone(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe || is_os_error(error, ERROR_PIPE_NOT_CONNECTED)
}

fn is_os_error(error: &io::Error, code: u32) -> bool {
    error.raw_os_error().and_then(|raw| u32::try_from(raw).ok()) == Some(code)
}

/// Starts `call`, an overlapped call on `handle` given the `OVERLAPPED` to use, and waits for it to finish: the bytes
/// it moved, or its error. Past `timeout` that call alone is cancelled ([`watchdog::timed`]).
fn overlapped(
    handle: HANDLE,
    timeout: Option<Duration>,
    call: impl FnOnce(*mut OVERLAPPED) -> BOOL,
) -> io::Result<u32> {
    // SAFETY: a manual-reset event, unnamed and unshared; the handle is checked before it is owned.
    let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateEventW returned a fresh handle that nothing else owns.
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    // SAFETY: an all-zero OVERLAPPED is a valid one (offset 0, no result yet).
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    overlapped.hEvent = event.as_raw_handle();
    if call(&raw mut overlapped) == 0 {
        let error = io::Error::last_os_error();
        if !is_os_error(&error, ERROR_IO_PENDING) {
            return Err(error);
        }
    }
    let pending = &raw const overlapped;
    watchdog::timed(handle, pending, timeout, || {
        let mut moved = 0;
        // SAFETY: `overlapped` and its event live until the call completes, which this waits for.
        if unsafe { GetOverlappedResult(handle, pending, &raw mut moved, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(moved)
    })
}

/// What each direction of a pipe buffers, near an `AF_UNIX` socket's default, so a response or a stream's lines are
/// not handed over a few hundred bytes at a time.
const BUFFER_BYTES: u32 = 64 * 1024;

/// One control connection, either end: a byte-mode pipe stream with the part of std's socket surface the server
/// and `ngtermctl` use. Reads and writes take the timeouts set on it (shared with its duplicates, as a socket's
/// options are), a call that times out ends alone, the calls pending on its duplicates untouched, and
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

    /// Bounds each later write: past `timeout` it fails with [`io::ErrorKind::TimedOut`]; what part of it reached
    /// the peer is unknown, so its caller ends the connection. `None` waits for as long as it takes.
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
        self.pipe.handle.as_raw_handle()
    }

    fn disconnect(&self) {
        let handle = self.handle();
        // Failures (a client's end, a pipe already disconnected, nothing pending) leave it as closed as it gets.
        if self.pipe.server {
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
        self.call(|| self.pipe.read(buf, timeout), || Ok(0))
    }
}

impl Write for &Connection {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let timeout = Timeouts::get(&self.timeouts.write);
        self.call(
            || self.pipe.write(buf, timeout),
            || Err(io::ErrorKind::BrokenPipe.into()),
        )
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
    let wide = U16CString::from_str(&path).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let descriptor = owner_only_security_descriptor()?;
    let first = create_instance(&wide, &descriptor, true).map_err(|error| name_taken_error(&path, error))?;
    Ok(Listener {
        instance: Mutex::new(first),
        path: wide,
        descriptor,
    })
}

/// One instance of the pipe: overlapped, duplex, byte mode, remote clients refused, [`BUFFER_BYTES`] each way, the
/// instance count unlimited, under the owner-only `descriptor`; with `FILE_FLAG_FIRST_PIPE_INSTANCE` when `first`.
/// These are the instances interprocess's listener made (`named_pipe/listener/create_instance.rs` at 2.4.6).
fn create_instance(path: &U16CString, descriptor: &SecurityDescriptor, first: bool) -> io::Result<OwnedHandle> {
    let mut open_mode = FILE_FLAG_OVERLAPPED | PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(io::Error::other)?,
        lpSecurityDescriptor: descriptor.as_sd().cast_mut(),
        bInheritHandle: 0,
    };
    // SAFETY: the path is a NUL-terminated wide string, the attributes and the descriptor they point to live across
    // the call; the returned handle is checked before it is owned.
    let handle = unsafe {
        CreateNamedPipeW(
            path.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            BUFFER_BYTES,
            BUFFER_BYTES,
            0,
            &raw const attributes,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateNamedPipeW returned a fresh handle that nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Waits for a client on the listening instance and hands that instance out, a fresh one listening in its place.
/// A client that connected, wrote and closed before the wait is handed out too: its bytes stay in the instance, read
/// before the end, as a socket's peer's are.
pub(super) fn accept(listener: &Listener) -> io::Result<Connection> {
    let mut instance = listener.instance.lock().unwrap_or_else(PoisonError::into_inner);
    let handle = instance.as_raw_handle();
    // SAFETY: the instance is open while the lock is held; `overlapped` waits for the call to finish.
    match overlapped(handle, None, |ov| unsafe { ConnectNamedPipe(handle, ov) }) {
        Ok(_) => {}
        Err(error) if is_os_error(&error, ERROR_PIPE_CONNECTED) || is_os_error(&error, ERROR_NO_DATA) => {}
        Err(error) => return Err(error),
    }
    // a failure here keeps the connected instance listening: the next accept hands it out
    let next = create_instance(&listener.path, &listener.descriptor, false)?;
    let connected = std::mem::replace(&mut *instance, next);
    Ok(Connection::new(Pipe {
        handle: connected,
        server: true,
    }))
}

pub(super) fn connect(socket: &Path) -> io::Result<Connection> {
    let path = pipe_path(socket);
    let pipe = DuplexPipeStream::<Bytes>::connect_by_path(path.as_str())?;
    ngterm_core::pipe_security::check_server_user(&path, pipe.server_process_id()?)?;
    client(pipe)
}

/// A connection to the pipe without the server-user check: the server's own, to its own pipe.
pub(super) fn connect_unchecked(socket: &Path) -> io::Result<Connection> {
    client(DuplexPipeStream::<Bytes>::connect_by_path(pipe_path(socket).as_str())?)
}

fn client(pipe: DuplexPipeStream<Bytes>) -> io::Result<Connection> {
    let handle = OwnedHandle::try_from(pipe).map_err(|_| io::Error::other("a split pipe stream"))?;
    Ok(Connection::new(Pipe { handle, server: false }))
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

#[cfg(test)]
#[path = "windows_tests.rs"]
mod tests;
