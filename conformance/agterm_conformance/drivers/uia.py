"""Windows accessibility: UI Automation over the app's AccessKit tree, `SendInput` for keys, text and the pointer.

The bindings (comtypes over `UIAutomationCore.dll`, user32 through ctypes) are imported lazily, so the host-free
self-tests need neither. An element's ui id is its `AutomationId` (AccessKit's author id). One pass caches a window's
whole raw-view subtree (`BuildUpdatedCache`), so a read is one cross-process call per top-level window, never a
partial walk. Input goes to the foreground window, which the driver makes the target's own window first. The
portable cmd modifier maps to Control, matching ngterm's Windows key bindings.

Runs on a CI runner or a throwaway VM only: its windows open on the desktop and its input is the desktop's.
"""

from __future__ import annotations

import ctypes
import sys
import time
from collections.abc import Iterator, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any

from agterm_conformance.drivers.base import Chord, Driver, DriverError, Element, ReadFailed, UiIds
from agterm_conformance.wait import DEFAULT_TIMEOUT, WaitTimeout, wait_until

# UIA control type ids (UIAutomationClient.h) to the portable roles.
CONTROL_TYPES: Mapping[int, str] = {
    50000: "button", 50002: "checkbox", 50003: "popup", 50004: "text-field", 50006: "image", 50007: "row",
    50011: "menu-item", 50013: "radio", 50015: "slider", 50019: "radio", 50020: "text", 50024: "row",
    50026: "group", 50029: "row", 50032: "window", 50033: "group",
}  # fmt: skip
MENU_CONTROL_TYPE = 50009
MENU_ITEM_CONTROL_TYPE = 50011
# UIA property ids (UIAutomationClient.h); read from the cache a window's subtree is built with.
RUNTIME_ID = 30000
BOUNDING_RECTANGLE = 30001
PROCESS_ID = 30002
CONTROL_TYPE = 30003
NAME = 30005
IS_ENABLED = 30010
AUTOMATION_ID = 30011
NATIVE_WINDOW_HANDLE = 30020
IS_EXPAND_COLLAPSE_AVAILABLE = 30028
IS_INVOKE_AVAILABLE = 30031
IS_RANGE_VALUE_AVAILABLE = 30033
IS_SELECTION_ITEM_AVAILABLE = 30036
IS_TOGGLE_AVAILABLE = 30041
IS_VALUE_AVAILABLE = 30043
VALUE_VALUE = 30045
VALUE_IS_READ_ONLY = 30046
RANGE_VALUE_VALUE = 30047
EXPAND_COLLAPSE_STATE = 30070
SELECTION_ITEM_IS_SELECTED = 30079
TOGGLE_STATE = 30086
FULL_DESCRIPTION = 30159
CACHED_PROPERTIES = (
    RUNTIME_ID, CONTROL_TYPE, NAME, IS_ENABLED, AUTOMATION_ID, NATIVE_WINDOW_HANDLE, IS_EXPAND_COLLAPSE_AVAILABLE,
    IS_INVOKE_AVAILABLE, IS_RANGE_VALUE_AVAILABLE, IS_SELECTION_ITEM_AVAILABLE, IS_TOGGLE_AVAILABLE,
    IS_VALUE_AVAILABLE, VALUE_VALUE, VALUE_IS_READ_ONLY, RANGE_VALUE_VALUE, EXPAND_COLLAPSE_STATE,
    SELECTION_ITEM_IS_SELECTED, TOGGLE_STATE, FULL_DESCRIPTION,
)  # fmt: skip
# UIA pattern ids.
INVOKE_PATTERN = 10000
SELECTION_ITEM_PATTERN = 10010
TOGGLE_PATTERN = 10015
EXPAND_COLLAPSE_PATTERN = 10005
TREE_SCOPE_CHILDREN = 2
TREE_SCOPE_SUBTREE = 7
TOGGLE_ON = 1
EXPAND_COLLAPSE_COLLAPSED = 0

