"""UIA mappings and SendInput structures: no comtypes, no Windows."""

import ctypes
import subprocess
import sys

import pytest
from agterm_conformance.drivers import uia
from agterm_conformance.drivers.base import NAMED_KEYS, PRINTABLE_KEYS, ROLES, Chord, DriverError


def test_lazy_import():
    result = subprocess.run(
        [sys.executable, "-c",
         "import sys; import agterm_conformance.drivers.uia; assert 'comtypes' not in sys.modules"],
        capture_output=True, text=True,
    )  # fmt: skip
    assert result.returncode == 0, result.stderr


def test_without_windows_or_comtypes_is_a_driver_error(monkeypatch):
    monkeypatch.setattr(sys, "platform", "linux")
    with pytest.raises(DriverError, match="runs on Windows"):
        uia.UiaDriver(1)
    monkeypatch.setattr(sys, "platform", "win32")
    monkeypatch.setitem(sys.modules, "comtypes", None)
    with pytest.raises(DriverError, match="needs comtypes"):
        uia.UiaDriver(1)


def test_control_types_map_to_portable_roles():
    assert set(uia.CONTROL_TYPES.values()) <= ROLES
    assert uia.CONTROL_TYPES[50000] == "button"
    assert uia.CONTROL_TYPES[50002] == "checkbox"
    assert uia.CONTROL_TYPES[50004] == "text-field"
    assert uia.CONTROL_TYPES[50011] == "menu-item"
    assert uia.CONTROL_TYPES[50020] == "text"
    assert uia.CONTROL_TYPES[50032] == "window"


def _props(**named):
    ids = {
        "control_type": uia.CONTROL_TYPE, "name": uia.NAME, "id": uia.AUTOMATION_ID, "enabled": uia.IS_ENABLED,
        "value_available": uia.IS_VALUE_AVAILABLE, "value": uia.VALUE_VALUE, "read_only": uia.VALUE_IS_READ_ONLY,
        "toggle_available": uia.IS_TOGGLE_AVAILABLE, "toggle": uia.TOGGLE_STATE,
        "selection_available": uia.IS_SELECTION_ITEM_AVAILABLE, "selected": uia.SELECTION_ITEM_IS_SELECTED,
        "range_available": uia.IS_RANGE_VALUE_AVAILABLE, "range": uia.RANGE_VALUE_VALUE,
        "expand_available": uia.IS_EXPAND_COLLAPSE_AVAILABLE, "description": uia.FULL_DESCRIPTION,
    }  # fmt: skip
    return {ids[key]: value for key, value in named.items()}


@pytest.mark.parametrize(
    ("props", "expected"),
    [
        (_props(control_type=50000, id="add", name="Add", enabled=True), ("add", "button", "Add", None, True, None)),
        (_props(control_type=50000, id="menu", expand_available=True), ("menu", "menu-button", None, None, None, None)),
        (_props(control_type=50000, id="split-toggle", value_available=True, value="none", enabled=False),
         ("split-toggle", "button", None, "none", False, None)),
        (_props(control_type=50002, id="c", toggle_available=True, toggle=1), ("c", "checkbox", None, 1, None, None)),
        (_props(control_type=50002, id="c", toggle_available=True, toggle=0), ("c", "checkbox", None, 0, None, None)),
        (_props(control_type=50013, id="r", selection_available=True, selected=True),
         ("r", "radio", None, 1, None, None)),
        (_props(control_type=50004, id="f", value_available=True, value="typed"),
         ("f", "text-field", None, "typed", None, None)),
        (_props(control_type=50004, id="f", value_available=True, value="path", read_only=True),
         ("f", "text", None, "path", None, None)),
        (_props(control_type=50020, id="t", name="Hello"), ("t", "text", "Hello", "Hello", None, None)),
        (_props(control_type=50020, name="agterm", description="Attached from agterm"),
         (None, "text", "agterm", "agterm", None, "Attached from agterm")),
        (_props(control_type=50015, id="s", range_available=True, range=3.0), ("s", "slider", None, 3.0, None, None)),
        (_props(control_type=50025, id="", name=""), (None, "other", None, None, None, None)),
    ],
)
def test_snapshot_values_read_the_portable_vocabulary(props, expected):
    assert uia.snapshot_values(props) == expected


