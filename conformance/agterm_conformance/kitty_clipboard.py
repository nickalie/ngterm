"""A program's Kitty clipboard protocol (OSC 5522) requests, and the replies a pane echoes back (capability
`kitty-clipboard`).

The pane runs `cat` on a file holding the program's request, then a second `cat`: the terminal's reply arrives as that
`cat`'s input, which the terminal echoes (a control character as `^` and its letter), so `session.text` reads it. A
marker line ahead of the request shows the first `cat` has run."""

from __future__ import annotations

import base64
import contextlib
import re
import uuid
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.wait import wait_until

MARKER = "kitty-request"
# A 2x2 red PNG, one of the representations a write carries.
PNG = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167WwAAAABJRU5ErkJggg=="


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


def requesting(ctl: Any, directory: Path, request: str, live: bool = False) -> str:
    """A session whose program sends `request` after the marker line, then reads the reply as `cat`'s input."""
    path = directory / f"request-{uuid.uuid4().hex[:8]}.vt"
    path.write_bytes(f"{MARKER}\r\n{request}".encode())
    sid = assert_ok(ctl.request("session.new", command=f'cat "{path}" && cat'))["id"]
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


def wait_echoed(ctl: Any, sid: str, fragment: str) -> str:
    """The pane's whole text once it holds `fragment`, rows joined, as a long echoed reply wraps."""

    def joined() -> str | None:
        text = "".join(assert_ok(ctl.request("session.text", sid, all=True))["text"].splitlines())
        return text if fragment in text else None

    return wait_until(joined)
