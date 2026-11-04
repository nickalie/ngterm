"""Ghostty's terminal-scoped binding actions, bound by `keybind` lines in the agterm-scoped `<state>/config/ghostty.conf`
and fired by their chords pressed into the selected pane: `clear_screen`, `reset`, `select_all` and the
`write_*_file:paste` actions (Ghostty's `Surface.performBindingAction` and `Termio.clearScreen`; upstream embeds the
Ghostty surface, so its chords reach them as the Rust build's reach `ngterm-term`'s).

The pane runs `cat`, so no shell prompt redraws itself: its text is the program's input echoed and the program's
output. The text goes in through `session.type`, the chords through the OS's keyboard input. Each test binds the chords
for its own run (`config.reload`) and puts the instance's config back; Ctrl+Shift with a letter neither the keymap's
Linux and Windows defaults nor Ghostty's take."""

import contextlib
import re
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

CLEAR = "ctrl+shift+k"
RESET = "ctrl+shift+u"
WRITE_SCREEN = "ctrl+shift+l"
SELECT_ALL = "ctrl+shift+y"
WRITE_SELECTION = "ctrl+shift+x"
BINDS = (
    f"keybind = {CLEAR}=clear_screen\n"
    f"keybind = {RESET}=reset\n"
    f"keybind = {WRITE_SCREEN}=write_screen_file:paste\n"
    f"keybind = {SELECT_ALL}=select_all\n"
    f"keybind = {WRITE_SELECTION}=write_selection_file:paste\n"
)


@pytest.fixture
def bound(ctl):
    """The chords bound for this test, the instance's config put back afterwards."""
    path = ctl.instance.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.rstrip("\n") + "\n" + BINDS, encoding="utf-8")
    try:
        assert assert_ok(ctl.request("config.reload")) == {"count": 0}
        yield
    finally:
        path.write_text(original, encoding="utf-8")
        ctl.request("config.reload")


@pytest.fixture
def cat(ctl, bound):
    """A selected session running `cat`, closed afterwards."""
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        yield sid
    finally:
        ctl.request("session.close", sid)


def _line(ctl, sid, text):
    """`text` and Return typed into `cat`: the terminal echoes the line and `cat` prints it again."""
    assert_ok(ctl.request("session.type", sid, text=text + "\n"))
    ctl.wait_text(sid, rf"{re.escape(text)}\s+{re.escape(text)}", all=True)


def _text(ctl, sid):
    return assert_ok(ctl.request("session.text", sid, all=True))["text"]


def _gone(ctl, sid, text):
    def gone():
        current = _text(ctl, sid)
        assert text not in current, f"{text!r} still in {current!r}"
        return True

    gone.__qualname__ = f"{text!r} gone from {sid}"
    wait_until(gone)


def _pasted_text(ctl, sid, name):
    """The text of the file whose path a `write_*_file:paste` action typed into the pane; its directory, the action's
    own temporary one, is removed with it (an `rmdir`: only once empty). `cat` waits on an empty input line, so the
    pasted path is that whole line, spaces in the temporary directory's name included."""
    pattern = re.compile(rf"^[ \t]*(\S.*?{re.escape(name)})[ \t]*$", re.MULTILINE)
    path = Path(ctl.wait_text(sid, pattern, all=True).group(1))
    try:
        return wait_until(lambda: path.is_file() and path.read_text(encoding="utf-8"))
    finally:
        with contextlib.suppress(OSError):
            path.unlink()
            path.parent.rmdir()


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_clear_screen_drops_the_rows_above_the_cursor(ctl, ui, cat):
    _line(ctl, cat, "cleared-line")
    ui.press(CLEAR)
    _gone(ctl, cat, "cleared-line")
    # the program keeps its terminal: what it prints next is there
    _line(ctl, cat, "after-clear")


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_reset_clears_the_terminal_and_keeps_the_program(ctl, ui, cat):
    _line(ctl, cat, "before-reset")
    ui.press(RESET)
    _gone(ctl, cat, "before-reset")
    _line(ctl, cat, "after-reset")


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_write_screen_file_pastes_the_path_of_the_screens_text(ctl, ui, cat):
    _line(ctl, cat, "on-the-screen")
    ui.press(WRITE_SCREEN)
    written = _pasted_text(ctl, cat, "screen.txt")
    # the echoed line and `cat`'s copy of it
    assert written.count("on-the-screen") == 2, written


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_write_selection_file_pastes_the_path_of_the_selected_text(ctl, ui, cat):
    _line(ctl, cat, "all-selected")
    ui.press(SELECT_ALL, WRITE_SELECTION)
    written = _pasted_text(ctl, cat, "selection.txt")
    assert written.count("all-selected") == 2, written
