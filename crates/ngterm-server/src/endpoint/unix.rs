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

pub(super) fn connect(socket: &Path) -> io::Result<Connection> {
    UnixStream::connect(socket)
}

pub(super) fn connect_unchecked(socket: &Path) -> io::Result<Connection> {
    UnixStream::connect(socket)
}

pub(super) fn remove(socket: &Path) {
    if let Err(error) = fs::remove_file(socket)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(%error, "control socket not removed");
    }
}
