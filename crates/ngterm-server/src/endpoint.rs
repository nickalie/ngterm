//! The control socket's endpoint: what a socket path stands for on each OS, its listener and its connections.
//!
//! The socket path is the identity everywhere (`NGTERM_CONTROL_SOCKET`, `ngtermctl --socket`, `NGTERM_SOCKET`, the
//! `<socket>.unavailable` marker); what serves it differs:
//!
//! - Unix (Linux, macOS): an `AF_UNIX` stream socket at the path, mode 0600. A file left there by an instance that
//!   died is stale: only the holder of the socket's ownership lock binds, and it removes whatever is at the path
//!   first. [`Connection`] is std's [`UnixStream`](std::os::unix::net::UnixStream).
//! - Windows: the named pipe `\\.\pipe\`[`control_pipe_name`](ngterm_core::paths::control_pipe_name)`(path)`, through
//!   `interprocess`, as ngterm-session's namespace pipe is: created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, owner-only
//!   (SYSTEM and the owner, at the server's integrity level: [`ngterm_core::pipe_security`]), closed to remote
//!   clients, and refused by [`connect`] when another user's process serves it. Nothing is created at the path itself,
//!   and nothing is stale: a pipe ends with its server's last handle, so a crashed instance leaves no endpoint
//!   behind. A pipe name some other process holds fails the bind with [`io::ErrorKind::AddrInUse`].
//!   [`Connection`] is a pipe stream with std's socket surface: read and write timeouts (named pipes have none, so a
//!   timer cancels the call past its timeout), duplicates, and `shutdown(Both)`, which disconnects a server's end.
//!   There is no half-close.

use std::io;
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;

#[cfg(windows)]
mod watchdog;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

pub use sys::Connection;

/// A bound endpoint, accepting connections.
pub(crate) struct Listener(sys::Listener);

impl Listener {
    /// Waits for the next client.
    pub(crate) fn accept(&self) -> io::Result<Connection> {
        sys::accept(&self.0)
    }
}

/// Takes the endpoint of `socket` for the server that owns it: on Unix it replaces whatever is at the path with a
/// fresh socket only the owner may connect to, on Windows it creates the pipe's first instance.
pub(crate) fn bind(socket: &Path) -> io::Result<Listener> {
    sys::bind(socket).map(Listener)
}

/// Connects to the server of `socket`, as `ngtermctl` does.
///
/// # Errors
///
/// No server listens ([`io::ErrorKind::NotFound`], or [`io::ErrorKind::ConnectionRefused`] for a stale Unix socket);
/// on Windows a pipe served by another user's process ([`io::ErrorKind::PermissionDenied`]), or one whose server's
/// user cannot be read.
pub fn connect(socket: &Path) -> io::Result<Connection> {
    sys::connect(socket)
}

/// Wakes the accept thread blocked on `socket` with a connection of its own, which the caller keeps open until that
/// thread is done: a pipe client gone before its accept completes is discarded (interprocess clears such
/// dead-on-arrival connections and waits for the next client), so a dropped one would wake nothing.
pub(crate) fn wake(socket: &Path) -> io::Result<Connection> {
    sys::connect_unchecked(socket)
}

/// Removes what the stopped server leaves at `socket`: the socket file on Unix, nothing on Windows.
pub(crate) fn remove(socket: &Path) {
    sys::remove(socket);
}