# Virtual keys: the US layout the runners and the VM use; `delete` is Backspace, as on a Mac keyboard.
VIRTUAL_KEYS: Mapping[str, int] = {
    "return": 0x0D, "enter": 0x0D, "escape": 0x1B, "tab": 0x09, "space": 0x20, "delete": 0x08,
    "forward-delete": 0x2E, "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28, "home": 0x24, "end": 0x23,
    "page-up": 0x21, "page-down": 0x22, **{f"f{n}": 0x6F + n for n in range(1, 13)},
    "`": 0xC0, "-": 0xBD, "=": 0xBB, "[": 0xDB, "]": 0xDD, "\\": 0xDC, ";": 0xBA, "'": 0xDE, ",": 0xBC, ".": 0xBE,
    "/": 0xBF,
}  # fmt: skip
# Keys whose scan code carries the E0 prefix: without the extended flag the arrows arrive as the keypad's.
EXTENDED_KEYS = frozenset({"enter", "forward-delete", "left", "up", "right", "down", "home", "end", "page-up",
                           "page-down"})  # fmt: skip
MODIFIER_KEYS: Mapping[str, int] = {"shift": 0x10, "ctrl": 0x11, "cmd": 0x11, "alt": 0x12}
VK_MENU = 0x12

INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010
MOUSEEVENTF_WHEEL = 0x0800
MOUSEEVENTF_HWHEEL = 0x1000
MOUSEEVENTF_VIRTUALDESK = 0x4000
MOUSEEVENTF_ABSOLUTE = 0x8000
WHEEL_DELTA = 120
BUTTON_FLAGS: Mapping[tuple[str, bool], int] = {
    ("left", True): MOUSEEVENTF_LEFTDOWN, ("left", False): MOUSEEVENTF_LEFTUP,
    ("right", True): MOUSEEVENTF_RIGHTDOWN, ("right", False): MOUSEEVENTF_RIGHTUP,
}  # fmt: skip
SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN = 76, 77, 78, 79
SW_RESTORE = 9
# DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: UIA's bounds and SendInput's absolute coordinates in physical pixels.
PER_MONITOR_AWARE_V2 = -4
FOREGROUND_TIMEOUT = 5.0
# How long pointer input waits after the driver moved the foreground. Windows CI, a wheel at once after taking the
# foreground from another app's window: 8 of 10 never reached the app, none of 10 after 50 ms.
POINTER_SETTLE = 0.3


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", ctypes.c_int32), ("dy", ctypes.c_int32), ("mouseData", ctypes.c_uint32),
                ("dwFlags", ctypes.c_uint32), ("time", ctypes.c_uint32), ("dwExtraInfo", ctypes.c_size_t)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", ctypes.c_uint16), ("wScan", ctypes.c_uint16), ("dwFlags", ctypes.c_uint32),
                ("time", ctypes.c_uint32), ("dwExtraInfo", ctypes.c_size_t)]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", ctypes.c_uint32), ("wParamL", ctypes.c_uint16), ("wParamH", ctypes.c_uint16)]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", ctypes.c_uint32), ("u", _INPUTUNION)]


def virtual_key(key: str) -> int:
    """A chord key's virtual key: a letter or digit is its upper-case character, the rest from `VIRTUAL_KEYS`."""
    if key in VIRTUAL_KEYS:
        return VIRTUAL_KEYS[key]
    if len(key) == 1 and (key.isdigit() or "a" <= key <= "z"):
        return ord(key.upper())
    raise KeyError(f"no virtual key for {key!r}")


def key_input(vk: int, down: bool, extended: bool = False) -> INPUT:
    flags = (0 if down else KEYEVENTF_KEYUP) | (KEYEVENTF_EXTENDEDKEY if extended else 0)
    return INPUT(type=INPUT_KEYBOARD, ki=KEYBDINPUT(wVk=vk, wScan=0, dwFlags=flags))


