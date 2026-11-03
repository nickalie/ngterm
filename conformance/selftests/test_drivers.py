import re
import sys
from pathlib import Path
from typing import Any

import pytest
from agterm_conformance.drivers import ax
from agterm_conformance.drivers import base as drivers
from agterm_conformance.drivers.base import (
    Chord,
    Driver,
    DriverError,
    Element,
    ElementNotFound,
    NoDriver,
    ReadFailed,
    UiIds,
    UnknownUiId,
    driver_class,
    driver_name,
    make_driver,
    parse_chord,
)
from upstream_adapter import UpstreamVocabularyAdapter

IDS = UiIds(
    [
        "add-session",
        "ask-button-{choice}",
        "settings-status-{status}",
        "settings-status-shape-{status}",
        "settings-status-reset",
        "pane-lead-cover-{pane}",
        "htmlOverlay.back",
    ]
)


class FakeDriver(Driver):
    name = "fake"

    def __init__(self, elements: list[Element], **kwargs: Any) -> None:
        super().__init__(4242, IDS, timeout=0.2, **kwargs)
        self.elements = elements
        self.menu_items: list[Element] = []
        self.menu_bar: dict[tuple[str, ...], Element] = {}
        self.tabs: list[Element] = []
        self.pressed: list[Element] = []
        self.posted: list[Chord] = []
        self.typed: list[tuple[Element, str]] = []
        self.hovered: list[Element] = []
        self.focused: list[Element] = []
        self.events = []
        self.bounds = (10, 20, 100, 40)
        self.polls = 0

    def _elements(self, ui_id: str) -> list[Element]:
        self.polls += 1
        return [element for element in self.elements if element.ui_id == ui_id]

    def _identified(self) -> list[Element]:
        return [element for element in self.elements if element.ui_id]

    def _menu_item(self, name: str) -> Element | None:
        return next((item for item in self.menu_items if item.name == name), None)

    def _menu_command(self, path: tuple[str, ...]) -> Element | None:
        return self.menu_bar.get(path)

    def _tab(self, name: str) -> Element | None:
        return next((tab for tab in self.tabs if tab.name == name), None)

    def _press_element(self, element: Element) -> None:
        self.pressed.append(element)

    def _text(self, element: Element) -> str:
        return str(element.value)

    def _post_chord(self, chord: Chord) -> None:
        self.posted.append(chord)

    def _set_text(self, element: Element, text: str) -> None:
        self.typed.append((element, text))

    def _hover(self, element: Element) -> None:
        self.hovered.append(element)

    def _focus(self, element: Element) -> None:
        self.focused.append(element)

    def _all_elements(self):
        return self.elements + self.menu_items + self.tabs

    def _bounds(self, element):
        return self.bounds

    def _pointer_move(self, x, y):
        self.events.append(("move", x, y))

    def _pointer_button(self, button, down):
        self.events.append((button, down))

    def _wheel(self, dx, dy):
        self.events.append(("wheel", dx, dy))

    def _type_text(self, text):
        self.events.append(("text", text))


# -- ids and templates


@pytest.mark.parametrize(
    ("ui_id", "key"),
    [
        ("add-session", "add-session"),
        ("ask-button-Yes", "ask-button-{choice}"),
        ("ask-button-Not now", "ask-button-{choice}"),
        ("settings-status-shape-idle", "settings-status-shape-{status}"),
        ("settings-status-idle", "settings-status-{status}"),
        ("settings-status-reset", "settings-status-reset"),
        ("htmlOverlay.back", "htmlOverlay.back"),
    ],
)
def test_key_for_maps_an_id_to_its_entry(ui_id: str, key: str) -> None:
    assert IDS.key_for(ui_id) == key


@pytest.mark.parametrize("ui_id", ["add-sessions", "ask-button-", "pane-lead-cover-{pane}", "htmlOverlayXback", ""])
def test_key_for_refuses_what_is_not_cataloged(ui_id: str) -> None:
    with pytest.raises(UnknownUiId):
        IDS.key_for(ui_id)


def test_expand_fills_every_placeholder() -> None:
    assert IDS.expand("ask-button-{choice}", choice="Yes") == "ask-button-Yes"
    assert IDS.expand("add-session") == "add-session"
    assert IDS.key_for(IDS.expand("pane-lead-cover-{pane}", pane="p1")) == "pane-lead-cover-{pane}"


