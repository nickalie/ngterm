"""UI drivers: elements found by their ui-ids.yaml accessibility identifier, pressed, read, and keys posted.

A driver speaks one platform's accessibility API to one app process. Tests see only `Element` and the portable
attribute vocabulary (`role`, `name`, `value`, `enabled`), never the platform's tree structure.
"""

from __future__ import annotations

import importlib
import math
import re
import string
import sys
from abc import ABC, abstractmethod
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, ClassVar, TypeVar

import yaml

from agterm_conformance.wait import DEFAULT_TIMEOUT, WaitTimeout, wait_until

DEFAULT_UI_IDS = Path(__file__).resolve().parents[2] / "ui-ids.yaml"
# The accessibility API per platform; a Rust build on Linux exposes AT-SPI, on Windows UIA.
PLATFORM_DRIVERS: Mapping[str, str] = {"darwin": "ax", "linux": "atspi", "win32": "uia"}
IMPL_PLATFORMS: Mapping[str, str] = {"swift-macos": "darwin"}
IMPLEMENTED: Mapping[str, str] = {"ax": "agterm_conformance.drivers.ax:AxDriver"}
ATTRIBUTES = frozenset({"role", "name", "value", "enabled"})
ROLES = frozenset({
    "button", "menu-button", "menu-item", "checkbox", "radio", "popup", "text-field", "text", "slider", "row",
    "group", "image", "window", "other",
})  # fmt: skip
MODIFIERS: Mapping[str, str] = {
    "cmd": "cmd", "command": "cmd", "ctrl": "ctrl", "control": "ctrl",
    "alt": "alt", "opt": "alt", "option": "alt", "shift": "shift",
}  # fmt: skip
NAMED_KEYS = frozenset({
    "return", "enter", "escape", "tab", "space", "delete", "forward-delete", "up", "down", "left", "right",
    "home", "end", "page-up", "page-down", *(f"f{n}" for n in range(1, 13)),
})  # fmt: skip
PRINTABLE_KEYS = frozenset(string.ascii_lowercase + string.digits + "`-=[]\\;',./")
_PLACEHOLDER = re.compile(r"\{(\w+)\}")
INVENTORY_LIMIT = 120
T = TypeVar("T")


class NoDriver(Exception):
    """No driver exists for this platform yet: the ui layer skips as unsupported."""


class DriverError(Exception):
    """The platform's driver exists but cannot run here (missing bindings, accessibility not granted)."""


class ReadFailed(AssertionError):
    """The accessibility API did not answer a read; a wait retries it and an empty tree is never inferred."""


class UnknownUiId(ValueError):
    pass


class ElementNotFound(WaitTimeout):
    pass


@dataclass(frozen=True)
class Chord:
    key: str
    modifiers: frozenset[str] = frozenset()


@dataclass(frozen=True)
class Element:
    """A snapshot of one element; `handle` is the driver's own reference, opaque to tests."""

    ui_id: str | None
    role: str
    name: str | None = None
    value: Any = None
    enabled: bool | None = None
    handle: Any = field(default=None, compare=False, repr=False)

    def matches(self, attrs: Mapping[str, Any]) -> bool:
        return all(
            bool(wanted.search(self.name or "")) if key == "name" and isinstance(wanted, re.Pattern)
            else getattr(self, key) == wanted for key, wanted in attrs.items()
        )


class UiIds:
    """The identifiers ui tests may address: ui-ids.yaml `ids` and `manual` keys, `{name}` parts as templates."""

    def __init__(self, keys: Iterable[str]) -> None:
        self.keys = frozenset(keys)
        templates = [key for key in self.keys if _PLACEHOLDER.search(key)]
        # Most literal text first: `settings-status-shape-x` belongs to `settings-status-shape-{status}`.
        templates.sort(key=lambda key: -len(_PLACEHOLDER.sub("", key)))
        self._templates = [(key, re.compile(_template_pattern(key))) for key in templates]

    @classmethod
    def load(cls, path: Path = DEFAULT_UI_IDS) -> UiIds:
        data = yaml.safe_load(path.read_text()) or {}
        return cls([*(data.get("ids") or {}), *(data.get("manual") or {})])

    def expand(self, template: str, **values: str) -> str:
        """`ask-button-{choice}` with choice="Yes" is `ask-button-Yes`; every placeholder, nothing else."""
        if template not in self.keys:
            raise UnknownUiId(f"{template!r} is not in ui-ids.yaml")
        wanted = set(_PLACEHOLDER.findall(template))
        if missing := sorted(wanted - values.keys()):
            raise ValueError(f"{template}: no value for {', '.join(missing)}")
        if extra := sorted(values.keys() - wanted):
            raise ValueError(f"{template}: no placeholder {', '.join(extra)}")
        for name, value in values.items():
            if not isinstance(value, str) or not value:
                raise ValueError(f"{template}: {name} must be a non-empty string, got {value!r}")
        return _PLACEHOLDER.sub(lambda match: values[match.group(1)], template)

    def key_for(self, ui_id: str) -> str:
        """The ui-ids.yaml key `ui_id` instantiates, which is also its `ui:` coverage key."""
        if _PLACEHOLDER.search(ui_id):
            raise UnknownUiId(f"{ui_id!r} is a template: expand it first")
        if ui_id in self.keys:
            return ui_id
        for key, pattern in self._templates:
            if pattern.fullmatch(ui_id):
                return key
        raise UnknownUiId(f"{ui_id!r} is neither an id nor an expansion of a template in ui-ids.yaml")