def test_plain_drops_the_not_supported_sentinel():
    assert uia.plain(object()) is None
    assert uia.plain([1, 2]) == (1, 2)
    assert uia.plain("x") == "x"


def test_every_chord_key_has_a_virtual_key():
    assert all(uia.virtual_key(key) for key in NAMED_KEYS | PRINTABLE_KEYS)
    assert uia.virtual_key("a") == 0x41
    assert uia.virtual_key("7") == 0x37
    assert uia.virtual_key("delete") == 0x08
    assert uia.virtual_key("forward-delete") == 0x2E
    assert uia.virtual_key(",") == 0xBC
    assert uia.virtual_key("f12") == 0x7B
    with pytest.raises(KeyError):
        uia.virtual_key("A")


def _keys(inputs):
    assert all(item.type == uia.INPUT_KEYBOARD for item in inputs)
    return [(item.ki.wVk, item.ki.wScan, item.ki.dwFlags) for item in inputs]


def test_input_structure_has_the_win32_layout():
    if ctypes.sizeof(ctypes.c_void_p) == 8:
        assert ctypes.sizeof(uia.INPUT) == 40
    else:
        assert ctypes.sizeof(uia.INPUT) == 28


def test_chord_presses_modifiers_around_the_key():
    up = uia.KEYEVENTF_KEYUP
    assert _keys(uia.chord_inputs(Chord("t", frozenset({"cmd", "shift"})))) == [
        (0x10, 0, 0), (0x11, 0, 0), (0x54, 0, 0), (0x54, 0, up), (0x11, 0, up), (0x10, 0, up),
    ]
    # cmd and ctrl are one Control
    assert len(uia.chord_inputs(Chord("a", frozenset({"cmd", "ctrl"})))) == 4
    ext = uia.KEYEVENTF_EXTENDEDKEY
    assert _keys(uia.chord_inputs(Chord("left"))) == [(0x25, 0, ext), (0x25, 0, ext | up)]


def test_text_is_unicode_code_units():
    down, up = uia.KEYEVENTF_UNICODE, uia.KEYEVENTF_UNICODE | uia.KEYEVENTF_KEYUP
    assert _keys(uia.text_inputs("A!é")) == [
        (0, ord("A"), down), (0, ord("A"), up), (0, ord("!"), down), (0, ord("!"), up),
        (0, 0xE9, down), (0, 0xE9, up),
    ]
    # outside the BMP: both surrogates, each down and up
    assert [scan for _, scan, _ in _keys(uia.text_inputs("😀"))] == [0xD83D, 0xD83D, 0xDE00, 0xDE00]


def test_absolute_normalizes_over_the_virtual_desktop():
    desk = (0, 0, 1025, 769)
    assert uia.absolute(0, 0, desk) == (0, 0)
    assert uia.absolute(1024, 768, desk) == (65535, 65535)
    assert uia.absolute(512, 384, desk) == (32768, 32768)
    # a monitor left of the primary moves the desktop's origin
    assert uia.absolute(-100, 0, (-100, 0, 201, 101)) == (0, 0)


class _User32:
    """user32 as the driver's input reaches it: SendInput records each event, the virtual desktop is fixed."""

    METRICS = {uia.SM_XVIRTUALSCREEN: 0, uia.SM_YVIRTUALSCREEN: 0, uia.SM_CXVIRTUALSCREEN: 1001,
               uia.SM_CYVIRTUALSCREEN: 1001}  # fmt: skip

    def __init__(self):
        self.sent = []

    def SendInput(self, count, array, size):  # noqa: N802 (user32's name)
        assert size == ctypes.sizeof(uia.INPUT)
        for item in array[:count]:
            assert item.type == uia.INPUT_MOUSE
            self.sent.append((item.mi.dwFlags, item.mi.dx, item.mi.dy))
        return count

    def GetSystemMetrics(self, index):  # noqa: N802
        return self.METRICS[index]