@pytest.mark.parametrize(
    ("template", "values", "error", "message"),
    [
        ("ask-title", {}, UnknownUiId, "not in ui-ids.yaml"),
        ("ask-button-{choice}", {}, ValueError, "no value for choice"),
        ("ask-button-{choice}", {"choice": "Yes", "pane": "p"}, ValueError, "no placeholder pane"),
        ("add-session", {"choice": "Yes"}, ValueError, "no placeholder choice"),
        ("ask-button-{choice}", {"choice": ""}, ValueError, "non-empty string"),
        ("ask-button-{choice}", {"choice": 1}, ValueError, "non-empty string"),
    ],
)
def test_expand_refuses(template: str, values: dict[str, Any], error: type[Exception], message: str) -> None:
    with pytest.raises(error, match=message):
        IDS.expand(template, **values)


def test_the_committed_ui_ids_load_with_manual_ids_and_templates() -> None:
    ids = UiIds.load()
    assert {"add-session", "session-row", "htmlOverlay.close", "settings-interface-{element}"} <= ids.keys
    assert ids.key_for("settings-interface-sidebar") == "settings-interface-{element}"
    assert ids.key_for("settings-status-shape-working") == "settings-status-shape-{status}"


# -- chords


@pytest.mark.parametrize(
    ("spec", "chord"),
    [
        ("a", Chord("a")),
        ("return", Chord("return")),
        ("cmd+shift+t", Chord("t", frozenset({"cmd", "shift"}))),
        ("option+command+up", Chord("up", frozenset({"alt", "cmd"}))),
        ("ctrl+/", Chord("/", frozenset({"ctrl"}))),
        ("f12", Chord("f12")),
    ],
)
def test_parse_chord(spec: str, chord: Chord) -> None:
    assert parse_chord(spec) == chord


@pytest.mark.parametrize("spec", ["", "+", "cmd+", "hyper+a", "A", "cmd+shift", "ab", "f13", "é"])
def test_parse_chord_refuses(spec: str) -> None:
    with pytest.raises(ValueError, match="bad chord"):
        parse_chord(spec)


def test_ax_has_a_key_code_for_every_key_and_a_portable_name_for_every_role() -> None:
    assert drivers.NAMED_KEYS | drivers.PRINTABLE_KEYS <= ax.KEY_CODES.keys()
    assert set(ax.ROLES.values()) <= drivers.ROLES
    assert set(ax.FLAGS) == set(drivers.MODIFIERS.values())


@pytest.mark.parametrize(
    ("value", "plain"),
    [(None, None), (True, True), (3, 3), (0.5, 0.5), ("x", "x"), (b"raw", "b'raw'")],
)
def test_ax_values_become_plain_python(value: Any, plain: Any) -> None:
    assert ax._plain(value) == plain and type(ax._plain(value)) is type(plain)


# -- the driver interface


def test_find_returns_the_first_element_matching_id_and_attributes() -> None:
    menu = Element("add-session", "menu-button", "Add session")
    button = Element("add-session", "button", "Add session", enabled=True)
    driver = FakeDriver([menu, button])
    assert driver.find("add-session") is menu
    assert driver.find("add-session", role="button", enabled=True) is button


def test_find_waits_for_the_element() -> None:
    late = Element("ask-button-Yes", "button", "Yes")
    driver = FakeDriver([])

    def appear(ui_id: str) -> list[Element]:
        driver.polls += 1
        return [late] if driver.polls >= 3 else []

    driver._elements = appear  # type: ignore[method-assign]
    assert driver.find("ask-button-Yes") is late


def test_find_times_out_with_a_diagnosis() -> None:
    driver = FakeDriver([Element("add-session", "button")])
    with pytest.raises(ElementNotFound, match=r"no add-session \{'role': 'menu-button'\} in pid 4242"):
        driver.find("add-session", timeout=0.1, role="menu-button")


@pytest.mark.parametrize(
    ("ui_id", "attrs", "error", "message"),
    [
        ("session-row", {}, UnknownUiId, "session-row"),
        ("add-session", {"title": "x"}, ValueError, "unknown attribute title"),
        ("add-session", {"role": "AXButton"}, ValueError, "unknown role"),
    ],
)
def test_find_refuses_bad_queries_before_polling(
    ui_id: str, attrs: dict[str, Any], error: type[Exception], message: str
) -> None:
    driver = FakeDriver([])
    with pytest.raises(error, match=message):
        driver.find(ui_id, **attrs)
    assert driver.polls == 0


def test_click_and_text_resolve_an_id_or_take_an_element() -> None:
    element = Element("add-session", "button", value="label")
    driver = FakeDriver([element])
    driver.click("add-session")
    driver.click(element)
    assert driver.pressed == [element, element]
    assert driver.text("add-session") == "label"


def test_set_text_hover_and_focus_resolve_an_id_or_take_an_element() -> None:
    field = Element("add-session", "text-field")
    driver = FakeDriver([field])
    driver.set_text("add-session", "needle")
    driver.hover(field)
    driver.focus("add-session")
    assert driver.typed == [(field, "needle")] and driver.hovered == driver.focused == [field]


