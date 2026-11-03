"""Typing into the selected pane through the OS's text input, read back from the terminal: its text and its cursor.

Upstream's UI tests type into the focused terminal with `typeText` (`SessionNavUITests`, `ControlSurfaceZoomUITests`
v0.39.1). The pane runs `cat` instead of a login shell (`session.new --command`), so no prompt of unknown length sits
before the typed text: the terminal echoes it from column 0, and the cursor's column (the protocol reports no row)
counts what was typed without wrapping."""

import re

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_cursor

# Case, digits and shifted symbols; nothing is sent before them, so no Return ever reaches `cat`.
FIRST = "Typed"
REST = "-Ab_C:1@2"


@pytest.mark.covers("session.text", "surface.cursor", "behavior:pane.typing")
def test_typed_text_reaches_the_selected_pane_and_moves_its_cursor(ctl, ui):
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        # no target: the active surface, the selected session's focused pane the keys go to
        wait_cursor(ctl, None, 0)
        ui.type_text(FIRST)
        ctl.wait_text(sid, re.escape(FIRST))
        wait_cursor(ctl, None, len(FIRST))
        ui.type_text(REST)
        ctl.wait_text(sid, re.escape(FIRST + REST))
        wait_cursor(ctl, None, len(FIRST + REST))
    finally:
        ctl.request("session.close", sid)
