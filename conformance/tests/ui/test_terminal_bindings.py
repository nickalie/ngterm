"""Ghostty's terminal-scoped binding actions, bound by `keybind` lines in the agterm-scoped `<state>/config/ghostty.conf`
and fired by their chords pressed into the selected pane: `clear_screen`, `reset`, `select_all`, `adjust_selection`,
`text`, the viewport's scrolls and the `write_*_file:paste` actions (Ghostty's `Surface.performBindingAction` and
`Termio.clearScreen`; upstream embeds the Ghostty surface, so its chords reach them as the Rust build's reach
`ngterm-term`'s). And the mouse wheel over the pane, which scrolls the same viewport (Ghostty's `scrollCallback`):
the pane under the pointer, not the focused one, with keys typed just before too, and never the terminal a page covers.

The pane runs `cat`, so no shell prompt redraws itself: its text is the program's input echoed and the program's
output. The text goes in through `session.type`, the chords through the OS's keyboard input. Each test binds the chords
for its own run (`config.reload`) and puts the instance's config back; Ctrl+Shift with a letter neither the keymap's
Linux and Windows defaults nor Ghostty's take. `session.text` reads the viewport, scrolled or not, and with `all` the
whole screen, scrollback first."""

import contextlib
import re
from pathlib import Path

import pytest
from agterm_conformance.ghostty_config import configured
from agterm_conformance.match import assert_ok, find_session
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
    with configured(ctl, BINDS), _cat(ctl) as sid:
        yield sid


def _line(ctl, sid, text):
    """`text` and Return typed into `cat`: the terminal echoes the line and `cat` prints it again."""
    assert_ok(ctl.request("session.type", sid, text=text + "\n"))
    ctl.wait_text(sid, rf"{re.escape(text)}\s+{re.escape(text)}", all=True)


def _text(ctl, sid, **pane):
    return assert_ok(ctl.request("session.text", sid, all=True, **pane))["text"]


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


def _screen(ctl, sid, **pane):
    """The viewport's lines; `pane="right"` reads a split's other pane."""
    return [line.rstrip() for line in assert_ok(ctl.request("session.text", sid, **pane))["text"].split("\n")]


def _all_lines(ctl, sid, **pane):
    return [line.rstrip() for line in _text(ctl, sid, **pane).split("\n")]


def _visible_chars(text):
    return len("".join(text.split()))


def _scrollback(ctl, sid, **pane):
    """`ROWS` numbered lines typed into `cat` at once, each echoed and printed again: a screen many viewports tall.
    The line discipline interleaves the echo with `cat`'s copies at any byte, splitting a line of either across
    rows, so no line is sure to show whole twice: what is waited on is the screen holding every character of both
    copies, for up to 30 s, as the instrumented build (`--coverage`) echoes the 400 lines slower than the default
    wait."""
    lines = [f"row-{index:03}" for index in range(ROWS)]
    expected = _visible_chars(_text(ctl, sid, **pane)) + 2 * _visible_chars("".join(lines))
    assert_ok(ctl.request("session.type", sid, text="".join(f"{line}\n" for line in lines), **pane))

    def echoed():
        text = _text(ctl, sid, **pane)
        seen = _visible_chars(text)
        assert seen == expected, f"{seen} of {expected} characters shown, the text ends {text[-80:]!r}"
        return True

    wait_until(echoed, timeout=30)


def _shows(ctl, sid, top):
    """Until the viewport shows the screen's lines from line `top` on."""

    def shown():
        screen, every = _screen(ctl, sid), _all_lines(ctl, sid)
        assert screen == every[top : top + len(screen)], f"viewport {screen[:3]}, screen from {top}: {every[top : top + 3]}"
        return True

    shown.__qualname__ = f"the viewport from line {top}"
    wait_until(shown)


def _without_trailing_blanks(lines):
    end = len(lines)
    while end and not lines[end - 1]:
        end -= 1
    return lines[:end]


