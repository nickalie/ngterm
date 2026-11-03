"""A program on a pty of its own, its output drained: a live session's second client (`live_client`, and the Swift
reference's own in impls/swift_macos.py), the local stand-in for a viewer attached over ssh."""

from __future__ import annotations

import fcntl
import os
import struct
import subprocess
import termios
import threading
from collections.abc import Mapping, Sequence
from pathlib import Path
from types import TracebackType
from typing import Protocol, Self

from agterm_conformance import llvm_profiles

STOP_TIMEOUT = 5.0
_KEPT_OUTPUT = 65536


class LiveInstance(Protocol):
    """What a leading client reads of the instance whose session it attaches."""

    live_dir: str
    home: Path


class PtyProcess:
    """A program on a pty of its own, its output drained so it never blocks writing; `stop` ends and reaps it."""

    def __init__(self, argv: Sequence[str], env: Mapping[str, str], columns: int = 80, rows: int = 24) -> None:
        master, slave = os.openpty()
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
            self.process = subprocess.Popen(
                list(argv), stdin=slave, stdout=slave, stderr=slave, env=dict(env), start_new_session=True
            )
        except BaseException:
            os.close(master)
            raise
        finally:
            os.close(slave)
        self._master: int | None = master
        self._output = bytearray()
        self._lock = threading.Lock()
        self._drain = threading.Thread(target=self._read_all, args=(master,), daemon=True)
        self._drain.start()

    def _read_all(self, master: int) -> None:
        while True:
            try:
                chunk = os.read(master, 4096)
            except OSError:  # EIO on Linux once the program and its pty slave are gone
                return
            if not chunk:
                return
            with self._lock:
                self._output += chunk
                del self._output[:-_KEPT_OUTPUT]

    def output(self) -> str:
        with self._lock:
            return self._output.decode(errors="replace")

    def stop(self, timeout: float = STOP_TIMEOUT) -> None:
        """SIGTERM this program's own pid, SIGKILL it if it ignores that, then close the pty."""
        if self.process.poll() is None:
            terminated = llvm_profiles.terminate(self.process)
            try:
                self.process.wait(timeout)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout)
            llvm_profiles.record_if_signalled(
                self.process.pid, self.process.args[0], self.process.returncode, "PtyProcess.stop: ended before it exited",
                terminated=terminated,
            )
        self._drain.join(timeout)
        if self._master is not None:
            os.close(self._master)
            self._master = None

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self, kind: type[BaseException] | None, error: BaseException | None, trace: TracebackType | None
    ) -> None:
        self.stop()