def test_press_validates_every_chord_before_posting_any() -> None:
    driver = FakeDriver([])
    with pytest.raises(ValueError, match="bad chord"):
        driver.press("a", "nope")
    assert driver.posted == []
    driver.press("a", "cmd+n")
    assert driver.posted == [Chord("a"), Chord("n", frozenset({"cmd"}))]


def test_menu_item_waits_by_title() -> None:
    driver = FakeDriver([])
    item = Element(None, "menu-item", "New Session")
    driver.menu_items = [item]
    assert driver.menu_item("New Session") is item
    with pytest.raises(ElementNotFound, match="menu item 'Open'"):
        driver.menu_item("Open", timeout=0.1)


def test_menu_presses_the_command_at_the_end_of_its_path() -> None:
    driver = FakeDriver([])
    find = Element(None, "menu-item", "Find…")
    driver.menu_bar = {("View", "Find…"): find}
    driver.menu("View", "Find…")
    assert driver.pressed == [find]
    with pytest.raises(ElementNotFound, match="menu command 'Edit > Find…'"):
        driver.menu("Edit", "Find…", timeout=0.1)
    with pytest.raises(ValueError, match="needs a path"):
        driver.menu()


def test_tab_waits_by_title() -> None:
    driver = FakeDriver([])
    general = Element(None, "button", "General")
    driver.tabs = [general]
    assert driver.tab("General") is general
    with pytest.raises(ElementNotFound, match="tab 'Interface'"):
        driver.tab("Interface", timeout=0.1)


def test_all_lists_every_element_of_an_id_without_waiting() -> None:
    rows = [Element("add-session", "button", value=str(n)) for n in range(3)]
    driver = FakeDriver([*rows, Element("htmlOverlay.back", "button")])
    assert driver.all("add-session") == rows
    assert driver.all("ask-button-No") == []
    with pytest.raises(UnknownUiId):
        driver.all("session-row")


def test_a_miss_lists_what_carries_an_identifier() -> None:
    driver = FakeDriver(
        [Element("add-session", "popup", "Add session"), Element("htmlOverlay.back", "button", enabled=False)]
    )
    with pytest.raises(
        ElementNotFound,
        match=r"identified now: add-session\(popup name='Add session'\), htmlOverlay.back\(button disabled\)$",
    ):
        driver.find("ask-button-Yes", timeout=0.1)


def test_a_miss_says_when_the_inventory_could_not_be_read() -> None:
    driver = FakeDriver([])

    def unread() -> list[Element]:
        raise ReadFailed("AXError -25204")

    driver._identified = unread  # type: ignore[method-assign]
    with pytest.raises(ElementNotFound, match="inventory unread: AXError -25204"):
        driver.find("add-session", timeout=0.1)


def test_absent_waits_until_no_element_carries_the_id() -> None:
    driver = FakeDriver([Element("add-session", "button")])
    driver.absent("htmlOverlay.back")
    with pytest.raises(AssertionError, match="fake: add-session still present in pid 4242"):
        driver.absent("add-session", timeout=0.1)


class FakeAx:
    """ApplicationServices for one app whose AXChildren answers each read with the next queued AXError."""

    def __init__(
        self, children_errors: list[int], child_error: int = ax.AX_SUCCESS, failing: frozenset[str] = frozenset()
    ) -> None:
        self.app, self.dialog = object(), object()
        self.children_errors = children_errors
        self.child_error = child_error
        self.failing = failing

    def AXIsProcessTrusted(self) -> bool:
        return True

    def AXUIElementCreateApplication(self, pid: int) -> object:
        return self.app

    def AXUIElementSetMessagingTimeout(self, handle: object, seconds: float) -> int:
        return ax.AX_SUCCESS

    def AXUIElementCopyAttributeValue(self, handle: object, name: str, out: None) -> tuple[int, Any]:
        if handle is self.app:
            if name != "AXChildren":
                return ax.AX_ATTRIBUTE_UNSUPPORTED, None
            error = self.children_errors.pop(0) if self.children_errors else ax.AX_SUCCESS
            return error, [self.dialog] if error == ax.AX_SUCCESS else None
        if self.child_error != ax.AX_SUCCESS:
            return self.child_error, None
        if name in self.failing:
            return ax.AX_FAILURE, None
        return {"AXIdentifier": (ax.AX_SUCCESS, "add-session"), "AXRole": (ax.AX_SUCCESS, "AXButton")}.get(
            name, (ax.AX_NO_VALUE, None)
        )


def fake_ax_driver(monkeypatch: pytest.MonkeyPatch, bindings: FakeAx) -> ax.AxDriver:
    monkeypatch.setattr(ax, "_bindings", lambda: (bindings, None))
    return ax.AxDriver(4242, IDS, timeout=0.2)


