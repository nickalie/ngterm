"""What a control socket path stands for on each OS, and its connections.

The socket path is the identity on every OS (`NGTERM_CONTROL_SOCKET`, `--socket`); what serves it differs:

- POSIX (Linux, macOS): an `AF_UNIX` stream socket at the path.
- Windows: the Rust build's byte-mode named pipe `\\\\.\\pipe\\ngterm-control-<hash>` the path names
  (`ngterm_core::paths::control_pipe_name`, crates/ngterm-server/src/endpoint.rs): the FNV-1a 64 of the path made
  absolute, case-folded, with `\\` separators and no trailing one. Nothing exists at the path itself. Upstream has no
  Windows build, so the pipe's name is the Rust build's alone.

`connect` returns a connection with the part of a socket's surface the client uses (`settimeout`, `sendall`, `recv`,
`close`, a context manager), raising what a socket raises: `TimeoutError` past the timeout, `FileNotFoundError` (an
`OSError`) when nothing serves, `BrokenPipeError` on a write the server no longer reads, and `b""` from `recv` once the
server closed. `Listener` is the server side, for the self-tests' fakes.
"""

from __future__ import annotations

import ntpath
import os
import socket
import sys
import threading
import time
from types import TracebackType
from typing import Any, Protocol, Self

WINDOWS = sys.platform == "win32"

# `ngterm_core::paths::CONTROL_PIPE_PREFIX`.
CONTROL_PIPE_PREFIX = "ngterm-control-"
PIPE_ROOT = "\\\\.\\pipe\\"
# What each direction of a pipe buffers, as the Rust build's server does (`BUFFER_BYTES`).
PIPE_BUFFER_BYTES = 64 * 1024

# winerror.h
ERROR_FILE_NOT_FOUND = 2
ERROR_BROKEN_PIPE = 109
ERROR_SEM_TIMEOUT = 121
ERROR_PIPE_BUSY = 231
ERROR_NO_DATA = 232
ERROR_PIPE_NOT_CONNECTED = 233
ERROR_OPERATION_ABORTED = 995
ERROR_IO_PENDING = 997
WAIT_TIMEOUT = 0x102
INFINITE = 0xFFFFFFFF
# The pipe's end is gone: a read sees the end of the stream, a write a broken pipe.
PIPE_GONE = frozenset({ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED})
PIPE_REJECT_REMOTE_CLIENTS = 0x8


def fnv1a64(data: bytes) -> str:
    value = 14_695_981_039_346_656_037
    for byte in data:
        value = ((value ^ byte) * 1_099_511_628_211) & 0xFFFF_FFFF_FFFF_FFFF
    return f"{value:016x}"


def pipe_name(prefix: str, path: str) -> str:
    """`ngterm_core::paths::pipe_name`: `prefix` and the FNV-1a 64 of `path` case-folded, `/` as `\\`, with no trailing
    separator; `path` is taken as given."""
    folded = path.replace("/", "\\").lower().rstrip("\\")
    return prefix + fnv1a64(folded.encode())


def control_pipe_name(socket_path: str | os.PathLike[str]) -> str:
    """`ngterm_core::paths::control_pipe_name`: the pipe (without `\\\\.\\pipe\\`) the socket at `socket_path` is on
    Windows, the path made absolute first (`GetFullPathNameW`, as `std::path::absolute` does) where this is Windows."""
    path = os.fspath(socket_path)
    if WINDOWS and path:
        path = ntpath.abspath(path)
    return pipe_name(CONTROL_PIPE_PREFIX, path)


def control_pipe_path(socket_path: str | os.PathLike[str]) -> str:
    return PIPE_ROOT + control_pipe_name(socket_path)


class Connection(Protocol):
    def settimeout(self, timeout: float | None) -> None: ...
    def sendall(self, data: bytes) -> None: ...
    def recv(self, size: int) -> bytes: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(self, *exc: object) -> None: ...


