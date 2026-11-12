"""What the `shell-profiles` tests share (tests/protocol and tests/ui): the profiles they seed and the read-back of the
profile a pane's shell was started with.

The profiles run the panes' POSIX `sh` (`posix_shell`: `/bin/sh`, Git for Windows' `sh.exe` on Windows) and differ only
in the value of `QA_SHELL` they set, so a line typed into a pane tells which profile, if any, started its shell."""

from __future__ import annotations

import itertools
from typing import Any

import pytest

from agterm_conformance.impls.base import posix_shell
from agterm_conformance.wait import wait_until

SHELL_PROFILES = pytest.mark.requires(
    "shell-profiles",
    reason="a Rust-only extension: upstream has no shell profiles, its panes run the login shell or ghostty's command",
)
ALPHA, BETA = "alpha", "beta"
# What `QA_SHELL` reads in a shell no profile started.
NO_PROFILE = "none"
# How long a pane's shell gets to start and print its read-back.
SHELL_START = 60.0

_markers = itertools.count(1)


def profile(name: str, executable: str | None = None, **extra: Any) -> dict[str, Any]:
    """A settings-file profile `qa-<name>` named `QA <Name>`, running `executable` (the panes' `sh`) with `QA_SHELL`
    set to `name`; `extra` replaces or adds fields (`args`, `env`, `startingDirectory`)."""
    return {
        "id": f"qa-{name}",
        "name": f"QA {name.title()}",
        "executable": executable or posix_shell(),
        "args": [],
        "env": {"QA_SHELL": name},
        **extra,
    }


def settings(default: dict[str, Any] | None = None, *more: dict[str, Any], **extra: Any) -> dict[str, Any]:
    """Seeded settings: the alpha and beta profiles (and `more`) in `shellProfiles`, `default` as the default."""
    seeded: dict[str, Any] = {"shellProfiles": [profile(ALPHA), profile(BETA), *more], **extra}
    if default is not None:
        seeded["defaultShellProfile"] = default
    return seeded


def shell_of(ctl: Any, sid: str, **pane: Any) -> str:
    """The `QA_SHELL` of the shell in `sid` (its `pane=` one), `none` where it is unset. Typed until the shell prints
    it, since a line typed before a starting shell reads keys can be lost; its halves are joined so the echo never
    matches."""
    marker = f"QS{next(_markers):04d}X"
    line = f"printf '%s%s|%s|\\n' {marker[:3]} {marker[3:]} \"${{QA_SHELL-{NO_PROFILE}}}\"\n"

    def printed() -> str | None:
        if not ctl.request("session.type", sid, text=line, **pane).ok:
            return None
        try:
            return ctl.wait_text(sid, rf"(?m)^{marker}\|([^|\n]*)\|\s*$", timeout=2, **pane).group(1)
        except AssertionError:
            return None

    return wait_until(printed, timeout=SHELL_START)