def test_ax_absent_never_accepts_a_read_the_app_did_not_answer(monkeypatch: pytest.MonkeyPatch) -> None:
    driver = fake_ax_driver(monkeypatch, FakeAx([ax.AX_CANNOT_COMPLETE] * 1000))
    with pytest.raises(AssertionError, match="still present.*AXChildren in pid 4242 failed: AXError -25204"):
        driver.absent("add-session", timeout=0.1)
    with pytest.raises(ReadFailed):
        driver.attribute(driver.app, "AXChildren")


def test_ax_retries_a_failed_read(monkeypatch: pytest.MonkeyPatch) -> None:
    bindings = FakeAx([ax.AX_CANNOT_COMPLETE, ax.AX_CANNOT_COMPLETE])
    element = fake_ax_driver(monkeypatch, bindings).find("add-session")
    assert (element.ui_id, element.role, element.name) == ("add-session", "button", None)
    assert bindings.children_errors == []


def test_ax_treats_a_vanished_child_as_absent_but_not_a_vanished_app(monkeypatch: pytest.MonkeyPatch) -> None:
    fake_ax_driver(monkeypatch, FakeAx([], child_error=ax.AX_INVALID_ELEMENT)).absent("add-session")
    driver = fake_ax_driver(monkeypatch, FakeAx([ax.AX_INVALID_ELEMENT]))
    with pytest.raises(ReadFailed, match="AXError -25202"):
        driver.attribute(driver.app, "AXChildren")


