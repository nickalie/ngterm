"""The quick terminal's panel on macOS, as the user meets it. The in-app chord (ctrl+`, through the OS's keyboard
input) shows it without a title bar: its AX window has none of the title bar's buttons, also when it is zoomed. It
floats over normal windows at the floating level, not over the floating panels of other apps, and it takes the
keyboard: the keys typed next go to its terminal. It also shows over the main window in native full screen. Upstream's
`QuickTerminalPanel` is `[.borderless, .nonactivatingPanel]` at the floating level, joins all Spaces as a full screen
auxiliary window and can become key. Its frame is
upstream's `targetFrame`: the panel size of its screen's visible frame (the menu bar and Dock left out), centered in
it, and the whole visible frame while zoomed. A full screen Space hides the Dock, so over a full screen window the
visible frame is the screen below the menu bar."""

import math
import re
import sys

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import WaitTimeout, wait_until

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="the panel's title bar buttons as macOS AX has them")

BUTTONS = ("AXCloseButton", "AXMinimizeButton", "AXZoomButton", "AXFullScreenButton")
MAX_DEPTH = 40
# `QuickTerminalMetrics`' built-in size: a share of the visible frame, at most this size
DEFAULT_SHARE = 0.9
DEFAULT_MAX_SIZE = (1100.0, 700.0)
# upstream frames a fractional size and AppKit puts its edges on whole points; ngterm rounds the size, then
# centers it on whole points: the origin is up to one point off the unrounded frame
ORIGIN_SLACK = 1.0
# the float error of the share, so that a whole size stays whole
EPSILON = 1e-6


def _holds_quick(ui, handle, depth=0):
    if str(ui.attribute(handle, "AXIdentifier") or "") == "quick-terminal":
        return True
    return depth < MAX_DEPTH and any(
        _holds_quick(ui, child, depth + 1) for child in ui.attribute(handle, "AXChildren") or ()
    )


def _frame(ui, window):
    """The AX window's frame: x, y from the primary screen's top left, width, height."""
    ok, origin = ui.ax.AXValueGetValue(ui.attribute(window, "AXPosition"), ui.ax.kAXValueCGPointType, None)
    assert ok, "the panel's AX window has no position"
    ok, size = ui.ax.AXValueGetValue(ui.attribute(window, "AXSize"), ui.ax.kAXValueCGSizeType, None)
    assert ok, "the panel's AX window has no size"
    return float(origin.x), float(origin.y), float(size.width), float(size.height)


def _flipped(rect, primary_height):
    """An AppKit rect (bottom-left origin) as x, y from the primary screen's top left, width, height."""
    return (
        float(rect.origin.x),
        float(primary_height - rect.origin.y - rect.size.height),
        float(rect.size.width),
        float(rect.size.height),
    )


def _visible_frame(frame, full_screen=False):
    """The visible frame of the screen that holds the center of `frame`, as `_frame` gives it. In a full screen Space
    it is the screen below the menu bar: AppKit in this process still gives the desktop's, with the Dock."""
    import AppKit

    screens = AppKit.NSScreen.screens()
    primary_height = screens[0].frame().size.height
    x, y = frame[0] + frame[2] / 2, frame[1] + frame[3] / 2
    for screen in screens:
        left, top, width, height = _flipped(screen.frame(), primary_height)
        if left <= x < left + width and top <= y < top + height:
            visible = _flipped(screen.visibleFrame(), primary_height)
            return (left, visible[1], width, top + height - visible[1]) if full_screen else visible
    raise AssertionError(f"no screen holds the panel's frame {frame}")


def _target_frame(visible):
    """Upstream's `targetFrame` unzoomed: the built-in panel size of the visible frame, centered in it."""
    left, top, width, height = visible
    panel = [min(extent * DEFAULT_SHARE, cap) for extent, cap in zip((width, height), DEFAULT_MAX_SIZE)]
    return left + (width - panel[0]) / 2, top + (height - panel[1]) / 2, *panel


def _sized(got, want):
    """`got` is the whole point below or above a fractional `want`, or `want` itself when it is whole."""
    return math.floor(want + EPSILON) - EPSILON <= got <= math.ceil(want - EPSILON) + EPSILON