def chord_inputs(chord: Chord) -> list[INPUT]:
    """Modifiers down (in a fixed order), the key down and up, the modifiers up in reverse."""
    modifiers = sorted({MODIFIER_KEYS[modifier] for modifier in chord.modifiers})
    key = (virtual_key(chord.key), chord.key in EXTENDED_KEYS)
    return [
        *(key_input(vk, True) for vk in modifiers),
        key_input(key[0], True, key[1]),
        key_input(key[0], False, key[1]),
        *(key_input(vk, False) for vk in reversed(modifiers)),
    ]


def text_inputs(text: str) -> list[INPUT]:
    """One Unicode down and up per UTF-16 code unit (VK_PACKET): case and symbols need no Shift, any layout."""
    data = text.encode("utf-16-le")
    units = [int.from_bytes(data[i : i + 2], "little") for i in range(0, len(data), 2)]
    return [
        INPUT(type=INPUT_KEYBOARD, ki=KEYBDINPUT(wVk=0, wScan=unit, dwFlags=KEYEVENTF_UNICODE | up))
        for unit in units
        for up in (0, KEYEVENTF_KEYUP)
    ]


def absolute(x: float, y: float, desk: tuple[int, int, int, int]) -> tuple[int, int]:
    """A screen point in SendInput's normalized 0..65535 coordinates over the virtual desktop (left, top, w, h)."""
    left, top, width, height = desk
    return (
        round((x - left) * 65535 / max(width - 1, 1)),
        round((y - top) * 65535 / max(height - 1, 1)),
    )


def move_input(x: float, y: float, desk: tuple[int, int, int, int]) -> INPUT:
    dx, dy = absolute(x, y, desk)
    flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK
    return INPUT(type=INPUT_MOUSE, mi=MOUSEINPUT(dx=dx, dy=dy, mouseData=0, dwFlags=flags))


def button_input(button: str, down: bool) -> INPUT:
    return INPUT(type=INPUT_MOUSE, mi=MOUSEINPUT(dx=0, dy=0, mouseData=0, dwFlags=BUTTON_FLAGS[(button, down)]))


def wheel_inputs(dx: int, dy: int) -> list[INPUT]:
    """One notch per tick: a positive dy scrolls down (a negative wheel delta), a positive dx right."""
    inputs = []
    for distance, flag, sign in ((dy, MOUSEEVENTF_WHEEL, -1), (dx, MOUSEEVENTF_HWHEEL, 1)):
        for _ in range(abs(distance)):
            delta = sign * WHEEL_DELTA * (1 if distance > 0 else -1)
            inputs.append(INPUT(type=INPUT_MOUSE, mi=MOUSEINPUT(mouseData=ctypes.c_uint32(delta).value,
                                                                dwFlags=flag)))
    return inputs


def plain(value: Any) -> Any:
    """A cached property's VARIANT as a Python value; UIA's not-supported sentinel and anything else is None."""
    if isinstance(value, (str, bool, int, float)):
        return value
    if isinstance(value, (tuple, list)):
        return tuple(value)
    return None


def snapshot_values(
    props: Mapping[int, Any],
) -> tuple[str | None, str, str | None, Any, bool | None, str | None]:
    """(ui_id, role, name, value, enabled, description) of one element from its UIA properties, as the portable
    vocabulary."""
    control_type = props.get(CONTROL_TYPE)
    role = CONTROL_TYPES.get(control_type, "other") if isinstance(control_type, int) else "other"
    if role == "button" and props.get(IS_EXPAND_COLLAPSE_AVAILABLE):
        # AccessKit gives a button with a popup the expand/collapse pattern: a menu button.
        role = "menu-button"
    if role == "text-field" and props.get(IS_VALUE_AVAILABLE) and props.get(VALUE_IS_READ_ONLY):
        # A read-only field is selectable text (upstream's static text with `.textSelection`).
        role = "text"
    name = props.get(NAME) or None
    value: Any = None
    if role in ("checkbox", "radio"):
        if props.get(IS_SELECTION_ITEM_AVAILABLE):
            value = int(bool(props.get(SELECTION_ITEM_IS_SELECTED)))
        elif props.get(IS_TOGGLE_AVAILABLE):
            value = int(props.get(TOGGLE_STATE) == TOGGLE_ON)
        else:
            value = 0
    elif props.get(IS_VALUE_AVAILABLE):
        value = props.get(VALUE_VALUE)
    elif props.get(IS_RANGE_VALUE_AVAILABLE):
        value = props.get(RANGE_VALUE_VALUE)
    if role == "text" and value is None:
        # AccessKit names a label after its text (its label comes from its value): the text is the name.
        value = name
    enabled = props.get(IS_ENABLED)
    enabled = None if enabled is None else bool(enabled)
    return (props.get(AUTOMATION_ID) or None, role, name, value, enabled, props.get(FULL_DESCRIPTION) or None)