def _bare_driver(**stubs):
    """A UiaDriver without UI Automation behind it: its own code with `stubs` in place of the tree reads."""
    driver = object.__new__(uia.UiaDriver)
    driver.pid, driver.timeout, driver.user32 = 1, 1.0, _User32()
    for name, stub in stubs.items():
        setattr(driver, name, stub)
    return driver


def test_drag_moves_presses_steps_and_releases():
    driver = _bare_driver(_point=lambda target, offset: (100.0, 100.0))
    driver.drag_by("src", 100, 200, steps=2)
    desk = (0, 0, 1001, 1001)
    move = uia.MOUSEEVENTF_MOVE | uia.MOUSEEVENTF_ABSOLUTE | uia.MOUSEEVENTF_VIRTUALDESK
    assert driver.user32.sent == [
        (move, *uia.absolute(100, 100, desk)),
        (uia.MOUSEEVENTF_LEFTDOWN, 0, 0),
        (move, *uia.absolute(150, 200, desk)),
        (move, *uia.absolute(200, 300, desk)),
        (uia.MOUSEEVENTF_LEFTUP, 0, 0),
    ]
    assert uia.button_input("right", True).mi.dwFlags == uia.MOUSEEVENTF_RIGHTDOWN


def test_menu_reveals_each_item_before_the_pointer_reaches_it():
    """A scrolled menu may hold a submenu's trigger below its viewport: it is wheeled into view before the hover."""
    calls = []
    opened = ["app"]
    shown = []

    def menus(hwnd):
        assert hwnd == 7
        return set(shown)

    def click(button):
        calls.append(("click", button))
        shown.append(opened.pop(0))

    def reveal(scope, item):
        calls.append(("reveal", scope, item))
        return f"{item} in view"

    def hover(item):
        calls.append(("hover", item))
        shown.append(f"{item} submenu")

    button = uia.Node(element=object(), runtime_id=(1,), props={uia.AUTOMATION_ID: "app-menu-button"}, hwnd=7)
    driver = _bare_driver(
        _menu_path=None, _menu_depth=0, _menu_hwnd=0, _menu_scope=None, _menu_pending=None,
        _key_window=lambda: 7, _nodes=lambda: [button], snapshot=lambda node: "app-menu-button", _menus=menus,
        mouse_click=click, _scoped_menu_item=lambda scope, name: f"{scope}/{name}", _reveal=reveal, _hover=hover,
    )  # fmt: skip
    found = driver._menu_command(("File", "Open Recent", "Recent"))
    file_item = "app/File"
    recent_item = f"{file_item} in view submenu/Open Recent"
    assert calls == [
        ("click", "app-menu-button"),
        ("reveal", "app", file_item), ("hover", f"{file_item} in view"),
        ("reveal", f"{file_item} in view submenu", recent_item), ("hover", f"{recent_item} in view"),
        ("reveal", f"{recent_item} in view submenu", f"{recent_item} in view submenu/Recent"),
    ]  # fmt: skip
    assert found == f"{recent_item} in view submenu/Recent in view"


def test_wheel_ticks_are_notches_down_and_right_positive():
    inputs = uia.wheel_inputs(dx=-1, dy=2)
    deltas = [(item.mi.dwFlags, ctypes.c_int32(item.mi.mouseData).value) for item in inputs]
    assert deltas == [
        (uia.MOUSEEVENTF_WHEEL, -120), (uia.MOUSEEVENTF_WHEEL, -120), (uia.MOUSEEVENTF_HWHEEL, -120),
    ]


def test_nodes_compare_by_runtime_id():
    first = uia.Node(element=object(), runtime_id=(42, 1), props={}, hwnd=1)
    again = uia.Node(element=object(), runtime_id=(42, 1), props={}, hwnd=1)
    other = uia.Node(element=object(), runtime_id=(42, 2), props={}, hwnd=1, parent=first)
    assert first == again and hash(first) == hash(again)
    assert first != other
    assert list(other.ancestors()) == [first]