def _framed(zoomed, full_screen=False):
    """A check that the panel's frame is the visible frame (zoomed), else the target frame in it: what is wrong, or
    None."""

    def check(frame):
        visible = _visible_frame(frame, full_screen)
        want = visible if zoomed else _target_frame(visible)
        origin = all(abs(got - expected) <= ORIGIN_SLACK for got, expected in zip(frame[:2], want[:2]))
        if origin and all(_sized(got, expected) for got, expected in zip(frame[2:], want[2:])):
            return None
        return f"the panel's frame is {frame} in the visible frame {visible}"

    return check


def _quick_windows(ui):
    """The frame and the title bar buttons of each AX window that holds the quick terminal."""
    return [
        (_frame(ui, window), [name for name in BUTTONS if ui.attribute(window, name) is not None])
        for window in ui.attribute(ui.app, "AXWindows") or ()
        if _holds_quick(ui, window)
    ]


def _layers(ui, pid, frame):
    """The window server layers of the on-screen windows of `pid` whose bounds are `frame`, as `_frame` gives it."""
    q = ui.quartz
    windows = q.CGWindowListCopyWindowInfo(
        q.kCGWindowListOptionOnScreenOnly | q.kCGWindowListExcludeDesktopElements, q.kCGNullWindowID
    )
    return [
        int(window[q.kCGWindowLayer])
        for window in windows or ()
        if int(window[q.kCGWindowOwnerPID]) == pid
        and tuple(float(window[q.kCGWindowBounds][key]) for key in ("X", "Y", "Width", "Height")) == frame
    ]


def _borderless(ui, pid, what, check):
    """The shown panel: one AX window with no title bar buttons, whose frame passes `check`, and over normal windows
    one window at its frame, at the floating level. A main window at the zoomed panel's frame stays at layer 0."""

    def panel():
        windows = _quick_windows(ui)
        assert len(windows) == 1, f"{what}: the quick terminal's windows are {windows}"
        ((frame, buttons),) = windows
        assert buttons == [], f"{what}: the panel has the title bar buttons {buttons}"
        problem = check(frame)
        assert problem is None, f"{what}: {problem}"
        layers = _layers(ui, pid, frame)
        floating = int(ui.quartz.CGWindowLevelForKey(ui.quartz.kCGFloatingWindowLevelKey))
        assert [layer for layer in layers if layer > 0] == [floating], (
            f"{what}: the layers of the windows at the panel's frame are {layers}, not one at the floating level "
            f"{floating}"
        )
        return frame

    panel.__qualname__ = f"the borderless panel {what}"
    return wait_until(panel)


def _types_into_it(ctl, ui, marker):
    """A line typed key by key, as a keyboard sends it, runs in the quick terminal."""
    for char in f"echo {marker}":
        ui.press("space" if char == " " else char)
    ui.press("return")
    _ran(ctl, marker)


def _ran(ctl, marker):
    def ran():
        text = assert_ok(ctl.request("quick.text", all=True))["text"]
        assert re.search(rf"^{marker}\s*$", text, re.MULTILINE), text[-400:]
        return True

    ran.__qualname__ = f"{marker} printed in the quick terminal"
    wait_until(ran)


@pytest.mark.covers("ui:quick-terminal", "quick.text", "surface.zoom")
def test_the_quick_panel_has_no_title_bar_and_takes_the_keyboard(app, ctl, ui):
    pid = ctl.instance.pid
    ui.activate()
    try:
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        _borderless(ui, pid, "shown", _framed(zoomed=False))
        _types_into_it(ctl, ui, "quick-has-keys-1")

        assert_ok(ctl.request("surface.zoom", "quick", mode="show"))
        _borderless(ui, pid, "zoomed", _framed(zoomed=True))
        _types_into_it(ctl, ui, "quick-has-keys-2")
        assert_ok(ctl.request("surface.zoom", "quick", mode="hide"))
        _borderless(ui, pid, "unzoomed", _framed(zoomed=False))

        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is False)
        wait_until(lambda: _quick_windows(ui) == [])
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        _borderless(ui, pid, "shown again", _framed(zoomed=False))
        _types_into_it(ctl, ui, "quick-has-keys-3")
    finally:
        ctl.request("surface.zoom", "quick", mode="hide")
        ctl.request("quick", mode="hide")


