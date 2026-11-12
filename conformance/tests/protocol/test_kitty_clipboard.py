"""A program's Kitty clipboard protocol (OSC 5522) requests under `clipboard-read` and `clipboard-write`: a Rust-only
extension (capability `kitty-clipboard`, conformance/docs/rust-build.md > Rust-only extensions).

The pane's probe echoes the reply its program gets (agterm_conformance/kitty_clipboard.py), so these tests need a
program that reads the terminal's OSC answers (`osc-answers`): Windows' pseudoconsole drops them on their way in. A
Live pane's request goes through the session server to the app as its bridge viewer and the reply back the same way.
Every request here is refused or reads nothing, so the desktop clipboard is never written; the protocol layer's
headless compositor has no primary selection, so an allowed write is the ui layer's
(tests/ui/test_kitty_clipboard_prompt.py)."""

import uuid

import pytest
from agterm_conformance.kitty_clipboard import configured, read_request, requesting, wait_reply, write_transaction
from agterm_conformance.match import assert_ok
from agterm_conformance.plugin import Ctl

KITTY_CLIPBOARD = pytest.mark.requires(
    "kitty-clipboard", reason="a Rust-only extension: the Kitty clipboard protocol of Ghostty 1.4, after upstream's pin"
)
OSC_ANSWERS = pytest.mark.requires(
    "osc-answers", reason="the program reads its reply, an OSC answer, which Windows' pseudoconsole drops"
)
LIVE_IN_ANY_SHELL = pytest.mark.requires(
    "live-sessions",
    "live-any-shell",
    any=True,
    reason="needs panes in live sessions, which upstream creates only for a zsh or fish login shell",
)
pytestmark = [KITTY_CLIPBOARD, OSC_ANSWERS]


@pytest.fixture(scope="module")
def live(adapter):
    instance = adapter.launch(settings={"restoreMode": "live"})
    try:
        ctl = Ctl(instance)
        assert assert_ok(ctl.request("restore.mode"))["restore"]["active"] == "live"
        yield ctl
    finally:
        adapter.stop(instance)


@pytest.fixture(params=["plain", pytest.param("live", marks=LIVE_IN_ANY_SHELL)])
def pane(request, adapter):
    """The instance, and whether its panes run in live sessions."""
    if request.param == "live":
        return request.getfixturevalue("live"), True
    return request.getfixturevalue("ctl"), False


def _replied(ctl, live, directory, config, request, fields):
    with configured(ctl, config):
        sid = requesting(ctl, directory, request, live)
        try:
            return wait_reply(ctl, sid, fields)
        finally:
            ctl.request("session.close", sid)


def test_a_denied_write_is_refused(pane, tmp_path):
    ctl, live = pane
    text = f"kitty-{uuid.uuid4().hex[:8]}"
    _replied(ctl, live, tmp_path, "clipboard-write = deny\n", write_transaction(text), "type=write:status=EPERM")


def test_a_denied_read_is_refused(pane, tmp_path):
    ctl, live = pane
    request = read_request("text/plain", "text/html")
    _replied(ctl, live, tmp_path, "clipboard-read = deny\n", request, "type=read:status=EPERM")


def test_a_listing_is_answered_without_a_prompt(pane, tmp_path):
    ctl, live = pane
    rest = _replied(ctl, live, tmp_path, "clipboard-read = ask\n", read_request("."), "type=read:status=")
    assert not rest.startswith("EPERM"), rest

