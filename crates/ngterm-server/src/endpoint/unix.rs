//! The endpoint on Unix: an `AF_UNIX` stream socket at the path.

use std::fs::{self, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

/// One control connection, either end.
pub type Connection = UnixStream;

pub(super) type Listener = UnixListener;

/// Replaces whatever is at `socket` (a stale socket of a dead instance; only the lock holder gets here) with a fresh
/// listener only the owner can connect to.
pub(super) fn bind(socket: &Path) -> io::Result<Listener> {
    if let Err(error) = fs::remove_file(socket)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::debug!(%error, socket = %socket.display(), "stale control socket not removed");
    }
    let listener = UnixListener::bind(socket)?;
    // another local user must not drive this terminal over the control channel
    fs::set_permissions(socket, Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub(super) fn accept(listener: &Listener) -> io::Result<Connection> {
    listener.accept().map(|(stream, _)| stream)
}

/// A client's connection: on macOS its writes to a server that hung up fail with `EPIPE` rather than raising
/// `SIGPIPE`, which `ngtermctl` keeps at its default action.
pub(super) fn connect(socket: &Path) -> io::Result<Connection> {
    let stream = UnixStream::connect(socket)?;
    #[cfg(target_vendor = "apple")]
    no_sigpipe(&stream)?;
    Ok(stream)
}

pub(super) fn connect_unchecked(socket: &Path) -> io::Result<Connection> {
    UnixStream::connect(socket)
}

pub(super) fn pair() -> io::Result<(Connection, Connection)> {
    UnixStream::pair()
}

pub(super) fn remove(socket: &Path) {
    if let Err(error) = fs::remove_file(socket)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(%error, "control socket not removed");
    }
}

/// Keeps a write to a peer that hung up from raising `SIGPIPE` on macOS whatever the flags it is sent with
/// (`SO_NOSIGPIPE`, upstream's guard); Linux has no such option and relies on `MSG_NOSIGNAL` (the stream's `send_all`).
#[cfg(target_vendor = "apple")]
pub(crate) fn no_sigpipe(socket: &Connection) -> io::Result<()> {
    use std::os::fd::AsRawFd as _;
    let on: libc::c_int = 1;
    let len = libc::socklen_t::try_from(size_of::<libc::c_int>()).map_err(io::Error::other)?;
    // SAFETY: the descriptor is the socket's, borrowed for the call; `on` is a live `c_int` of the length passed.
    let set = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&raw const on).cast::<libc::c_void>(),
            len,
        )
    };
    if set == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
