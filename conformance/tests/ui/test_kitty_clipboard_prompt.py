"""The prompt a program's Kitty clipboard protocol (OSC 5522) request raises under `clipboard-read` or
`clipboard-write = ask`, and an allowed write: a Rust-only extension (capability `kitty-clipboard`,
conformance/docs/rust-build.md > Rust-only extensions).

The pane's program is the probe of agterm_conformance/kitty_clipboard.py. The prompt, with its preview of the data a
write would put on the clipboard or a read would disclose, takes the keyboard as it opens: Escape answers it with
Deny, Return with Allow, its default. A key pressed before it opens reaches the probe, which echoes it and goes on
reading. Two ways to see what happened:

- the reply, which the probe echoes where it reads OSC answers (`osc-answers`, not Windows): a request is answered
  only once its prompt is; a read reads what a write put on the selection clipboard, so it needs `private-primary`
  too;
- the selection clipboard, where it is the instance's own (`private-primary`, not macOS): an allowed write's text
  pasted back into the probe with Ghostty's `paste_from_selection`, an asked write's only once its prompt allows it.

Only the selection clipboard (`loc=primary`) is written, so with `private-primary` the desktop clipboard never is; the
protocol layer's headless compositor has no primary selection."""

import re
import time
import uuid

import pytest
from agterm_conformance.ghostty_config import configured
from agterm_conformance.kitty_clipboard import (
    PASTE_CHORD,
    PASTE_SELECTION,
    joined_text,
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
OSC_ANSWERS = pytest.mark.requires(
    "osc-answers", reason="the program reads its reply, an OSC answer, which Windows' pseudoconsole drops"
)
PRIVATE_PRIMARY = pytest.mark.requires(
    "private-primary", reason="a program's primary-selection write would reach the system pasteboard on macOS"
)
LIVE_IN_ANY_SHELL = pytest.mark.requires(
    "live-sessions",
    "live-any-shell",
    any=True,
    reason="needs panes in live sessions, which upstream creates only for a zsh or fish login shell",
)
pytestmark = [KITTY_CLIPBOARD]
PANES = pytest.mark.parametrize("live", [False, pytest.param(True, marks=LIVE_IN_ANY_SHELL)], ids=["plain", "live"])

PRESS_EVERY = 0.5
# Long enough for a request answered without its prompt to show its answer.
UNANSWERED_FOR = 1.5


def _launch(launch, live):
    return launch(settings={"restoreMode": "live"} if live else None)


def _pressing(ui, chords, done):
    """`chords` pressed every `PRESS_EVERY` until `done` holds."""
    pressed = -PRESS_EVERY

    def check():
        nonlocal pressed
        if time.monotonic() - pressed >= PRESS_EVERY:
            ui.press(*chords)
            pressed = time.monotonic()
        return done()

    wait_until(check)


def _unanswered(what, done, press=lambda: None):
    """`done` false for `UNANSWERED_FOR` from the request on, `press` run every `PRESS_EVERY` meanwhile."""
    pressed = -PRESS_EVERY
    until = time.monotonic() + UNANSWERED_FOR
    while time.monotonic() < until:
        if time.monotonic() - pressed >= PRESS_EVERY:
            press()
            pressed = time.monotonic()
        assert not done(), f"{what} before its prompt was answered"
        time.sleep(0.1)


def _written(ctl, directory, text, live):
    """`text` written to the selection clipboard by a program, its write allowed, as its reply says."""
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

    _unanswered(f"the {kind} was answered", answered)
    _pressing(ui, ["escape"], answered)
    wait_reply(ctl, sid, f"type={kind}:status=EPERM")


@OSC_ANSWERS
@PANES
def test_an_asked_write_waits_for_the_prompt_and_escape_denies_it(launch, ui_for, tmp_path, live):
    ctl = _launch(launch, live)
    ui = ui_for(ctl)
    with configured(ctl, "clipboard-write = ask\n"):
        sid = requesting(ctl, tmp_path, write_transaction(f"kitty-{uuid.uuid4().hex[:8]}", "primary"), live)
        try:
            _denied_at_the_prompt(ctl, ui, sid, "write")
        finally:
            ctl.request("session.close", sid)


@OSC_ANSWERS
@PRIVATE_PRIMARY
@PANES
def test_an_asked_read_waits_for_the_prompt_and_escape_denies_it(launch, ui_for, tmp_path, live):
    ctl = _launch(launch, live)
    ui = ui_for(ctl)
    with configured(ctl, "clipboard-write = allow\nclipboard-read = ask\n"):
        _written(ctl, tmp_path, f"kitty-{uuid.uuid4().hex[:8]}", live)
        reader = requesting(ctl, tmp_path, read_request("text/plain", "text/html", location="primary"), live)
        try:
            _denied_at_the_prompt(ctl, ui, reader, "read")
        finally:
            ctl.request("session.close", reader)


@OSC_ANSWERS
@PRIVATE_PRIMARY
@PANES
def test_an_allowed_write_reads_back_from_the_selection_clipboard(launch, tmp_path, live):
    ctl = _launch(launch, live)
    text = f"kitty-{uuid.uuid4().hex[:8]}"
    with configured(ctl, "clipboard-write = allow\nclipboard-read = allow\n"):
        _written(ctl, tmp_path, text, live)
        reader = requesting(ctl, tmp_path, read_request("text/plain", location="primary"), live)
        try:
            wait_echoed(ctl, reader, payload(text))
        finally:
            ctl.request("session.close", reader)


@PRIVATE_PRIMARY
@PANES
def test_an_allowed_write_pastes_back_from_the_selection_clipboard(launch, ui_for, tmp_path, live):
    ctl = _launch(launch, live)
    ui = ui_for(ctl)
    text = f"kitty-{uuid.uuid4().hex[:8]}"
    with configured(ctl, "clipboard-write = allow\n" + PASTE_SELECTION):
        sid = requesting(ctl, tmp_path, write_transaction(text, "primary"), live)
        try:
            _pressing(ui, [PASTE_CHORD], lambda: text in joined_text(ctl, sid))
        finally:
            ctl.request("session.close", sid)


@PRIVATE_PRIMARY
@PANES
def test_an_asked_write_lands_once_return_allows_its_prompt(launch, ui_for, tmp_path, live):
    ctl = _launch(launch, live)
    ui = ui_for(ctl)
    text = f"kitty-{uuid.uuid4().hex[:8]}"
    with configured(ctl, "clipboard-write = ask\n" + PASTE_SELECTION):
        sid = requesting(ctl, tmp_path, write_transaction(text, "primary"), live)
        try:

            def landed():
                return text in joined_text(ctl, sid)

            _unanswered("the write landed", landed, lambda: ui.press(PASTE_CHORD))
            _pressing(ui, ["return", PASTE_CHORD], landed)
        finally:
            ctl.request("session.close", sid)