def _rows_below_the_viewport(ctl, sid, **pane):
    """How many of the screen's rows lie below the viewport, 0 at the bottom. The viewport is placed by where its
    lines sit in the screen, not by one line's text: the echo and `cat`'s copy repeat each line, so the last line can
    also show some rows higher up. Its text is matched without its trailing blank rows, which the whole screen also
    has in other places, and those rows are then counted back in, so a viewport one row short of the cursor's blank
    row is not the bottom."""
    viewport, every = _screen(ctl, sid, **pane), _all_lines(ctl, sid, **pane)
    screen = _without_trailing_blanks(viewport)
    assert screen, "the viewport shows no text"
    ends = [end for end in range(len(screen), len(every) + 1) if every[end - len(screen) : end] == screen]
    assert ends, f"the viewport {screen[:3]}…{screen[-3:]} is no run of the screen's lines"
    return len(every) - ends[-1] - (len(viewport) - len(screen))


def _at_bottom(ctl, sid, **pane):
    def bottom():
        below = _rows_below_the_viewport(ctl, sid, **pane)
        assert below == 0, f"{below} lines below the viewport"
        return True

    bottom.__qualname__ = "the viewport at the bottom"
    wait_until(bottom)


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_text_types_its_decoded_bytes_into_the_program(ctl, ui):
    # `text:` takes a Zig string literal: `\x41` is `A`, `\r` the Return `cat` reads a line at
    with configured(ctl, f"keybind = {CLEAR}=text:bound\\x41text\\r\n"), _cat(ctl) as sid:
        ui.press(CLEAR)
        ctl.wait_text(sid, r"boundAtext\s+boundAtext", all=True)


@pytest.mark.covers("session.type", "session.text", "config.reload", "behavior:pane.binding-actions")
def test_adjust_selection_moves_the_selections_end(ctl, ui):
    binds = (
        f"keybind = {SELECT_ALL}=select_all\n"
        f"keybind = {CLEAR}=adjust_selection:up\n"
        f"keybind = {WRITE_SELECTION}=write_selection_file:paste\n"
    )
    with configured(ctl, binds), _cat(ctl) as sid:
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
    with configured(ctl, binds), _cat(ctl) as sid:
        _scrollback(ctl, sid)
        ui.press(top)
        _shows(ctl, sid, 0)
        ui.press(lines)
        _shows(ctl, sid, 2)
        ui.press(bottom)
        _at_bottom(ctl, sid)
        # the selection starts on the screen's first written line: scrolling to it shows that line on top
        first = next(index for index, line in enumerate(_all_lines(ctl, sid)) if line)
        ui.press(select, to_selection)
        _shows(ctl, sid, first)


@pytest.mark.covers("session.type", "session.text", "behavior:pane.wheel-scroll")
def test_the_wheel_scrolls_the_viewport(ctl, ui):
    with _cat(ctl) as sid:
        _scrollback(ctl, sid)
        _at_bottom(ctl, sid)
        # the window's middle is over the selected session's pane, the sidebar on its left
        ui.scroll(ui.window(), -5)
        _scrolled_up(ctl, sid)
        ui.scroll(ui.window(), 50)
        _at_bottom(ctl, sid)


def _scrolled_up(ctl, sid, **pane):
    def scrolled_up():
        below = _rows_below_the_viewport(ctl, sid, **pane)
        assert below > 0, "the viewport is still at the bottom"
        return True

    scrolled_up.__qualname__ = "the viewport scrolled up"
    wait_until(scrolled_up)


def _typed_with_the_pointer_resting(ctl, ui, sid, **pane):
    """The pointer brought over the window's middle, then keys pressed into the focused pane, as a keyboard's (not
    `type_text`, whose Windows text the pane takes as pasted): the window's last input is a key, and a wheel turned at
    the window's middle next comes with no pointer move."""
    ui.move(ui.window())
    ui.press(*"typed")
    ctl.wait_text(sid, r"typed", **pane)


def _runs_cat(argv):
    """Whether a pane's foreground argv is `cat`'s: a shell's `exec` names its program by path (Windows: Git's
    `cat.exe`)."""
    return bool(argv) and re.fullmatch(r"(?:.*[\\/])?cat(?:\.exe)?", argv[0]) is not None