def test_ax_reads_a_failing_display_attribute_as_no_value_but_not_a_failing_identifier(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    failing = frozenset({"AXDescription", "AXTitle"})
    element = fake_ax_driver(monkeypatch, FakeAx([], failing=failing)).find("add-session")
    assert (element.ui_id, element.name) == ("add-session", None)
    driver = fake_ax_driver(monkeypatch, FakeAx([], failing=frozenset({"AXIdentifier"})))
    with pytest.raises(AssertionError, match="still present.*AXIdentifier.*-25200"):
        driver.absent("add-session", timeout=0.1)


# -- selection


@pytest.mark.parametrize(
    ("impl", "platform", "name"),
    [
        ("swift-macos", "linux", "ax"),
        ("swift-macos", "win32", "ax"),
        ("rust", "darwin", "ax"),
        ("rust", "linux", "atspi"),
        ("rust", "win32", "uia"),
    ],
)
def test_driver_name_follows_the_reference_platform_else_the_host(impl: str, platform: str, name: str) -> None:
    assert driver_name(impl, platform) == name


def test_driver_name_refuses_an_unknown_platform() -> None:
    with pytest.raises(NoDriver, match="'sunos5'"):
        driver_name("rust", "sunos5")


@pytest.mark.parametrize(
    ("impl", "platform", "message"),
    [
        ("swift-macos", "linux", "needs darwin, this host is linux"),
        ("swift-macos", "win32", "needs darwin, this host is win32"),
    ],
)
def test_driver_class_says_why_there_is_none(impl: str, platform: str, message: str) -> None:
    with pytest.raises(NoDriver, match=message):
        driver_class(impl, platform)


def test_uia_is_the_driver_for_windows() -> None:
    from agterm_conformance.drivers import uia

    assert driver_class("rust", "win32") is uia.UiaDriver


def test_ax_is_the_driver_for_macos() -> None:
    assert driver_class("swift-macos", "darwin") is ax.AxDriver
    assert driver_class("rust", "darwin") is ax.AxDriver


def test_make_driver_hands_the_pid_and_ids_over(monkeypatch: pytest.MonkeyPatch) -> None:
    created: list[tuple[int, UiIds | None, dict[str, Any]]] = []

    class Recording:
        def __init__(self, pid: int, ui_ids: UiIds | None, **kwargs: Any) -> None:
            created.append((pid, ui_ids, kwargs))

    monkeypatch.setattr(drivers, "driver_class", lambda impl, platform: Recording)
    make_driver("swift-macos", 42, IDS, platform="darwin", timeout=3.0)
    assert created == [(42, IDS, {"timeout": 3.0})]


def test_ax_without_pyobjc_is_a_driver_error(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setitem(sys.modules, "ApplicationServices", None)
    with pytest.raises(DriverError, match="pyobjc-framework-ApplicationServices"):
        ax.AxDriver(1, IDS)
    with pytest.raises(DriverError):
        ax.trust()


def test_ax_refuses_an_unknown_post_mode() -> None:
    with pytest.raises(ValueError, match="post must be one of"):
        ax.AxDriver(1, IDS, post="tap")


# -- the `ui` fixture


@pytest.fixture(autouse=True)
def _upstream_impl(upstream_impl: str) -> str:
    """The ui fixture's suite runs go through the self-test double, which names no platform of its own."""
    return upstream_impl


def run_ui_suite(pytester: pytest.Pytester, fake_bin: Path, *args: str) -> pytest.RunResult:
    return pytester.runpytest(
        "-p", "no:cacheprovider", "-rs", "--impl", UpstreamVocabularyAdapter.name, "--bin", str(fake_bin), *args
    )  # fmt: skip


def test_ui_fixture_skips_as_unsupported_before_anything_launches(
    pytester: pytest.Pytester, roots: list[Path], fake_bin: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(drivers, "IMPLEMENTED", {"ax": "agterm_conformance.drivers.ax:AxDriver"})
    pytester.makepyfile(
        """
        def test_ui(ui):
            raise AssertionError("must not run")

        def test_ui_after_ctl(ctl, ui):
            raise AssertionError("must not run")
        """
    )
    result = run_ui_suite(pytester, fake_bin)
    result.assert_outcomes(skipped=2)
    result.stdout.fnmatch_lines_random(
        ["*unsupported: the atspi ui driver is not written yet*", "*2 skip(unsupported)*"]
    )
    assert roots == []


def test_ui_fixture_drives_the_ctl_instance(
    pytester: pytest.Pytester, roots: list[Path], fake_bin: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class Recording:
        def __init__(self, pid: int, ui_ids: UiIds | None, instance=None) -> None:
            self.pid, self.ui_ids = pid, ui_ids
            self.instance = instance

    monkeypatch.setattr(drivers, "driver_class", lambda impl, platform=sys.platform: Recording)
    pytester.makepyfile(
        """
        def test_ui(ctl, ui):
            assert ui.pid == ctl.instance.pid
            assert ui.instance is ctl.instance
            assert ui.ui_ids.keys == {"next-to-the-catalog"}
        """
    )
    (pytester.path / "spec").mkdir()
    (pytester.path / "spec" / "ui-ids.yaml").write_text("ids:\n  next-to-the-catalog: {}\n")
    run_ui_suite(pytester, fake_bin, "--catalog", "spec/catalog.yaml").assert_outcomes(passed=1)


def test_ui_for_skips_as_unsupported_before_anything_launches(
    pytester: pytest.Pytester, roots: list[Path], fake_bin: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(drivers, "IMPLEMENTED", {"ax": "agterm_conformance.drivers.ax:AxDriver"})
    pytester.makepyfile(
        """
        def test_ui_for(launch, ui_for):
            raise AssertionError("must not run")
        """
    )
    result = run_ui_suite(pytester, fake_bin)
    result.assert_outcomes(skipped=1)
    result.stdout.fnmatch_lines(["*unsupported: the atspi ui driver is not written yet*"])
    assert roots == []


def test_ui_for_drives_the_instance_it_is_given(
    pytester: pytest.Pytester, roots: list[Path], fake_bin: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    class Recording:
        def __init__(self, pid: int, ui_ids: UiIds | None, instance=None) -> None:
            self.pid = pid

    monkeypatch.setattr(drivers, "driver_class", lambda impl, platform=sys.platform: Recording)
    pytester.makepyfile(
        """
        def test_ui_for(launch, ui_for):
            first, second = launch(), launch()
            assert ui_for(second).pid == second.instance.pid != first.instance.pid
            assert ui_for(first.instance).pid == first.instance.pid
        """
    )
    (pytester.path / "spec").mkdir()
    (pytester.path / "spec" / "ui-ids.yaml").write_text("ids: {}\n")
    run_ui_suite(pytester, fake_bin, "--catalog", "spec/catalog.yaml").assert_outcomes(passed=1)


@pytest.mark.parametrize("button", ["left", "right"])
def test_pointer_click_and_double_click(button):
    el = Element("add-session", "button", enabled=True)
    driver = FakeDriver([el])
    driver.mouse_click(el, button=button, offset=(0, 1))
    assert driver.events == [("move", 10, 60), (button, True), (button, False)]
    driver.events.clear()
    driver.mouse_double_click(el, button=button)
    assert driver.events == [("move", 60, 40), (button, True), (button, False),
                             (button, True), (button, False)]


def test_pointer_bounds_are_read_at_action_time_and_stale_identity_is_preserved():
    old = Element("add-session", "button", "Second", handle="old")
    first = Element("add-session", "button", "First", handle="first")
    new = Element("add-session", "button", "Second", handle="new")
    driver = FakeDriver([first, new])
    seen = []
    def bounds(el):
        seen.append(el.handle)
        return driver.bounds
    driver._bounds = bounds
    driver.move(old)
    driver.bounds = (100, 200, 40, 20)
    driver.move(old, offset=(1, 0))
    assert seen == ["new", "new"]
    assert driver.events == [("move", 60, 40), ("move", 140, 200)]
    driver.elements = [first]
    with pytest.raises(ElementNotFound):
        driver.move(old)
    assert len(driver.events) == 2


def test_pointer_without_id_matches_role_and_name():
    el = Element(None, "menu-item", "Open", handle="old")
    replacement = Element(None, "menu-item", "Open", handle="new")
    driver = FakeDriver([Element(None, "button", "Open"), replacement])
    seen = []
    driver._bounds = lambda current: (seen.append(current.handle) or driver.bounds)
    driver.move(el)
    assert seen == ["new"]
    driver.elements = []
    driver.events.clear()
    with pytest.raises(ElementNotFound):
        driver.move(el)
    assert driver.events == []


def test_pointer_waits_for_enabled_and_never_inputs_on_timeout():
    driver = FakeDriver([Element("add-session", "button", enabled=False)])
    with pytest.raises(ElementNotFound, match="enabled"):
        driver.mouse_click("add-session")
    assert driver.events == []
    count = 0
    def appears(ui_id):
        nonlocal count
        count += 1
        return [Element(ui_id, "button", enabled=count >= 3)]
    driver._elements = appears
    driver.mouse_click("add-session")
    assert count >= 3
    assert driver.events[-1] == ("left", False)


def test_drag_scroll_and_typing():
    driver = FakeDriver([Element("add-session", "button")])
    driver.drag_by("add-session", 20, -10, steps=2)
    assert driver.events == [("move", 60, 40), ("left", True), ("move", 70, 35),
                             ("move", 80, 30), ("left", False)]
    driver.events.clear()
    driver.drag("add-session", "add-session", src_offset=(0, 0), dst_offset=(1, 1), steps=1)
    assert driver.events == [("move", 10, 20), ("left", True), ("move", 110, 60), ("left", False)]
    driver.scroll("add-session", -3, dx=2)
    driver.type_text("Hello, Я! 😀")
    assert driver.events[-3:] == [("move", 60, 40), ("wheel", 2, -3), ("text", "Hello, Я! 😀")]


@pytest.mark.parametrize(("action", "handle"), [
    (lambda d: d.move("add-session"), "add"),
    (lambda d: d.mouse_click("add-session", button="right"), "add"),
    (lambda d: d.mouse_double_click("add-session"), "add"),
    (lambda d: d.scroll("add-session", 2), "add"),
    (lambda d: d.drag_by("add-session", 5, 5, steps=1), "add"),
    (lambda d: d.drag("add-session", "settings-status-reset", steps=1), "add"),
    (lambda d: d.drag("settings-status-reset", "add-session", steps=1), "reset"),
])
def test_pointer_actions_target_the_live_element_before_the_first_move(action, handle):
    """Positional input reaches the window on top: every pointer action names its target first."""
    driver = FakeDriver([Element("add-session", "button", handle="add"),
                         Element("settings-status-reset", "button", handle="reset")])
    driver._pointer_target = lambda element: driver.events.append(("target", element.handle))
    action(driver)
    assert driver.events[0] == ("target", handle)
    assert [event for event in driver.events if event[0] == "target"] == [driver.events[0]]


def test_pointer_target_is_not_raised_for_bad_arguments_or_missing_targets():
    driver = FakeDriver([Element("add-session", "button", enabled=False)])
    driver._pointer_target = lambda element: driver.events.append(("target", element.handle))
    with pytest.raises(ValueError):
        driver.move("add-session", offset=(2, 0))
    with pytest.raises(ElementNotFound):
        driver.mouse_click("add-session")
    assert driver.events == []


def test_drag_releases_button_on_failure_without_retry():
    driver = FakeDriver([Element("add-session", "button")])
    def move(x, y):
        driver.events.append(("move", x, y))
        if len(driver.events) > 1:
            raise DriverError("injection failed")
    driver._pointer_move = move
    with pytest.raises(DriverError, match="injection failed"):
        driver.drag_by("add-session", 20, 0)
    assert driver.events[-1] == ("left", False)
    assert driver.events.count(("left", True)) == 1


@pytest.mark.parametrize("action", [
    lambda d: d.move("add-session", offset=(2, 0)),
    lambda d: d.mouse_click("add-session", button="middle"),
    lambda d: d.drag_by("add-session", 1, 2, steps=0),
    lambda d: d.drag_by("add-session", float("nan"), 2),
    lambda d: d.type_text("hello\n"),
])
def test_pointer_bad_arguments_send_nothing(action):
    driver = FakeDriver([Element("add-session", "button")])
    with pytest.raises(ValueError):
        action(driver)
    assert driver.events == []


def test_count_and_regex_name():
    driver = FakeDriver([Element("add-session", "button", "Session 12")])
    assert driver.find("add-session", name=re.compile(r"Session \d+")) == driver.elements[0]
    assert driver.count("add-session", 1) == driver.elements
    assert driver.count("htmlOverlay.back", 0) == []
    with pytest.raises(ElementNotFound, match="count.*2.*last count.*1"):
        driver.count("add-session", 2, timeout=0)
    with pytest.raises(ElementNotFound):
        driver.find("add-session", name=re.compile("Workspace"), timeout=0)


def test_wait_cursor_returns_readback_and_timeout_has_last_value():
    from agterm_conformance.client import decode_response
    from agterm_conformance.wait import wait_cursor, WaitTimeout
    class Client:
        def request(self, command, target, **args):
            assert (command, target, args) == ("surface.cursor", "sid", {"paneID": "right"})
            return decode_response(b'{"ok":true,"result":{"cursor":{"column":7}}}')
    assert wait_cursor(Client(), "sid", 7, paneID="right") == {"cursor": {"column": 7}}
    with pytest.raises(WaitTimeout, match='column.*7'):
        wait_cursor(Client(), "sid", 8, timeout=0, paneID="right")


def test_wait_cursor_polls_until_column_matches():
    from agterm_conformance.client import Response
    from agterm_conformance.wait import wait_cursor
    class Client:
        columns = iter([1, 3, 7])
        def request(self, command, target, **args):
            result = {"cursor": {"column": next(self.columns)}}
            return Response(True, result, None, {"ok": True, "result": result})
    assert wait_cursor(Client(), "surface:sid:left", 7) == {"cursor": {"column": 7}}


class FakeQuartz:
    kCGEventLeftMouseDragged = "left-drag"
    kCGEventRightMouseDragged = "right-drag"
    kCGEventMouseMoved = "move"
    kCGMouseButtonRight = "right"
    kCGMouseButtonLeft = "left"
    kCGEventLeftMouseDown = "left-down"
    kCGEventLeftMouseUp = "left-up"
    kCGEventRightMouseDown = "right-down"
    kCGEventRightMouseUp = "right-up"
    kCGMouseEventClickState = "clicks"
    kCGHIDEventTap = "hid"
    kCGScrollEventUnitLine = "line"

    def __init__(self):
        self.events = []

    def CGEventCreateMouseEvent(self, source, kind, point, button):
        return {"kind": kind, "point": point, "button": button}

    def CGEventSetIntegerValueField(self, event, field, value):
        event[field] = value

    def CGEventPost(self, tap, event):
        self.events.append((tap, event))

    def CGEventPostToPid(self, pid, event):
        self.events.append((pid, event))

    def CGEventCreateScrollWheelEvent(self, source, units, count, dy, dx):
        return {"units": units, "count": count, "dy": dy, "dx": dx}

    def CGEventCreateKeyboardEvent(self, source, code, down):
        return {"code": code, "down": down}

    def CGEventKeyboardSetUnicodeString(self, event, length, text):
        event.update(length=length, text=text)


def test_ax_pointer_primitives_emit_drag_click_counts_and_scroll(monkeypatch):
    driver = fake_ax_driver(monkeypatch, FakeAx([]))
    driver.quartz = q = FakeQuartz()
    driver.activate = lambda: None
    driver._pointer_move(10, 20)
    driver._pointer_button("left", True)
    driver._pointer_move(30, 40)
    driver._pointer_button("left", False)
    driver._double_click("right")
    driver._wheel(2, -3)
    assert [event["kind"] for _, event in q.events[:4]] == ["move", "left-down", "left-drag", "left-up"]
    assert q.events[3][1]["point"] == (30, 40)
    assert [event["clicks"] for _, event in q.events[4:8]] == [1, 1, 2, 2]
    assert [event["kind"] for _, event in q.events[4:8]] == [
        "right-down", "right-up", "right-down", "right-up",
    ]
    assert all(event["button"] == "right" for _, event in q.events[4:8])
    assert all(tap == "hid" for tap, _ in q.events)
    assert q.events[-1][1] == {"units": "line", "count": 2, "dy": 3, "dx": -2}
    assert driver._held_button is None


@pytest.mark.parametrize("post", ["pid", "hid"])
def test_ax_unicode_includes_utf16_surrogate_pairs(monkeypatch, post):
    driver = fake_ax_driver(monkeypatch, FakeAx([]))
    driver.quartz = q = FakeQuartz()
    driver.post = post
    driver.type_text("Я😀")
    assert [event["length"] for _, event in q.events] == [1, 1, 2, 2]
    assert [event["text"] for _, event in q.events] == ["Я", "Я", "😀", "😀"]
    assert [event["down"] for _, event in q.events] == [True, False, True, False]
    assert all(destination == (4242 if post == "pid" else "hid") for destination, _ in q.events)


@pytest.mark.parametrize("target_kind", ["id", "snapshot", "no-id-snapshot"])
def test_pointer_wait_follows_live_control_name_changes(target_kind):
    ui_id = None if target_kind == "no-id-snapshot" else "add-session"
    old = Element(ui_id, "button", "Saving…", enabled=False, handle="same")
    new = Element(ui_id, "button", "Save", enabled=True, handle="same")
    driver = FakeDriver([old])
    polls = 0
    def elements(*args):
        nonlocal polls
        polls += 1
        return [old if polls == 1 else new]
    driver._elements = elements
    driver._all_elements = elements
    driver.mouse_click("add-session" if target_kind == "id" else old)
    assert polls >= 2
    assert driver.events == [("move", 60, 40), ("left", True), ("left", False)]


def test_pointer_id_wait_can_follow_a_rebuilt_and_renamed_control():
    driver = FakeDriver([])
    polls = 0
    def elements(ui_id):
        nonlocal polls
        polls += 1
        if polls == 1:
            return [Element(ui_id, "button", "Saving…", enabled=False, handle="old")]
        return [Element(ui_id, "button", "Save", enabled=True, handle="new")]
    driver._elements = elements
    driver.mouse_click("add-session")
    assert driver.events[-1] == ("left", False)


@pytest.mark.parametrize("ui_id", ["add-session", None])
def test_pointer_same_named_controls_use_handle_and_reject_stale_ambiguity(ui_id):
    first = Element(ui_id, "button", "Same", handle="first")
    second = Element(ui_id, "button", "Same", handle="second")
    driver = FakeDriver([first, second])
    seen = []
    driver._bounds = lambda el: (seen.append(el.handle) or driver.bounds)
    driver.move(second)
    assert seen == ["second"]
    driver.events.clear()
    with pytest.raises(ElementNotFound, match="ambiguous"):
        driver.move(Element(ui_id, "button", "Same", handle="stale"))
    assert driver.events == []


@pytest.mark.parametrize("recovers", [True, False])
def test_pointer_retries_failed_bounds_without_input(recovers):
    el = Element("add-session", "button", handle="old")
    driver = FakeDriver([el])
    reads = 0
    def bounds(current):
        nonlocal reads
        reads += 1
        assert driver.events == []
        if reads == 1 or not recovers:
            raise ReadFailed("bounds vanished")
        return driver.bounds
    driver._bounds = bounds
    if recovers:
        driver.mouse_click(el)
        assert reads == 2
        assert driver.events[-1] == ("left", False)
    else:
        with pytest.raises(ElementNotFound, match="bounds vanished"):
            driver.mouse_click(el)
        assert driver.events == []


@pytest.mark.parametrize("source_change", ["moves", "disappears", "disabled"])
def test_drag_refreshes_source_while_destination_becomes_ready(source_change):
    source = Element("add-session", "button", "Source", enabled=True, handle="source")
    dest = Element("htmlOverlay.back", "button", "Destination", enabled=False, handle="dest")
    driver = FakeDriver([source, dest])
    dest_polls = 0
    def elements(ui_id):
        nonlocal dest_polls
        if ui_id == dest.ui_id:
            dest_polls += 1
            return [Element(dest.ui_id, dest.role, dest.name, enabled=dest_polls > 1, handle="dest")]
        if dest_polls and source_change == "disappears":
            return []
        return [Element(source.ui_id, source.role, source.name,
                        enabled=not (dest_polls and source_change == "disabled"), handle="source")]
    driver._elements = elements
    def bounds(el):
        if el.handle == "dest":
            return (400, 0, 100, 40)
        return (100 if dest_polls else 0, 0, 100, 40)
    driver._bounds = bounds
    if source_change == "moves":
        driver.drag(source, dest, steps=1)
        assert driver.events[0] == ("move", 150, 20)
    else:
        with pytest.raises(ElementNotFound):
            driver.drag(source, dest, steps=1)
        assert driver.events == []


@pytest.mark.parametrize("missing", ["AXPosition", "AXSize"])
def test_ax_missing_pointer_geometry_is_retryable_but_hover_error_unchanged(monkeypatch, missing):
    from types import SimpleNamespace
    bindings = FakeAx([])
    bindings.kAXValueCGPointType = 1
    bindings.kAXValueCGSizeType = 2
    bindings.AXValueGetValue = lambda value, kind, out: (True, value)
    driver = fake_ax_driver(monkeypatch, bindings)
    driver.attribute = lambda handle, name: None if name == missing else SimpleNamespace(
        x=10, y=20, width=100, height=40,
    )
    el = Element("add-session", "button", handle=bindings.dialog)
    with pytest.raises(ReadFailed, match=missing):
        driver._bounds(el)
    with pytest.raises(DriverError, match=missing):
        driver._hover(el)
