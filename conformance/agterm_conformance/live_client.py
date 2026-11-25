"""A second client of one of the Rust build's live sessions: the local stand-in for a viewer attached over ssh.

The sessions run in one session server per instance (`ngterm-session`), so the viewer is `ngterm-session attach`,
started as `ngterm_core::remote::ssh::attach_command` starts the viewer's on the origin, minus ssh: the attach
contract's variables (`NGTERM_LIVE_*`, empty meaning unset), explicit leadership claimed, and the create-only guard
after the session's name. On Windows, which has no pty to give it, it runs on pipes, as `attach` reads a piped input
there (`crates/ngterm-session/src/attach_tty/windows.rs`) and detaches at its end.
"""

from __future__ import annotations

import os
import sys
import uuid
from collections.abc import Mapping
from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol

from agterm_conformance import llvm_profiles
from agterm_conformance.impls.base import MINIMAL_PATH
from agterm_conformance.piped_process import PipedProcess

if TYPE_CHECKING:
    from agterm_conformance.pty_process import PtyProcess

# `ngterm_core::remote::ssh`'s `GONE_GUARD`: the viewer's program when the session is gone, in the Rust build's name.
GONE_GUARD = "printf '%s\\n' 'ngterm: remote session is gone'; exit 1"
# The same guard in Windows' own command interpreter, which every Windows has.
WINDOWS_GONE_GUARD = ("cmd.exe", "/d", "/c", "echo ngterm: remote session is gone& exit /b 1")


class LiveInstance(Protocol):
    """What a leading client reads of the instance whose session it attaches."""

    live_dir: str
    home: Path
    env: dict[str, str]


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
