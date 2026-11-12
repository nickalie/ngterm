"""The prompt a program's Kitty clipboard protocol (OSC 5522) request raises under `clipboard-read` or
`clipboard-write = ask`: a Rust-only extension (capability `kitty-clipboard`, conformance/docs/rust-build.md >
Rust-only extensions).

Each pane echoes the reply its program gets (agterm_conformance/kitty_clipboard.py). The prompt, with its preview of
the data a write would put on the clipboard or a read would disclose, takes the keyboard as it opens, and Escape
answers it with Deny, so the program hears EPERM only once the prompt is answered; an Escape pressed before it opens
reaches the pane's `cat`, which echoes it and goes on reading. Only the selection clipboard (`loc=primary`) is
written, which is the instance's own on every OS (Linux's primary selection on the instance's private display, an
app-wide buffer on Windows, a pasteboard named for the instance's app id on macOS), so the desktop clipboard never is;
the protocol layer's headless compositor offers no primary selection."""

import re
import time
import uuid

import pytest
from agterm_conformance.kitty_clipboard import (
    configured,
    payload,
    read_request,
    reply_pattern,
    requesting,
    wait_echoed,
    wait_reply,
    write_transaction,
)
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

KITTY_CLIPBOARD = pytest.mark.requires(
    "kitty-clipboard", reason="a Rust-only extension: the Kitty clipboard protocol of Ghostty 1.4, after upstream's pin"
)
LIVE_IN_ANY_SHELL = pytest.mark.requires(
    "live-sessions",
    "live-any-shell",
    any=True,
    reason="needs panes in live sessions, which upstream creates only for a zsh or fish login shell",
)
pytestmark = [KITTY_CLIPBOARD]
PANES = pytest.mark.parametrize("live", [False, pytest.param(True, marks=LIVE_IN_ANY_SHELL)], ids=["plain", "live"])

ESCAPE_EVERY = 0.5


def _written(ctl, directory, text, live):
    """`text` written to the selection clipboard by a program, its write allowed."""
    writer = requesting(ctl, directory, write_transaction(text, "primary"), live)
    try:
        wait_reply(ctl, writer, "type=write:status=DONE")
    finally:
        ctl.request("session.close", writer)


def _denied_at_the_prompt(ctl, ui, sid, kind):
    """The request of `kind` (`read`, `write`) of `sid` unanswered until Escape answers its prompt, then EPERM."""
    any_reply = re.compile(reply_pattern(f"type={kind}:status="))

    def answered():
        return any_reply.search(assert_ok(ctl.request("session.text", sid, all=True))["text"]) is not None

    assert not answered(), f"the {kind} was answered before its prompt"
    pressed = -ESCAPE_EVERY

    def escape_answers():
        nonlocal pressed
        if time.monotonic() - pressed >= ESCAPE_EVERY:
            ui.press("escape")
            pressed = time.monotonic()
        return answered()

    wait_until(escape_answers)
    wait_reply(ctl, sid, f"type={kind}:status=EPERM")


@PANES
def test_an_asked_write_waits_for_the_prompt_and_escape_denies_it(launch, ui_for, tmp_path, live):
    ctl = launch(settings={"restoreMode": "live"} if live else None)
    ui = ui_for(ctl)
    with configured(ctl, "clipboard-write = ask\n"):
        sid = requesting(ctl, tmp_path, write_transaction(f"kitty-{uuid.uuid4().hex[:8]}", "primary"), live)
        try:
            _denied_at_the_prompt(ctl, ui, sid, "write")
        finally:
            ctl.request("session.close", sid)


@PANES
def test_an_asked_read_waits_for_the_prompt_and_escape_denies_it(launch, ui_for, tmp_path, live):
    ctl = launch(settings={"restoreMode": "live"} if live else None)
    ui = ui_for(ctl)
    with configured(ctl, "clipboard-write = allow\nclipboard-read = ask\n"):
        _written(ctl, tmp_path, f"kitty-{uuid.uuid4().hex[:8]}", live)
        reader = requesting(ctl, tmp_path, read_request("text/plain", "text/html", location="primary"), live)
        try:
            _denied_at_the_prompt(ctl, ui, reader, "read")
        finally:
            ctl.request("session.close", reader)


@PANES
def test_an_allowed_write_reads_back_from_the_selection_clipboard(launch, tmp_path, live):
    ctl = launch(settings={"restoreMode": "live"} if live else None)
    text = f"kitty-{uuid.uuid4().hex[:8]}"
    with configured(ctl, "clipboard-write = allow\nclipboard-read = allow\n"):
        _written(ctl, tmp_path, text, live)
        reader = requesting(ctl, tmp_path, read_request("text/plain", location="primary"), live)
        try:
            wait_echoed(ctl, reader, payload(text))
        finally:
            ctl.request("session.close", reader)