def _describe(element: Element) -> str:
    parts = [element.role]
    if element.name is not None:
        parts.append(f"name={element.name!r}")
    if element.value is not None:
        parts.append(f"value={element.value!r}"[:80])
    if element.enabled is False:
        parts.append("disabled")
    return f"{element.ui_id}({' '.join(parts)})"


def _template_pattern(template: str) -> str:
    parts = _PLACEHOLDER.split(template)
    # split() alternates literal text and placeholder names.
    return "".join(re.escape(part) if index % 2 == 0 else ".+" for index, part in enumerate(parts))


def parse_chord(spec: str) -> Chord:
    """`cmd+shift+t`, `return`, `a`: modifiers then one key, lower case."""
    *mods, key = spec.split("+")
    unknown = [mod for mod in mods if mod not in MODIFIERS]
    if unknown or not key:
        raise ValueError(f"bad chord {spec!r}: modifiers are {', '.join(sorted(set(MODIFIERS.values())))}")
    if key not in NAMED_KEYS and key not in PRINTABLE_KEYS:
        raise ValueError(f"bad chord {spec!r}: unknown key {key!r}")
    return Chord(key, frozenset(MODIFIERS[mod] for mod in mods))


class Driver(ABC):
    """`find` / `click` / `press` / `text` over one app process; `menu`, `menu_item` and `tab` reach the platform's
    own chrome, which carries no identifiers."""

    name: ClassVar[str] = ""

    def __init__(self, pid: int, ui_ids: UiIds | None = None, timeout: float = DEFAULT_TIMEOUT) -> None:
        self.pid = pid
        self.ui_ids = ui_ids if ui_ids is not None else UiIds.load()
        self.timeout = timeout

    def find(self, ui_id: str, timeout: float | None = None, **attrs: Any) -> Element:
        """Wait for the first element carrying `ui_id` whose attributes equal `attrs`.

        A compiled `name` pattern uses regex search; a missing name is read as an empty string.
        """
        self.ui_ids.key_for(ui_id)
        if unknown := sorted(attrs.keys() - ATTRIBUTES):
            raise ValueError(f"unknown attribute {', '.join(unknown)} (known: {', '.join(sorted(ATTRIBUTES))})")
        if "role" in attrs and attrs["role"] not in ROLES:
            raise ValueError(f"unknown role {attrs['role']!r} (known: {', '.join(sorted(ROLES))})")
        wanted = f"{ui_id} {attrs}" if attrs else ui_id

        def check() -> Element | None:
            return next((el for el in self._elements(ui_id) if el.matches(attrs)), None)

        return self._wait(check, wanted, timeout)

    def all(self, ui_id: str) -> list[Element]:
        """Every element carrying `ui_id` now, unwaited: for counts and for picking one by its text."""
        self.ui_ids.key_for(ui_id)
        return self._elements(ui_id)

    def count(self, ui_id: str, n: int, timeout: float | None = None) -> list[Element]:
        """Wait for exactly n matches, including zero, and return their snapshots."""
        self.ui_ids.key_for(ui_id)
        if not isinstance(n, int) or n < 0:
            raise ValueError("count must be a nonnegative integer")
        found = []

        def check():
            nonlocal found
            found = self._elements(ui_id)
            assert len(found) == n, f"last count: {len(found)}"
            return True

        self._wait(check, f"count {n} of {ui_id}", timeout)
        return found

    def absent(self, ui_id: str, timeout: float | None = None) -> None:
        """Wait until no element carries `ui_id`."""
        self.ui_ids.key_for(ui_id)
        try:
            wait_until(lambda: not self._elements(ui_id), self.timeout if timeout is None else timeout)
        except WaitTimeout as error:
            raise WaitTimeout(f"{self.name}: {ui_id} still present in pid {self.pid}: {error}") from None

    def menu_item(self, name: str, timeout: float | None = None) -> Element:
        """An item of the open menu by title: system menus carry no identifiers."""
        return self._wait(lambda: self._menu_item(name), f"menu item {name!r}", timeout)

    def menu(self, *path: str, timeout: float | None = None) -> None:
        """Run a main-menu command by its titles, top level first (`"View", "Find…"`)."""
        if not path:
            raise ValueError("menu needs a path")
        item = self._wait(lambda: self._menu_command(path), f"menu command {' > '.join(path)!r}", timeout)
        self._press_element(item)

    def tab(self, name: str, timeout: float | None = None) -> Element:
        """A tab of a tabbed window, such as Settings, by its title."""
        return self._wait(lambda: self._tab(name), f"tab {name!r}", timeout)

    def click(self, target: str | Element) -> None:
        self._press_element(self._resolve(target))

    def text(self, target: str | Element) -> str:
        return self._text(self._resolve(target))

    def set_text(self, target: str | Element, text: str) -> None:
        """Replace a text field's contents through the accessibility API, independent of keyboard focus."""
        self._set_text(self._resolve(target), text)

    def focus(self, target: str | Element) -> None:
        """Give the element keyboard focus, so `press` types into it."""
        self._focus(self._resolve(target))

    def hover(self, target: str | Element) -> None:
        """Move the pointer over the element, for controls that appear only under it."""
        self._hover(self._resolve(target))

    def press(self, *chords: str) -> None:
        """Post each chord as a synthesized key down and up, to the app process."""
        parsed = [parse_chord(chord) for chord in chords]
        for chord in parsed:
            self._post_chord(chord)

    def _pointer_element(self, target: str | Element) -> Element:
        if isinstance(target, str):
            self.ui_ids.key_for(target)
            # An id denotes the current control, not the label of its first snapshot.
            current = next(iter(self._elements(target)), None)
        else:
            candidates = self._elements(target.ui_id) if target.ui_id else self._all_elements()
            current = next((el for el in candidates if target.handle is not None and
                            el.handle == target.handle), None)
            if current is None:
                matches = [el for el in candidates if el.name == target.name and
                           (target.ui_id is not None or el.role == target.role)]
                current = matches[0] if len(matches) == 1 else None
        assert current is not None, "pointer target missing or ambiguous"
        assert current.enabled is not False, "element not enabled"
        return current

    def _point_now(self, target: str | Element, offset: tuple[float, float]) -> tuple[float, float]:
        if len(offset) != 2 or any(not math.isfinite(v) or not 0 <= v <= 1 for v in offset):
            raise ValueError("offset must contain two finite fractions in [0, 1]")
        current = self._pointer_element(target)
        x, y, width, height = self._bounds(current)
        assert all(math.isfinite(v) for v in (x, y, width, height)), "nonfinite bounds"
        assert width > 0 and height > 0, "empty bounds"
        return (x + width * offset[0], y + height * offset[1])

    def _point(self, target: str | Element, offset: tuple[float, float]) -> tuple[float, float]:
        wanted = target if isinstance(target, str) else _describe(target)
        return self._wait(lambda: self._point_now(target, offset), f"enabled pointer target {wanted}", None)

    def move(self, target: str | Element, offset=(0.5, 0.5)) -> None:
        self._pointer_move(*self._point(target, offset))

    @staticmethod
    def _check_button(button: str) -> None:
        if button not in ("left", "right"):
            raise ValueError("button must be left or right")

    def mouse_click(self, target: str | Element, button="left", offset=(0.5, 0.5)) -> None:
        """A physical pointer click; click() remains the accessibility press."""
        self._check_button(button)
        self.move(target, offset)
        self._pointer_button(button, True)
        self._pointer_button(button, False)

    def mouse_double_click(self, target: str | Element, button="left", offset=(0.5, 0.5)) -> None:
        self._check_button(button)
        self.move(target, offset)
        self._double_click(button)

    @staticmethod
    def _check_steps(steps: int) -> None:
        if not isinstance(steps, int) or steps < 1:
            raise ValueError("steps must be a positive integer")

    def _drag_points(self, start, end, steps: int) -> None:
        self._pointer_move(*start)
        self._pointer_button("left", True)
        try:
            for step in range(1, steps + 1):
                self._pointer_move(*(a + (b - a) * step / steps for a, b in zip(start, end)))
        finally:
            self._pointer_button("left", False)

    def drag(self, src: str | Element, dst: str | Element, src_offset=(0.5, 0.5),
             dst_offset=(0.5, 0.5), steps=10) -> None:
        self._check_steps(steps)
        # Neither endpoint may retain coordinates across the other endpoint's readiness wait.
        start, end = self._wait(
            lambda: (self._point_now(src, src_offset), self._point_now(dst, dst_offset)),
            f"enabled drag endpoints {src!r} -> {dst!r}", None,
        )
        self._drag_points(start, end, steps)

    def drag_by(self, target: str | Element, dx: float, dy: float, offset=(0.5, 0.5), steps=10) -> None:
        self._check_steps(steps)
        if not all(math.isfinite(v) for v in (dx, dy)):
            raise ValueError("drag distances must be finite")
        start = self._point(target, offset)
        self._drag_points(start, (start[0] + dx, start[1] + dy), steps)

    def scroll(self, target: str | Element, dy: int, dx: int = 0) -> None:
        """Scroll wheel ticks: positive down/right, negative up/left."""
        if not all(isinstance(v, int) for v in (dx, dy)):
            raise ValueError("scroll distances must be integer wheel ticks")
        self.move(target)
        self._wheel(dx, dy)

    def type_text(self, text: str) -> None:
        """Type printable Unicode into the focused control; use press for control keys."""
        if not isinstance(text, str) or any(not char.isprintable() for char in text):
            raise ValueError("type_text needs printable text")
        self._type_text(text)

    def _double_click(self, button: str) -> None:
        # Platforms with explicit click counts override this (CGEvent on macOS).
        for _ in range(2):
            self._pointer_button(button, True)
            self._pointer_button(button, False)

    @abstractmethod
    def _all_elements(self) -> list[Element]:
        """All elements, including controls without an author id."""

    @abstractmethod
    def _bounds(self, element: Element) -> tuple[float, float, float, float]:
        """Fresh screen coordinates (x, y, width, height)."""

    @abstractmethod
    def _pointer_move(self, x: float, y: float) -> None: ...

    @abstractmethod
    def _pointer_button(self, button: str, down: bool) -> None: ...

    @abstractmethod
    def _wheel(self, dx: int, dy: int) -> None: ...

    @abstractmethod
    def _type_text(self, text: str) -> None: ...

    def _resolve(self, target: str | Element) -> Element:
        return target if isinstance(target, Element) else self.find(target)

    def _wait(self, check: Callable[[], T], wanted: str, timeout: float | None) -> T:
        try:
            return wait_until(check, self.timeout if timeout is None else timeout)
        except WaitTimeout as error:
            raise ElementNotFound(f"{self.name}: no {wanted} in pid {self.pid}: {error}; {self._present()}") from None

    def _present(self) -> str:
        try:
            elements = self._identified()
        except ReadFailed as error:
            return f"inventory unread: {error}"
        shown = ", ".join(_describe(element) for element in elements[:INVENTORY_LIMIT])
        more = f" and {len(elements) - INVENTORY_LIMIT} more" if len(elements) > INVENTORY_LIMIT else ""
        return f"identified now: {shown or 'none'}{more}"

    @abstractmethod
    def _elements(self, ui_id: str) -> list[Element]:
        """Every element carrying `ui_id`; raises ReadFailed rather than answer from a partial read."""

    @abstractmethod
    def _identified(self) -> list[Element]:
        """Every element carrying an identifier."""

    @abstractmethod
    def _menu_item(self, name: str) -> Element | None: ...

    @abstractmethod
    def _menu_command(self, path: tuple[str, ...]) -> Element | None: ...

    @abstractmethod
    def _tab(self, name: str) -> Element | None: ...

    @abstractmethod
    def _press_element(self, element: Element) -> None: ...

    @abstractmethod
    def _text(self, element: Element) -> str: ...

    @abstractmethod
    def _post_chord(self, chord: Chord) -> None: ...

    @abstractmethod
    def _set_text(self, element: Element, text: str) -> None: ...

    @abstractmethod
    def _focus(self, element: Element) -> None: ...

    @abstractmethod
    def _hover(self, element: Element) -> None: ...


def driver_name(impl: str, platform: str = sys.platform) -> str:
    """The accessibility API an implementation's app exposes: the reference's platform, else this host's."""
    host = IMPL_PLATFORMS.get(impl, platform)
    if host not in PLATFORM_DRIVERS:
        raise NoDriver(f"no ui driver for platform {host!r}")
    return PLATFORM_DRIVERS[host]


def driver_class(impl: str, platform: str = sys.platform) -> type[Driver]:
    """The driver for `impl` on this host, or NoDriver saying why there is none."""
    name = driver_name(impl, platform)
    if name not in IMPLEMENTED:
        raise NoDriver(f"the {name} ui driver is not written yet")
    if PLATFORM_DRIVERS.get(platform) != name:
        raise NoDriver(f"the {name} ui driver needs {IMPL_PLATFORMS.get(impl, platform)}, this host is {platform}")
    module, _, cls = IMPLEMENTED[name].partition(":")
    return getattr(importlib.import_module(module), cls)


def make_driver(
    impl: str, pid: int, ui_ids: UiIds | None = None, platform: str = sys.platform, **kwargs: Any
) -> Driver:
    return driver_class(impl, platform)(pid, ui_ids, **kwargs)