def connect(socket_path: str | os.PathLike[str], timeout: float | None) -> Connection:
    """A connection to the server of `socket_path`; `timeout` bounds the connect and is the connection's timeout."""
    if WINDOWS:
        return PipeConnection.connect(control_pipe_path(socket_path), timeout)
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.settimeout(timeout)
        sock.connect(os.fspath(socket_path))
    except BaseException:
        sock.close()
        raise
    return sock


class PipeConnection:
    """One end of a byte-mode named pipe, opened for overlapped I/O so every call can time out (a pipe has no timeouts
    of its own): the call is cancelled past its timeout and `TimeoutError` raised."""

    def __init__(self, handle: int) -> None:
        self._handle: int | None = handle
        self._timeout: float | None = None
        self._lock = threading.Lock()

    @classmethod
    def connect(cls, name: str, timeout: float | None) -> PipeConnection:
        """Opens the client end of `name`; a busy pipe (every instance taken, the server between two accepts) is waited
        for until `timeout` runs out. A missing one raises `FileNotFoundError` at once."""
        import _winapi

        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            try:
                handle = _winapi.CreateFile(
                    name,
                    _winapi.GENERIC_READ | _winapi.GENERIC_WRITE,
                    0,
                    _winapi.NULL,
                    _winapi.OPEN_EXISTING,
                    _winapi.FILE_FLAG_OVERLAPPED,
                    _winapi.NULL,
                )
            except OSError as error:
                if error.winerror != ERROR_PIPE_BUSY:
                    raise
            else:
                connection = cls(handle)
                connection.settimeout(timeout)
                return connection
            remaining = None if deadline is None else deadline - time.monotonic()
            if remaining is not None and remaining <= 0:
                raise TimeoutError(f"{name} stayed busy")
            wait_ms = 1000 if remaining is None else max(1, min(1000, int(remaining * 1000)))
            try:
                _winapi.WaitNamedPipe(name, wait_ms)
            except OSError as error:
                # Timed out, or gone again between the two calls: the next CreateFile says which.
                if error.winerror not in (ERROR_SEM_TIMEOUT, ERROR_FILE_NOT_FOUND):
                    raise

    def settimeout(self, timeout: float | None) -> None:
        self._timeout = timeout

    def gettimeout(self) -> float | None:
        return self._timeout

    def _require(self) -> int:
        if self._handle is None:
            raise OSError("the pipe connection is closed")
        return self._handle

    def _finish(self, overlapped: Any, error: int) -> tuple[int, int]:
        """Waits for an overlapped call up to the timeout; cancels it past that. Returns its byte count and error."""
        import _winapi

        if error == ERROR_IO_PENDING:
            wait_ms = INFINITE if self._timeout is None else max(0, int(self._timeout * 1000))
            if _winapi.WaitForMultipleObjects([overlapped.event], False, wait_ms) == WAIT_TIMEOUT:
                overlapped.cancel()
                try:
                    overlapped.GetOverlappedResult(True)
                except OSError:
                    pass
                raise TimeoutError("timed out")
        return overlapped.GetOverlappedResult(True)

    def sendall(self, data: bytes) -> None:
        import _winapi

        view = memoryview(data)
        while view:
            try:
                overlapped, error = _winapi.WriteFile(self._require(), view, overlapped=True)
                written, error = self._finish(overlapped, error)
            except OSError as failure:
                if failure.winerror in PIPE_GONE:
                    raise BrokenPipeError(failure.errno, failure.strerror) from None
                raise
            if error in PIPE_GONE:
                raise BrokenPipeError("the pipe's reader is gone")
            if error:
                raise OSError(None, f"pipe write failed ({error})", None, error)
            view = view[written:]

    def recv(self, size: int) -> bytes:
        import _winapi

        try:
            overlapped, error = _winapi.ReadFile(self._require(), size, overlapped=True)
            _, error = self._finish(overlapped, error)
        except OSError as failure:
            if failure.winerror in PIPE_GONE:
                return b""
            raise
        if error in PIPE_GONE:
            return b""
        if error:
            raise OSError(None, f"pipe read failed ({error})", None, error)
        return bytes(overlapped.getbuffer())

    def shutdown(self, how: int = socket.SHUT_RDWR) -> None:
        """Disconnects a server's end (`how` is always both: a pipe has no half-close); a client's end has nothing to
        shut but its handle."""
        del how
        if self._handle is not None:
            try:
                _disconnect(self._handle)
            except OSError:
                pass

    def close(self) -> None:
        import _winapi

        with self._lock:
            handle, self._handle = self._handle, None
        if handle is not None:
            _winapi.CloseHandle(handle)

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self, kind: type[BaseException] | None, error: BaseException | None, trace: TracebackType | None
    ) -> None:
        self.close()