def _fullscreen(ctl):
    return [bool(window.get("fullscreen")) for window in assert_ok(ctl.request("window.list"))["windows"]]


def _set_fullscreen(ctl, on):
    """Toggle the main window's native full screen until `window.list` reads `on`, sending a toggle that changed
    nothing in 5s again: macOS drops a full screen toggle now and then."""
    for attempt in range(3):
        if _fullscreen(ctl) == [on]:
            return
        ctl.request("window.fullscreen")
        try:
            wait_until(lambda: _fullscreen(ctl) == [on], timeout=5.0)
            return
        except WaitTimeout:
            if attempt == 2:
                raise


def _full_screen_settled(ui):
    """The app is frontmost and its focused AX window is the main window in full screen, spanning its screen: the Space
    animation is over and the keys go to it. A screen with a camera housing keeps the strip of the menu bar above a
    full screen window. An activation sent during the Space animation can be lost, so it is sent again. Returns the
    main window's frame, as `_frame` gives it."""
    import AppKit

    def settled():
        if not ui.attribute(ui.app, "AXFrontmost"):
            ui.activate()
            raise AssertionError("the app is not frontmost")
        window = ui.attribute(ui.app, "AXFocusedWindow")
        assert window is not None, "no focused window"
        assert ui.attribute(window, "AXFullScreen"), "the focused window is not in full screen"
        x, y, width, height = _frame(ui, window)
        primary_height = AppKit.NSScreen.screens()[0].frame().size.height
        screens = [_flipped(screen.frame(), primary_height) for screen in AppKit.NSScreen.screens()]
        assert any(
            (x, width, y + height) == (left, extent, top + tall) and y >= top for left, top, extent, tall in screens
        ), f"the focused window's frame {(x, y, width, height)} does not span a screen of {screens}"
        return x, y, width, height

    settled.__qualname__ = "the main window settled in full screen"
    return wait_until(settled)


@pytest.mark.covers("ui:quick-terminal", "quick.text", "window.fullscreen")
def test_the_quick_panel_shows_over_a_full_screen_window(app, ctl, ui):
    pid = ctl.instance.pid
    ui.activate()
    try:
        _set_fullscreen(ctl, True)
        main = _full_screen_settled(ui)
        # the first key after the move to the full screen Space can be lost, upstream's and ngterm's alike
        for attempt in range(3):
            ui.press("ctrl+`")
            try:
                ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True, timeout=3.0)
                break
            except WaitTimeout:
                if attempt == 2:
                    raise
        _borderless(ui, pid, "shown over the full screen window", _framed(zoomed=False, full_screen=True))

        def main_on_screen():
            layers = _layers(ui, pid, main)
            assert 0 in layers, f"the full screen main window {main} is not on screen under the panel: {layers}"
            return True

        main_on_screen.__qualname__ = "the full screen main window on screen under the panel"
        wait_until(main_on_screen)
        _types_into_it(ctl, ui, "quick-over-full-screen")
    finally:
        ctl.request("quick", mode="hide")
        _set_fullscreen(ctl, False)


# Text expanders and password managers type text as key events that carry it as a Unicode string (key code 0, the A
# key): letters of other scripts, CJK and an emoji.
UNICODE_TEXT = "über-жук-你好-🙂"


@pytest.mark.covers("ui:quick-terminal", "quick.text")
def test_the_quick_panel_takes_text_typed_as_unicode_key_events(app, ctl, ui):
    ui.activate()
    try:
        ui.press("ctrl+`")
        ctl.wait_tree(lambda tree: tree["tree"]["quickVisible"] is True)
        _borderless(ui, ctl.instance.pid, "shown", _framed(zoomed=False))
        ui.type_text(f"echo {UNICODE_TEXT}")
        ui.press("return")
        _ran(ctl, re.escape(UNICODE_TEXT))
    finally:
        ctl.request("quick", mode="hide")
