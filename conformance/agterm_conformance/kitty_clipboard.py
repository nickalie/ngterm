"""A program's Kitty clipboard protocol (OSC 5522) requests, and what a test reads of them (capability
`kitty-clipboard`).

The pane's program is a probe: the harness's own Python running `PROBE`, which prints a marker line, writes the
request (once a gate file exists, when given one), then echoes each line it reads. Where the program reads the
terminal's OSC answers (`osc-answers`), the reply arrives as the probe's input, which the terminal's line discipline
echoes (a control character as `^` and its letter), so `session.text` reads it. Where a write lands in a clipboard of
the instance's own (`private-primary`), Ghostty's `paste_from_selection` bound to a chord pastes it back into the
probe, whose input is echoed. The command line is the probe's words alone, unquoted, so every pane shell (`/bin/sh`,
`cmd.exe`, a Live pane's PowerShell, zsh, fish) runs it as written."""

from __future__ import annotations

import base64
import contextlib
import os
import re
import sys
import uuid
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.wait import wait_until

MARKER = "kitty-request"
# A 2x2 red PNG, one of the representations a write carries.
PNG = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167WwAAAABJRU5ErkJggg=="
# A chord neither the keymap's defaults nor Ghostty's take, bound to `paste_from_selection` by `PASTE_SELECTION`.
PASTE_CHORD = "ctrl+shift+k"
PASTE_SELECTION = f"keybind = {PASTE_CHORD}=paste_from_selection\n"
PROBE = f"""\
import os, sys, time

if os.name == "nt":
    import ctypes

    kernel32 = ctypes.windll.kernel32
    out = kernel32.GetStdHandle(-11)
    mode = ctypes.c_uint32()
    if kernel32.GetConsoleMode(out, ctypes.byref(mode)):
        kernel32.SetConsoleMode(out, mode.value | 0x1 | 0x4)
os.write(1, b"{MARKER}\\r\\n")
if len(sys.argv) > 2:
    while not os.path.exists(sys.argv[2]):
        time.sleep(0.05)
with open(sys.argv[1], "rb") as request:
    os.write(1, request.read())
for line in sys.stdin:
    os.write(1, line.encode())
"""


def _b64(text: str) -> str:
    return base64.b64encode(text.encode()).decode()


def _osc(body: str) -> str:
    return f"\x1b]5522;{body}\x1b\\"


def write_transaction(text: str, location: str | None = None) -> str:
    """One write of `text` as plain text, as HTML and with an image, ended by the empty `wdata`; `location` names the
    clipboard (`primary`: the selection clipboard), else the system clipboard."""
    return (
        _osc("type=write" + (f":loc={location}" if location else ""))
        + _osc(f"type=wdata:mime={_b64('text/plain')};{_b64(text)}")
        + _osc(f"type=wdata:mime={_b64('text/html')};{_b64(f'<b>{text}</b>')}")
        + _osc(f"type=wdata:mime={_b64('image/png')};{PNG}")
        + _osc("type=wdata")
    )


def read_request(*mimes: str, location: str | None = None) -> str:
    """A read of `mimes`; `.` asks for the listing of what the clipboard holds. `location` as in `write_transaction`."""
    return _osc("type=read" + (f":loc={location}" if location else "") + f";{_b64(' '.join(mimes))}")


@contextlib.contextmanager
def configured(ctl: Any, lines: str) -> Iterator[None]:
    """`lines` added to the instance's ghostty config, the config put back afterwards."""
    path = ctl.instance.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.rstrip("\n") + "\n" + lines, encoding="utf-8")
    try:
        assert assert_ok(ctl.request("config.reload")) == {"count": 0}
        yield
    finally:
        path.write_text(original, encoding="utf-8")
        ctl.request("config.reload")


def probe_command(directory: Path, request: str, gate: Path | None = None) -> str:
    """The command line of a probe sending `request` (after `gate` exists, when given)."""
    probe = directory / "kitty_probe.py"
    probe.write_text(PROBE, encoding="utf-8")
    path = directory / f"request-{uuid.uuid4().hex[:8]}.vt"
    path.write_bytes(request.encode())
    words = [sys.executable, os.fspath(probe), os.fspath(path), *([os.fspath(gate)] if gate else [])]
    if any(" " in word for word in words):
        raise ValueError(f"the probe's words go unquoted to every pane shell, so none may hold a space: {words}")
    return " ".join(words)


def requesting(ctl: Any, directory: Path, request: str, live: bool = False, gate: Path | None = None) -> str:
    """A session whose probe sends `request`, shown with its marker line."""
    sid = assert_ok(ctl.request("session.new", command=probe_command(directory, request, gate)))["id"]
    ctl.wait_mounted(sid, "left")
    if live:
        ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)
    ctl.wait_text(sid, MARKER, all=True)
    return sid


def reply_pattern(fields: str) -> str:
    """An echoed reply whose fields start with `fields`; group 1 is the rest of its fields."""
    return rf"\^\[\]5522;{re.escape(fields)}([^\s^]*)"


def wait_reply(ctl: Any, sid: str, fields: str) -> str:
    """The rest of the fields of the echoed reply that starts with `fields`, once the pane shows it."""
    return ctl.wait_text(sid, reply_pattern(fields), all=True).group(1)


def payload(text: str) -> str:
    """`text` as a reply carries it."""
    return _b64(text)


def joined_text(ctl: Any, sid: str) -> str:
    """The pane's whole text, its rows joined, as a long echoed reply or paste wraps."""
    return "".join(assert_ok(ctl.request("session.text", sid, all=True))["text"].splitlines())


def wait_echoed(ctl: Any, sid: str, fragment: str) -> str:
    """The pane's joined text once it holds `fragment`."""

    def holding() -> str | None:
        text = joined_text(ctl, sid)
        return text if fragment in text else None

    return wait_until(holding)
