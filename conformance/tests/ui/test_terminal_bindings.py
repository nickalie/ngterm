"""Ghostty's terminal-scoped binding actions, bound by `keybind` lines in the agterm-scoped `<state>/config/ghostty.conf`
and fired by their chords pressed into the selected pane: `clear_screen`, `reset`, `select_all`, `adjust_selection`,
`text`, the viewport's scrolls and the `write_*_file:paste` actions (Ghostty's `Surface.performBindingAction` and
`Termio.clearScreen`; upstream embeds the Ghostty surface, so its chords reach them as the Rust build's reach
`ngterm-term`'s). And the mouse wheel over the pane, which scrolls the same viewport (Ghostty's `scrollCallback`).

The pane runs `cat`, so no shell prompt redraws itself: its text is the program's input echoed and the program's
output. The text goes in through `session.type`, the chords through the OS's keyboard input. Each test binds the chords
for its own run (`config.reload`) and puts the instance's config back; Ctrl+Shift with a letter neither the keymap's
Linux and Windows defaults nor Ghostty's take. `session.text` reads the viewport, scrolled or not, and with `all` the
whole screen, scrollback first."""

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


@contextlib.contextmanager
def _bound(ctl, binds):
    """`binds` in force, the instance's config put back afterwards."""
    path = ctl.instance.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.rstrip("\n") + "\n" + binds, encoding="utf-8")
    try:
        assert assert_ok(ctl.request("config.reload")) == {"count": 0}
        yield
    finally:
        path.write_text(original, encoding="utf-8")
        ctl.request("config.reload")


@contextlib.contextmanager
def _cat(ctl):
    """A selected session running `cat`, closed afterwards: yielded once `cat` reads. The pane mounts before its
    program starts, and a line typed then is echoed ahead of whatever the pane prints first (the reference's
    `Last login` line, from `login` before it runs the command), with only `cat`'s copy after it; so a line typed
    until `cat` has printed it back is a readiness probe, not part of a test's text."""
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        assert_ok(ctl.request("session.type", sid, text="cat-reads\n"))
        ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True)
        yield sid
    finally:
        ctl.request("session.close", sid)


@pytest.fixture
def cat(ctl):
    """`cat` with the chords of `BINDS` bound."""
    with _bound(ctl, BINDS), _cat(ctl) as sid:
        yield sid


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


# -- the other actions, each test binding the same free chords to its own

ROWS = 200


def _screen(ctl, sid):
    """The viewport's lines."""
    return [line.rstrip() for line in assert_ok(ctl.request("session.text", sid))["text"].split("\n")]


def _all_lines(ctl, sid):
    return [line.rstrip() for line in _text(ctl, sid).split("\n")]


def _scrollback(ctl, sid):
    """`ROWS` numbered lines typed into `cat` at once, each echoed and printed again: a screen many viewports tall.
    The copies may interleave (the terminal echoes what `cat` has not read yet), so only the last line's count is
    waited on, for up to 30 s: the instrumented build (`--coverage`) echoes the 400 lines slower than the default
    wait. Returns that line."""
    lines = [f"row-{index:03}" for index in range(ROWS)]
    assert_ok(ctl.request("session.type", sid, text="".join(f"{line}\n" for line in lines)))

    def echoed():
        text = _text(ctl, sid)
        seen = text.count(lines[-1])
        assert seen == 2, f"{lines[-1]!r} seen {seen} times, the text ends {text[-80:]!r}"
        return True

    wait_until(echoed, timeout=30)
    return lines[-1]


def _shows(ctl, sid, top):
    """Until the viewport shows the screen's lines from line `top` on."""

    def shown():
        screen, every = _screen(ctl, sid), _all_lines(ctl, sid)
        assert screen == every[top : top + len(screen)], f"viewport {screen[:3]}, screen from {top}: {every[top : top + 3]}"
        return True

    shown.__qualname__ = f"the viewport from line {top}"
    wait_until(shown)


def _at_bottom(ctl, sid, last):
    def bottom():
        screen = _screen(ctl, sid)
        assert last in screen, f"{last!r} not in the viewport's {screen[-3:]}"
        return True

    bottom.__qualname__ = f"{last!r} in the viewport"
    wait_until(bottom)


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_text_types_its_decoded_bytes_into_the_program(ctl, ui):
    # `text:` takes a Zig string literal: `\x41` is `A`, `\r` the Return `cat` reads a line at
    with _bound(ctl, f"keybind = {CLEAR}=text:bound\\x41text\\r\n"), _cat(ctl) as sid:
        ui.press(CLEAR)
        ctl.wait_text(sid, r"boundAtext\s+boundAtext", all=True)


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_adjust_selection_moves_the_selections_end(ctl, ui):
    binds = (
        f"keybind = {SELECT_ALL}=select_all\n"
        f"keybind = {CLEAR}=adjust_selection:up\n"
        f"keybind = {WRITE_SELECTION}=write_selection_file:paste\n"
    )
    with _bound(ctl, binds), _cat(ctl) as sid:
        _line(ctl, sid, "kept-line")
        _line(ctl, sid, "last-line")
        # select_all ends on the last character written, `cat`'s copy of the last line; up moves that end a row up,
        # onto the echoed copy, so the selection keeps one of the two
        ui.press(SELECT_ALL, CLEAR, WRITE_SELECTION)
        written = _pasted_text(ctl, sid, "selection.txt")
        assert (written.count("kept-line"), written.count("last-line")) == (2, 1), written


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_scroll_actions_move_the_viewport(ctl, ui):
    top, lines, bottom, select, to_selection = CLEAR, RESET, WRITE_SCREEN, SELECT_ALL, WRITE_SELECTION
    binds = (
        f"keybind = {top}=scroll_to_top\n"
        f"keybind = {lines}=scroll_page_lines:2\n"
        f"keybind = {bottom}=scroll_to_bottom\n"
        f"keybind = {select}=select_all\n"
        f"keybind = {to_selection}=scroll_to_selection\n"
    )
    with _bound(ctl, binds), _cat(ctl) as sid:
        last = _scrollback(ctl, sid)
        ui.press(top)
        _shows(ctl, sid, 0)
        ui.press(lines)
        _shows(ctl, sid, 2)
        ui.press(bottom)
        _at_bottom(ctl, sid, last)
        # the selection starts on the screen's first written line: scrolling to it shows that line on top
        first = next(index for index, line in enumerate(_all_lines(ctl, sid)) if line)
        ui.press(select, to_selection)
        _shows(ctl, sid, first)


@pytest.mark.covers("session.type", "session.text", "behavior:pane.wheel-scroll")
def test_the_wheel_scrolls_the_viewport(ctl, ui):
    with _cat(ctl) as sid:
        last = _scrollback(ctl, sid)
        _at_bottom(ctl, sid, last)
        # the window's middle is over the selected session's pane, the sidebar on its left
        ui.scroll(ui.window(), -5)

        def scrolled_up():
            screen = _screen(ctl, sid)
            assert last not in screen, f"{last!r} still in the viewport's {screen[-3:]}"
            return True

        wait_until(scrolled_up)
        ui.scroll(ui.window(), 50)
        _at_bottom(ctl, sid, last)