@dataclass(eq=False)
class Node:
    """A driver handle: the UIA element (its subtree cached), its runtime id, its parent node and its top-level
    window. Two snapshots of one element compare equal by runtime id."""

    element: Any
    runtime_id: tuple[int, ...]
    props: Mapping[int, Any]
    hwnd: int
    parent: Node | None = field(default=None, repr=False)

    def __eq__(self, other: object) -> bool:
        return isinstance(other, Node) and other.runtime_id == self.runtime_id

    def __hash__(self) -> int:
        return hash(self.runtime_id)

    def ancestors(self) -> Iterator[Node]:
        node = self.parent
        while node is not None:
            yield node
            node = node.parent


def _bindings() -> tuple[Any, Any, Any]:
    """(comtypes' generated UIAutomationClient module, the IUIAutomation object, user32)."""
    if sys.platform != "win32":
        raise DriverError(f"the uia driver runs on Windows, this host is {sys.platform}")
    try:
        import comtypes
        import comtypes.client
    except ImportError as error:
        raise DriverError(f"the uia driver needs comtypes: {error}") from None
    try:
        module = comtypes.client.GetModule("UIAutomationCore.dll")
        automation = comtypes.client.CreateObject(module.CUIAutomation, interface=module.IUIAutomation)
    except (OSError, comtypes.COMError) as error:
        raise DriverError(f"UI Automation is not available: {error}") from None
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.SetProcessDpiAwarenessContext.argtypes = [ctypes.c_void_p]
    # Fails when the process already chose its awareness; either way bounds and input then agree.
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(PER_MONITOR_AWARE_V2))
    user32.SendInput.argtypes = [ctypes.c_uint, ctypes.POINTER(INPUT), ctypes.c_int]
    user32.SendInput.restype = ctypes.c_uint
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    for name in ("SetForegroundWindow", "IsIconic", "BringWindowToTop"):
        getattr(user32, name).argtypes = [ctypes.c_void_p]
    user32.ShowWindow.argtypes = [ctypes.c_void_p, ctypes.c_int]
    return module, automation, user32


