"""A second client of one of the Rust build's live sessions: the local stand-in for a viewer attached over ssh.

The sessions run in one session server per instance (`ngterm-session`), so the viewer is `ngterm-session attach`,
started as `ngterm_core::remote::ssh::attach_command` starts the viewer's on the origin, minus ssh: the attach
contract's variables (`NGTERM_LIVE_*`, empty meaning unset), explicit leadership claimed, and the create-only guard
after the session's name. On Windows, which has no pty to give it, it runs on pipes, as `attach` reads a piped input
there (`crates/ngterm-session/src/attach_tty/windows.rs`) and detaches at its end.
"""

from __future__ import annotations

import os
import subprocess
import sys
import threading
import uuid
from collections.abc import Mapping, Sequence
from pathlib import Path
from types import TracebackType
from typing import TYPE_CHECKING, Any, Protocol, Self

from agterm_conformance import llvm_profiles
from agterm_conformance.impls.base import MINIMAL_PATH

if TYPE_CHECKING:
    from agterm_conformance.pty_process import PtyProcess

# `ngterm_core::remote::ssh`'s `GONE_GUARD`: the viewer's program when the session is gone, in the Rust build's name.
GONE_GUARD = "printf '%s\\n' 'ngterm: remote session is gone'; exit 1"
# The same guard in Windows' own command interpreter, which every Windows has.
WINDOWS_GONE_GUARD = ("cmd.exe", "/d", "/c", "echo ngterm: remote session is gone& exit /b 1")
STOP_TIMEOUT = 5.0
_KEPT_OUTPUT = 65536


class LiveInstance(Protocol):
    """What a leading client reads of the instance whose session it attaches."""

    live_dir: str
    home: Path
    env: dict[str, str]


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
                del self._output[:-_KEPT_OUTPUT]

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


def leading_client(instance: LiveInstance, endpoint: Mapping[str, Any], session: str) -> PtyProcess | PipedProcess:
    """`ngterm-session attach <session>` claiming the lead, so the pane's own client in `instance` follows.

    `endpoint` is the `live.tree` read-back; one naming another instance's namespace is refused.
    """
    executable, directory = endpoint["executable"], endpoint["socketDirectory"]
    if directory != instance.live_dir:
        raise ValueError(f"endpoint socket directory {directory!r} is not this instance's {instance.live_dir!r}")
    if not os.path.isabs(executable):
        raise ValueError(f"endpoint executable {executable!r} is not absolute")
    env = {
        # Windows: the instance's own profile directories and system variables, without its app's variables.
        **({k: v for k, v in instance.env.items() if not k.startswith("NGTERM_")} if sys.platform == "win32" else {}),
        "PATH": MINIMAL_PATH,
        "HOME": os.fspath(instance.home),
        "TERM": "xterm-256color",
        # The variables the origin's own pane sets (`ngterm_core::live::support`), empty meaning unset.
        "NGTERM_LIVE_SESSION": "",
        "NGTERM_LIVE_SESSION_PREFIX": "",
        "NGTERM_LIVE_NO_DETACH_KEY": "1",
        "NGTERM_LIVE_DIR": directory,
        "NGTERM_LIVE_MANAGED": uuid.uuid4().hex,
        "NGTERM_LIVE_MANAGED_CLAIM": "1",
    }
    # the run's coverage profile, as every process of the build under test gets (`llvm_profiles`)
    llvm_profiles.apply(env)
    if sys.platform == "win32":
        return PipedProcess([executable, "attach", session, *WINDOWS_GONE_GUARD], env)
    # Late: POSIX-only, and this module loads on Windows too.
    from agterm_conformance.pty_process import PtyProcess

    return PtyProcess([executable, "attach", session, "/bin/sh", "-c", GONE_GUARD], env)