def _disconnect(handle: int) -> None:
    import ctypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    if not kernel32.DisconnectNamedPipe(ctypes.c_void_p(handle)):
        raise ctypes.WinError(ctypes.get_last_error())


class Listener:
    """The server side of a control socket path, for the self-tests' fakes: an `AF_UNIX` listener at the path, or on
    Windows the pipe it names, its first instance created with `FILE_FLAG_FIRST_PIPE_INSTANCE` (a name another
    process holds is refused), closed to remote clients. `accept` returns one connection; `close` wakes a waiting
    `accept`, which then raises `OSError`."""

    def __init__(self, socket_path: str | os.PathLike[str]) -> None:
        self.path = os.fspath(socket_path)
        self._closed = False
        self._pending: Any = None
        self._lock = threading.Lock()
        if WINDOWS:
            self.name = control_pipe_path(socket_path)
            self._next: int | None = self._instance(first=True)
            self._socket: socket.socket | None = None
        else:
            self.name = self.path
            self._next = None
            self._socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            self._socket.bind(self.path)
            self._socket.listen()

    def _instance(self, first: bool = False) -> int:
        import _winapi

        flags = _winapi.PIPE_ACCESS_DUPLEX | _winapi.FILE_FLAG_OVERLAPPED
        if first:
            flags |= _winapi.FILE_FLAG_FIRST_PIPE_INSTANCE
        return _winapi.CreateNamedPipe(
            self.name,
            flags,
            _winapi.PIPE_TYPE_BYTE | _winapi.PIPE_READMODE_BYTE | _winapi.PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            _winapi.PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            _winapi.NULL,
        )

    def accept(self) -> Connection:
        if self._socket is not None:
            connection, _ = self._socket.accept()
            return connection
        import _winapi

        with self._lock:
            if self._closed or self._next is None:
                raise OSError("the listener is closed")
            handle, self._next = self._next, self._instance()
        try:
            overlapped = _winapi.ConnectNamedPipe(handle, overlapped=True)
        except OSError as error:
            # A client that connected, wrote and left before the accept: its data is still there to read.
            if error.winerror != ERROR_NO_DATA:
                _winapi.CloseHandle(handle)
                raise
            return PipeConnection(handle)
        with self._lock:
            self._pending = overlapped
            closed = self._closed
        if closed:
            overlapped.cancel()
        _winapi.WaitForMultipleObjects([overlapped.event], False, INFINITE)
        with self._lock:
            self._pending = None
        try:
            _, error = overlapped.GetOverlappedResult(True)
        except OSError:
            _winapi.CloseHandle(handle)
            raise
        if error or self._closed:
            _winapi.CloseHandle(handle)
            raise OSError("the listener is closed")
        return PipeConnection(handle)

    def close(self) -> None:
        if self._socket is not None:
            # close() alone does not wake a blocked accept() on Linux.
            try:
                self._socket.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self._socket.close()
            return
        import _winapi

        with self._lock:
            self._closed = True
            pending, handle, self._next = self._pending, self._next, None
        if pending is not None:
            pending.cancel()
        if handle is not None:
            _winapi.CloseHandle(handle)
