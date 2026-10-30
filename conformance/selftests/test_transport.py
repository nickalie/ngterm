"""The control socket's transport: the Windows pipe's name a socket path stands for (everywhere), and on Windows the
pipe itself (the client's waits, timeouts and end-of-stream; the listener the fakes serve on)."""

import sys
import threading
import time
from pathlib import Path

import pytest
from agterm_conformance import transport
from agterm_conformance.client import Client, ConnectFailed

windows_only = pytest.mark.skipif(sys.platform != "win32", reason="the named pipe is Windows'")


def test_the_pipe_name_is_the_hash_of_the_folded_path() -> None:
    # crates/ngterm-core/src/paths.rs: pipe_name's test
    name = transport.pipe_name("p-", r"C:\Users\Nick\AppData\Local\ngterm\ngterm.sock")
    assert name == "p-" + transport.fnv1a64(rb"c:\users\nick\appdata\local\ngterm\ngterm.sock")
    assert name != transport.pipe_name("p-", r"C:\Users\Nick\AppData\Local\ngterm\other.sock")


def test_every_spelling_of_one_path_names_one_pipe() -> None:
    plain = transport.pipe_name(transport.CONTROL_PIPE_PREFIX, r"C:\a\ngterm\ngterm.sock")
    for spelling in [r"C:\A\NGTERM\ngterm.sock", "c:/a/ngterm/ngterm.sock", "C:\\a\\ngterm\\ngterm.sock\\"]:
        assert transport.pipe_name(transport.CONTROL_PIPE_PREFIX, spelling) == plain, spelling
    assert plain.startswith("ngterm-control-") and len(plain) == len("ngterm-control-") + 16


def test_the_fnv_hash_is_upstreams() -> None:
    # selftests/test_vocabulary.py pins the same value for the live-session namespace
    assert transport.fnv1a64(b"/nonexistent-agterm/state") == "22c4fb0b9e3e04dc"


@windows_only
def test_a_relative_socket_path_names_the_pipe_of_its_absolute_form(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.chdir(tmp_path)
    assert transport.control_pipe_name("ngterm.sock") == transport.control_pipe_name(tmp_path / "ngterm.sock")
    assert transport.control_pipe_path("x.sock").startswith("\\\\.\\pipe\\ngterm-control-")


@windows_only
def test_nothing_is_created_at_the_socket_path(tmp_path: Path) -> None:
    listener = transport.Listener(tmp_path / "s.sock")
    try:
        assert not (tmp_path / "s.sock").exists()
    finally:
        listener.close()


@windows_only
def test_a_second_listener_on_one_path_is_refused(tmp_path: Path) -> None:
    listener = transport.Listener(tmp_path / "s.sock")
    try:
        with pytest.raises(OSError):
            transport.Listener(tmp_path / "s.sock")
    finally:
        listener.close()


@windows_only
def test_a_missing_pipe_fails_at_once(tmp_path: Path) -> None:
    started = time.monotonic()
    with pytest.raises(FileNotFoundError):
        transport.connect(tmp_path / "none.sock", 5.0)
    assert time.monotonic() - started < 2
    with pytest.raises(ConnectFailed):
        Client(tmp_path / "none.sock", tmp_path).request("version")


@windows_only
def test_a_busy_pipe_is_waited_for_until_the_server_accepts_again(tmp_path: Path) -> None:
    listener = transport.Listener(tmp_path / "s.sock")
    served: list[bytes] = []

    def serve(count: int) -> None:
        for _ in range(count):
            with listener.accept() as conn:
                served.append(conn.recv(100))
                conn.sendall(b"ok\n")

    thread = threading.Thread(target=serve, args=(3,), daemon=True)
    thread.start()
    try:
        for index in range(3):
            with transport.connect(tmp_path / "s.sock", 5.0) as conn:
                conn.sendall(b"%d\n" % index)
                assert conn.recv(100) == b"ok\n"
        thread.join(5)
        assert served == [b"0\n", b"1\n", b"2\n"]
    finally:
        listener.close()


@windows_only
def test_a_read_times_out_and_a_closed_server_reads_as_the_end(tmp_path: Path) -> None:
    listener = transport.Listener(tmp_path / "s.sock")
    release = threading.Event()

    def serve() -> None:
        with listener.accept() as conn:
            release.wait(5)
            conn.sendall(b"last\n")

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    try:
        with transport.connect(tmp_path / "s.sock", 0.3) as conn:
            with pytest.raises(TimeoutError):
                conn.recv(100)
            conn.settimeout(5)
            release.set()
            thread.join(5)
            # what the server wrote before it closed is still read, then the end
            assert conn.recv(100) == b"last\n"
            assert conn.recv(100) == b""
            with pytest.raises(BrokenPipeError):
                conn.sendall(b"x" * 10)
    finally:
        release.set()
        listener.close()


@windows_only
def test_closing_the_listener_wakes_its_accept(tmp_path: Path) -> None:
    listener = transport.Listener(tmp_path / "s.sock")
    failed: list[BaseException] = []

    def wait() -> None:
        try:
            listener.accept()
        except OSError as error:
            failed.append(error)

    thread = threading.Thread(target=wait, daemon=True)
    thread.start()
    time.sleep(0.2)
    listener.close()
    thread.join(5)
    assert not thread.is_alive() and failed
