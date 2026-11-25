"""A program on pipes, its output drained: a live session's second client on Windows (`live_client`), which has no pty
to give it."""

from __future__ import annotations

import subprocess
import threading
from collections.abc import Mapping, Sequence
from types import TracebackType
from typing import Self

from agterm_conformance import llvm_profiles

# Shared with `pty_process`, which loads on POSIX only.
STOP_TIMEOUT = 5.0
KEPT_OUTPUT = 65536


class PipedProcess:
    """A program on pipes, its output drained so it never blocks writing; `stop` ends its input, which detaches an
    attach client so it writes its coverage profile, and reaps it."""

    def __init__(self, argv: Sequence[str], env: Mapping[str, str]) -> None:
        self.process = subprocess.Popen(
            list(argv), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, env=dict(env)
        )
        self._output = bytearray()
        self._lock = threading.Lock()
        self._drain = threading.Thread(target=self._read_all, daemon=True)
        self._drain.start()

    def _read_all(self) -> None:
        assert self.process.stdout is not None
        while chunk := self.process.stdout.read1(4096):
            with self._lock:
                self._output += chunk
                del self._output[:-KEPT_OUTPUT]

    def output(self) -> str:
        with self._lock:
            return self._output.decode(errors="replace")

    def stop(self, timeout: float = STOP_TIMEOUT) -> None:
        """Closes the program's input and waits for it to exit; ends it if it does not."""
        if self.process.stdin is not None and not self.process.stdin.closed:
            try:
                self.process.stdin.close()
            except OSError:  # the program already exited and its pipe broke
                pass
        try:
            self.process.wait(timeout)
        except subprocess.TimeoutExpired:
            terminated = llvm_profiles.terminate(self.process)
            self.process.wait(timeout)
            llvm_profiles.record_if_signalled(
                self.process.pid, self.process.args[0], self.process.returncode,
                "PipedProcess.stop: kept running after its input ended", terminated=terminated,
            )
        self._drain.join(timeout)
        if self.process.stdout is not None:
            self.process.stdout.close()

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self, kind: type[BaseException] | None, error: BaseException | None, trace: TracebackType | None
    ) -> None:
        self.stop()