class UiaDriver(Driver):
    name = "uia"

    def __init__(self, pid: int, ui_ids: UiIds | None = None, timeout: float = DEFAULT_TIMEOUT) -> None:
        super().__init__(pid, ui_ids, timeout)
        self.uia, self.automation, self.user32 = _bindings()
        import comtypes

        self._com_error = comtypes.COMError
        self._request = self.automation.CreateCacheRequest()
        for prop in CACHED_PROPERTIES:
            self._request.AddProperty(prop)
        self._request.TreeScope = TREE_SCOPE_SUBTREE
        self._request.TreeFilter = self.automation.RawViewCondition
        self._windows_request = self.automation.CreateCacheRequest()
        self._windows_request.AddProperty(NATIVE_WINDOW_HANDLE)
        self._pid_condition = self.automation.CreatePropertyCondition(PROCESS_ID, pid)
        self._menu_path: tuple[str, ...] | None = None
        self._menu_depth = 0
        self._menu_hwnd = 0
        self._menu_scope: Node | None = None
        self._menu_pending: set[Node] | None = None
        try:
            wait_until(lambda: self._read(self._windows), timeout)
        except WaitTimeout as error:
            raise DriverError(f"uia: pid {pid} shows no top-level window: {error}") from None

    # -- the tree

    def _read(self, fn):
        try:
            return fn()
        except (self._com_error, OSError) as error:
            raise ReadFailed(f"uia pid {self.pid}: {error}") from error

    def _windows(self) -> list[tuple[Any, int]]:
        """The app's top-level windows, foreground first, as (element, hwnd)."""
        root = self.automation.GetRootElement()
        found = root.FindAllBuildCache(TREE_SCOPE_CHILDREN, self._pid_condition, self._windows_request)
        windows = []
        for index in range(found.Length if found else 0):
            element = found.GetElement(index)
            windows.append((element, int(element.GetCachedPropertyValue(NATIVE_WINDOW_HANDLE) or 0)))
        foreground = self.user32.GetForegroundWindow() or 0
        windows.sort(key=lambda window: window[1] != foreground)
        return windows

    def _props(self, element: Any) -> dict[int, Any]:
        return {prop: plain(element.GetCachedPropertyValue(prop)) for prop in CACHED_PROPERTIES}

    def _nodes(self) -> list[Node]:
        """Every element of every top-level window, each window's subtree cached in one call, in document order."""

        def read() -> list[Node]:
            nodes: list[Node] = []
            for window, hwnd in self._windows():
                cached = window.BuildUpdatedCache(self._request)
                stack: list[tuple[Any, Node | None]] = [(cached, None)]
                while stack:
                    element, parent = stack.pop()
                    props = self._props(element)
                    node = Node(element, props.get(RUNTIME_ID) or (), props, hwnd, parent)
                    nodes.append(node)
                    children = element.GetCachedChildren()
                    count = children.Length if children else 0
                    stack.extend((children.GetElement(i), node) for i in reversed(range(count)))
            return nodes

        return self._read(read)

    @staticmethod
    def snapshot(node: Node) -> Element:
        ui_id, role, name, value, enabled, description = snapshot_values(node.props)
        return Element(ui_id, role, name, value, enabled, node, description)

    def _all_elements(self) -> list[Element]:
        return [self.snapshot(node) for node in self._nodes()]

    def _elements(self, ui_id: str) -> list[Element]:
        return [self.snapshot(node) for node in self._nodes() if node.props.get(AUTOMATION_ID) == ui_id]

    def _identified(self) -> list[Element]:
        return [self.snapshot(node) for node in self._nodes() if node.props.get(AUTOMATION_ID)]

    # -- menus and tabs

    def _menu_item(self, name: str) -> Element | None:
        return next((el for el in self._all_elements() if el.role == "menu-item" and el.name == name), None)

    def _menus(self, hwnd: int) -> set[Node]:
        return {node for node in self._nodes()
                if node.hwnd == hwnd and node.props.get(CONTROL_TYPE) == MENU_CONTROL_TYPE}

    def _scoped_menu_item(self, scope: Node, name: str) -> Element | None:
        """An item of the menu `scope` by title; an item of a submenu nested in one of its items is not its own."""
        for node in self._nodes():
            if node.props.get(CONTROL_TYPE) != MENU_ITEM_CONTROL_TYPE or node.props.get(NAME) != name:
                continue
            for ancestor in node.ancestors():
                if ancestor == scope:
                    return self.snapshot(node)
                if ancestor.props.get(CONTROL_TYPE) == MENU_ITEM_CONTROL_TYPE:
                    break
        return None

    def _menu_command(self, path: tuple[str, ...]) -> Element | None:
        # GPUI's menus are in-window popups under the title bar's app menu button, each submenu a new Menu
        # beside the last: after each input the one new Menu is the next level's scope.
        if self._menu_path != path:
            hwnd = self._key_window()
            button = next((self.snapshot(node) for node in self._nodes()
                           if node.hwnd == hwnd and node.props.get(AUTOMATION_ID) == "app-menu-button"), None)
            if button is None:
                return None
            self._menu_hwnd = hwnd
            self._menu_pending = self._menus(hwnd)
            self.mouse_click(button)
            self._menu_path, self._menu_depth = path, 0
        while True:
            if self._menu_pending is not None:
                opened = self._menus(self._menu_hwnd) - self._menu_pending
                if len(opened) != 1:
                    return None
                (self._menu_scope,) = opened
                self._menu_pending = None
            assert self._menu_scope is not None
            if self._menu_depth == len(path) - 1:
                item = self._scoped_menu_item(self._menu_scope, path[-1])
                return None if item is None else self._reveal(self._menu_scope, item)
            title = path[self._menu_depth]
            if title in ("ngterm", "agterm"):
                # The app menu's own items sit at its top level.
                self._menu_depth += 1
                continue
            item = self._scoped_menu_item(self._menu_scope, title)
            if item is None:
                return None
            before = self._menus(self._menu_hwnd)
            # A submenu's trigger may sit below a scrolled menu's viewport too: the pointer only opens it in view.
            self._hover(self._reveal(self._menu_scope, item))
            self._menu_pending = before
            self._menu_depth += 1

    def _reveal(self, scope: Node, item: Element, attempts: int = 40) -> Element:
        """Wheel a scrolled menu under the pointer until `item` lies inside it, as a person scrolls to it."""

        def placement() -> int:
            top = self._bounds_of(scope)
            bottom = self._bounds_of(item.handle)
            if bottom[1] < top[1]:
                return -1
            if bottom[1] + bottom[3] > top[1] + top[3]:
                return 1
            return 0

        for _ in range(attempts):
            step = placement()
            if step == 0:
                return item
            x, y, w, h = self._bounds_of(scope)
            self._pointer_move(x + w / 2, y + h / 2)
            self._wheel(0, step)
            before = self._bounds_of(item.handle)
            try:
                wait_until(lambda: self._bounds_of(item.handle) != before, 1)
            except WaitTimeout:
                pass
        raise DriverError(f"uia: menu item {item.name!r} stays outside its menu after {attempts} wheel ticks")

    def _tab(self, name: str) -> Element | None:
        return next((el for el in self._all_elements()
                     if el.name == name and (el.ui_id == f"settings-tab-{name}" or el.role == "radio")), None)

    # -- actions through patterns

    def _pattern(self, node: Node, pattern_id: int, interface: Any) -> Any:
        return node.element.GetCurrentPattern(pattern_id).QueryInterface(interface)

    def _press_element(self, element: Element) -> None:
        """The accessibility press: Invoke, else Toggle, else SelectionItem's Select, else Expand/Collapse."""
        self._menu_path = None
        node: Node = element.handle
        props = node.props
        uia = self.uia

        def perform() -> None:
            if props.get(IS_INVOKE_AVAILABLE):
                self._pattern(node, INVOKE_PATTERN, uia.IUIAutomationInvokePattern).Invoke()
            elif props.get(IS_TOGGLE_AVAILABLE):
                self._pattern(node, TOGGLE_PATTERN, uia.IUIAutomationTogglePattern).Toggle()
            elif props.get(IS_SELECTION_ITEM_AVAILABLE):
                self._pattern(node, SELECTION_ITEM_PATTERN, uia.IUIAutomationSelectionItemPattern).Select()
            elif props.get(IS_EXPAND_COLLAPSE_AVAILABLE):
                pattern = self._pattern(node, EXPAND_COLLAPSE_PATTERN, uia.IUIAutomationExpandCollapsePattern)
                if props.get(EXPAND_COLLAPSE_STATE) == EXPAND_COLLAPSE_COLLAPSED:
                    pattern.Expand()
                else:
                    pattern.Collapse()
            else:
                raise DriverError(f"uia has no press pattern on {element}")

        try:
            perform()
        except self._com_error as error:
            raise DriverError(f"uia press on {element} failed: {error}") from error

    def _text(self, element: Element) -> str:
        return element.value if isinstance(element.value, str) else element.name or ""

    def _set_text(self, element: Element, text: str) -> None:
        # GPUI handles no SetValue action: replace the field's text as a person does, through the keyboard on the
        # focused field (select all, then type or delete).
        if element.role != "text-field":
            raise DriverError(f"uia cannot set text on {element}")
        self._focus(element)
        self._post_chord(Chord("a", frozenset({"ctrl"})))
        if text:
            self._type_text(text)
        else:
            self._post_chord(Chord("delete", frozenset()))

    def _focus(self, element: Element) -> None:
        node: Node = element.handle
        self._activate(node.hwnd)
        try:
            node.element.SetFocus()
        except self._com_error as error:
            raise DriverError(f"uia cannot focus {element}: {error}") from error

    def _hover(self, element: Element) -> None:
        self.move(element)

    # -- the foreground window

    def _key_window(self) -> int:
        """The app's foreground window, else its first: where keys and menus go."""
        windows = self._read(self._windows)
        if not windows:
            raise ReadFailed(f"uia pid {self.pid}: no top-level window")
        return windows[0][1]

    def _activate(self, hwnd: int) -> bool:
        """Make `hwnd` the foreground window, the one SendInput's keys and the topmost pointer target reach; whether
        the foreground moved.

        Windows lets a process take the foreground only right after input of its own; a tap of Alt is that input
        (GPUI's windows have no menu bar for it to open)."""
        user32 = self.user32
        if (user32.GetForegroundWindow() or 0) == hwnd:
            return False
        if user32.IsIconic(hwnd):
            user32.ShowWindow(hwnd, SW_RESTORE)
        self._send([key_input(VK_MENU, True), key_input(VK_MENU, False)])
        user32.SetForegroundWindow(hwnd)
        user32.BringWindowToTop(hwnd)
        try:
            wait_until(lambda: (user32.GetForegroundWindow() or 0) == hwnd, FOREGROUND_TIMEOUT)
        except WaitTimeout:
            raise DriverError(f"uia: window {hwnd:#x} of pid {self.pid} did not come to the foreground") from None
        return True

    # -- input

    def _send(self, inputs: Sequence[INPUT]) -> None:
        if not inputs:
            return
        array = (INPUT * len(inputs))(*inputs)
        sent = self.user32.SendInput(len(inputs), array, ctypes.sizeof(INPUT))
        if sent != len(inputs):
            raise DriverError(f"SendInput sent {sent} of {len(inputs)} events: error {ctypes.get_last_error()}")

    def _desk(self) -> tuple[int, int, int, int]:
        metric = self.user32.GetSystemMetrics
        return (metric(SM_XVIRTUALSCREEN), metric(SM_YVIRTUALSCREEN), metric(SM_CXVIRTUALSCREEN),
                metric(SM_CYVIRTUALSCREEN))

    def _bounds_of(self, node: Node) -> tuple[float, float, float, float]:
        def read() -> tuple[float, float, float, float]:
            rect = node.element.CurrentBoundingRectangle
            return rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top

        return self._read(read)

    def _bounds(self, element: Element) -> tuple[float, float, float, float]:
        return self._bounds_of(element.handle)

    def _pointer_target(self, element: Element) -> None:
        # The pointer hits the topmost window at the point, and another window (of this app or another instance)
        # may cover the target's: bring the target's own window to the foreground, which also raises it.
        if self._activate(element.handle.hwnd):
            # keys follow the foreground at once, but a wheel sent right after the move may never reach the window
            time.sleep(POINTER_SETTLE)

    def _pointer_move(self, x: float, y: float) -> None:
        self._send([move_input(x, y, self._desk())])

    def _pointer_button(self, button: str, down: bool) -> None:
        self._send([button_input(button, down)])

    def _wheel(self, dx: int, dy: int) -> None:
        self._send(wheel_inputs(dx, dy))

    def _post_chord(self, chord: Chord) -> None:
        self._activate(self._key_window())
        self._send(chord_inputs(chord))

    def _type_text(self, text: str) -> None:
        self._activate(self._key_window())
        self._send(text_inputs(text))