@pytest.mark.covers("session.type", "session.text", "behavior:pane.wheel-scroll")
def test_the_wheel_scrolls_after_typing_with_the_pointer_still(ctl, ui):
    with _cat(ctl) as sid:
        _scrollback(ctl, sid)
        _at_bottom(ctl, sid)
        _typed_with_the_pointer_resting(ctl, ui, sid)
        ui.scroll(ui.window(), -5)
        _scrolled_up(ctl, sid)


@pytest.mark.covers(
    "session.split",
    "session.focus",
    "session.resize",
    "session.type",
    "session.text",
    "field:tree.session.splitForeground",
    "behavior:pane.wheel-scroll",
)
def test_the_wheel_scrolls_the_split_pane_under_the_pointer_not_the_focused_one(ctl, ui):
    """Both panes of a side-by-side split hold a screen many viewports tall; keys go to the right one, and the wheel
    over the left one scrolls the left one alone."""
    with _cat(ctl) as sid:
        assert_ok(ctl.request("session.split", sid, mode="on", axis="vertical"))
        ctl.wait_mounted(sid, "left", "right")
        # the split's new pane takes the focus, which a request with no pane addresses
        _scrollback(ctl, sid, pane="left")
        # the new pane's shell hands it to `cat` as well, so no prompt redraws there
        assert_ok(ctl.request("session.type", sid, pane="right", text="exec cat\n"))
        ctl.wait_tree(lambda tree: _runs_cat(find_session(tree, sid).get("splitForeground")))
        assert_ok(ctl.request("session.type", sid, pane="right", text="cat-reads\n"))
        ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True, pane="right")
        _scrollback(ctl, sid, pane="right")
        # the sidebar, narrower than half the window, on its left: at 0.8 the window's middle is over the left pane
        assert_ok(ctl.request("session.resize", sid, ratio=0.8))
        ctl.wait_tree(lambda tree: find_session(tree, sid).get("splitRatio") == pytest.approx(0.8, abs=0.004))
        assert_ok(ctl.request("session.focus", sid, pane="right"))
        ctl.wait_tree(lambda tree: find_session(tree, sid).get("splitFocused") is True)
        _at_bottom(ctl, sid, pane="left")
        _at_bottom(ctl, sid, pane="right")
        _typed_with_the_pointer_resting(ctl, ui, sid, pane="right")
        ui.scroll(ui.window(), -5)
        _scrolled_up(ctl, sid, pane="left")
        below = _rows_below_the_viewport(ctl, sid, pane="right")
        assert below == 0, f"the focused pane scrolled too: {below} rows below its viewport"


# a page taller than its view, scrolled down a way once loaded: its title says when a key reaches it and when a wheel
# turned up over it moves it back
WHEEL_PAGE = """<!doctype html><title>loading</title><body style="height: 6000px; margin: 0">
<script>
addEventListener("load", () => {
  scrollTo(0, 2000);
  addEventListener("keydown", () => { document.title = "page-typed"; });
  addEventListener("scroll", () => { if (scrollY < 2000) document.title = "page-scrolled"; });
  document.title = "page-ready";
});
</script>"""


def _page_title(ctl, sid, title):
    def titled(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page.get("title") == title, page
        return True

    ctl.wait_tree(titled)


@pytest.mark.covers("session.overlay.open", "session.text", "behavior:pane.wheel-scroll")
def test_the_wheel_over_a_page_scrolls_the_page_not_its_sessions_terminal(ctl, ui):
    page = ctl.instance.root / "wheel-page.html"
    page.write_text(WHEEL_PAGE, encoding="utf-8")
    with _cat(ctl) as sid:
        _scrollback(ctl, sid)
        _at_bottom(ctl, sid)
        assert_ok(ctl.request("session.overlay.open", sid, html=str(page), javascript=True))
        try:
            _page_title(ctl, sid, "page-ready")
            view = ui.find("htmlOverlay.page")
            # keys typed with the pointer resting over the page, the wheel turned there next
            ui.move(view)
            ui.press(*"typed")
            _page_title(ctl, sid, "page-typed")
            ui.scroll(view, -5)
            _page_title(ctl, sid, "page-scrolled")
            below = _rows_below_the_viewport(ctl, sid)
            assert below == 0, f"the terminal of the session the page covers scrolled: {below} rows below its viewport"
        finally:
            ctl.request("session.overlay.close", sid)
