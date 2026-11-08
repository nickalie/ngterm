"""Pointer input on the sidebar and the split: row drags, the divider, the wheel and the rows' context menus.

The gestures are upstream's UI tests' (`ReorderUITests`, `SidebarUITests`, `SplitRatioUITests`,
`ControlOverlaySplitUITests` v0.39.1), read back through the tree. Upstream has no context menu on a terminal pane
(`GhosttySurfaceView+Input.swift`: a right click is the terminal's own right-click-action), so none is tested here.
Every test leaves the shared instance's workspaces, sidebar and selection as it found them."""

import time

import pytest
from agterm_conformance.match import assert_ok, find_session, sessions, workspaces
from agterm_conformance.wait import WaitTimeout, wait_until

# Upstream's row drags (`ReorderUITests.dragRow`): a press held still before the move, a human pace, a hold before the
# release. An outline view starts a row drag only from such a gesture.
ROW_DRAG = {"press_for": 0.7, "duration": 0.4, "hold_for": 0.25, "steps": 12}
# Upstream's divider drag (`SplitRatioUITests.testDividerDragPersistsTheRatio`).
DIVIDER_DRAG = {"press_for": 0.2, "duration": 0.5, "hold_for": 0.2, "steps": 10}
# A thin divider's grab band is a few points wide: upstream retries the start at small offsets until the ratio moves.
DIVIDER_OFFSETS = (0, -2, 2, -4, 4)
# Upstream drags 60 points across a 900-point window (0.067 of it); a fraction of the window's own width keeps the move
# past the 0.05 the test asserts whatever the display scale (UIA's bounds and SendInput are physical pixels).
DIVIDER_DRAG_FRACTION = 0.08
# A session row's lower part: upstream drops ON the row (inserted after it), this build after a row's lower half.
AFTER_ROW = (0.5, 0.7)
# A workspace row's top sliver: the slot above it (`ReorderUITests.dragWorkspaceRow`).
ABOVE_ROW = (0.5, 0.05)


def _row(ui, ui_id, name):
    """The `ui_id` row whose text is `name`: rows share their id, the text tells them apart."""
    return wait_until(lambda: next((row for row in ui.all(ui_id) if ui.text(row) == name), None), timeout=ui.timeout)


def _workspace(ctl, name):
    return assert_ok(ctl.request("workspace.new", name=name))["id"]


def _named_sessions(ctl, wid, names):
    ids = {}
    for name in names:
        sid = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
        assert_ok(ctl.request("session.rename", sid, name=name))
        ids[name] = sid
    return ids


def _session_names(tree, wid):
    return [session["name"] for session in sessions(tree, wid)]


def _workspace_names(tree, names):
    return [workspace["name"] for workspace in workspaces(tree) if workspace["name"] in names]


def _select_by_click(ctl, ui, row, sid):
    """Upstream clicks a row before dragging it: an outline view drags only its selected row."""
    ui.mouse_click(row)
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("active") is True)


@pytest.mark.covers("ui:session-row", "behavior:sidebar.drag-reorder")
def test_dragging_a_session_row_onto_another_reorders_its_workspace(ctl, ui):
    """ccc dropped on aaa lands just after it (`ReorderUITests.testReorderSessionUp`)."""
    wid = _workspace(ctl, "ui-drag-sessions")
    try:
        ids = _named_sessions(ctl, wid, ["aaa", "bbb", "ccc"])
        _row(ui, "session-row", "aaa")
        _select_by_click(ctl, ui, _row(ui, "session-row", "ccc"), ids["ccc"])
        ui.drag(_row(ui, "session-row", "ccc"), _row(ui, "session-row", "aaa"), dst_offset=AFTER_ROW, **ROW_DRAG)
        ctl.wait_tree(lambda tree: _session_names(tree, wid) == ["aaa", "ccc", "bbb"])
    finally:
        ctl.request("workspace.delete", wid)


@pytest.mark.covers("ui:workspace-row", "behavior:sidebar.drag-reorder")
def test_dragging_a_workspace_row_above_another_reorders_the_workspaces(ctl, ui):
    """A workspace dropped on another's top edge lands before it (`ReorderUITests.testReorderWorkspace`)."""
    names = ["ui-ws-one", "ui-ws-two", "ui-ws-three"]
    wids = [_workspace(ctl, name) for name in names]
    try:
        ctl.wait_tree(lambda tree: _workspace_names(tree, names) == names)
        source = _row(ui, "workspace-row", "ui-ws-three")
        # selected first, as upstream does; a click on an empty workspace only toggles its (empty) disclosure
        ui.mouse_click(source)
        time.sleep(0.3)
        ui.drag(_row(ui, "workspace-row", "ui-ws-three"), _row(ui, "workspace-row", "ui-ws-one"),
                dst_offset=ABOVE_ROW, **ROW_DRAG)
        ctl.wait_tree(lambda tree: _workspace_names(tree, names) == ["ui-ws-three", "ui-ws-one", "ui-ws-two"])
    finally:
        for wid in wids:
            ctl.request("workspace.delete", wid)


def _split_session(ctl, ui):
    """A new selected session split side by side, the sidebar hidden so the panes span the window's width."""
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("sidebar", mode="hide"))
    wait_until(lambda: not assert_ok(ctl.request("window.list"))["windows"][0]["sidebarVisible"])
    assert_ok(ctl.request("session.split", sid, mode="on", axis="vertical"))
    ctl.wait_mounted(sid, "left", "right")
    return sid


def _ratio(ctl, sid):
    return find_session(ctl.tree(), sid).get("splitRatio", 0.5)


def _divider_offsets(ui, window, ratio):
    """Window offsets on the divider of a side-by-side split at `ratio`, first the computed one, then its neighbours:
    the panes span the window's width, so the divider sits at `ratio` of it."""
    _, _, width, _ = ui.frame(window)
    return [((width * ratio + delta) / width, 0.6) for delta in DIVIDER_OFFSETS]


@pytest.mark.covers("field:tree.session.splitRatio", "behavior:split.divider-drag")
def test_dragging_the_divider_moves_the_split_ratio(ctl, ui):
    """`SplitRatioUITests.testDividerDragPersistsTheRatio`: a drag right by 0.08 of the window's width (upstream: 60
    points) moves the ratio past 0.55."""
    sid = _split_session(ctl, ui)
    try:
        assert _ratio(ctl, sid) == pytest.approx(0.5, abs=0.004)
        window = ui.window()
        distance = ui.frame(window)[2] * DIVIDER_DRAG_FRACTION
        for offset in _divider_offsets(ui, window, 0.5):
            ui.drag_by(window, distance, 0, offset=offset, **DIVIDER_DRAG)
            try:
                wait_until(lambda: _ratio(ctl, sid) > 0.55, timeout=2)
                break
            except WaitTimeout:
                continue
        else:
            pytest.fail(f"no drag near the divider moved the ratio: {_ratio(ctl, sid)}")
    finally:
        ctl.request("session.close", sid)
        ctl.request("sidebar", mode="show")


@pytest.mark.covers("session.resize", "field:tree.session.splitRatio", "behavior:split.divider-double-click")
def test_double_clicking_the_divider_restores_the_even_split(ctl, ui):
    """`ControlOverlaySplitUITests.testDoubleClickOnDividerRestoresEvenSplit`: off center at 0.7, a double-click on
    the divider brings it back to 0.5."""
    sid = _split_session(ctl, ui)
    try:
        assert_ok(ctl.request("session.resize", sid, ratio=0.7))
        ctl.wait_tree(lambda tree: find_session(tree, sid).get("splitRatio") == pytest.approx(0.7, abs=0.004))
        window = ui.window()
        for offset in _divider_offsets(ui, window, 0.7):
            ui.mouse_double_click(window, offset=offset)
            try:
                wait_until(lambda: _ratio(ctl, sid) == pytest.approx(0.5, abs=0.004), timeout=2)
                break
            except WaitTimeout:
                continue
        else:
            pytest.fail(f"no double-click near the divider restored the even split: {_ratio(ctl, sid)}")
    finally:
        ctl.request("session.close", sid)
        ctl.request("sidebar", mode="show")


@pytest.mark.covers("ui:workspace-row", "behavior:sidebar.scroll")
def test_the_wheel_scrolls_an_overflowing_sidebar(ctl, ui):
    """Enough workspace rows to overflow the sidebar; three wheel ticks down over it move a row up."""
    window = ui.window()
    _, top, _, height = ui.frame(window)
    first = _workspace(ctl, "ui-scroll-00")
    wids = [first]
    try:
        row_height = ui.frame(_row(ui, "workspace-row", "ui-scroll-00"))[3]
        count = int(height / max(row_height, 8)) + 10
        wids += [_workspace(ctl, f"ui-scroll-{index:02}") for index in range(1, count)]
        # a row in the middle of the window, which stays in view after a short scroll
        middle = top + height / 2

        def middle_row():
            rows = [row for row in ui.all("workspace-row") if ui.text(row).startswith("ui-scroll-")]
            framed = [(ui.frame(row), row) for row in rows]
            shown = [(frame, row) for frame, row in framed if top < frame[1] < middle]
            return max(shown, key=lambda item: item[0][1]) if shown else None

        (x, y, _, _), row = wait_until(middle_row, timeout=ui.timeout)
        ui.scroll(row, 3)
        wait_until(lambda: ui.frame(row)[1] < y - 1, timeout=ui.timeout)
    finally:
        for wid in wids:
            ctl.request("workspace.delete", wid)


@pytest.mark.covers("ui:session-row", "session.flag", "behavior:sidebar.row-context-menu")
def test_a_session_row_context_menu_flags_the_row(ctl, ui):
    """A right click on a row opens its menu; Flag flags that session (`FlaggedViewUITests`, `ReorderUITests`)."""
    wid = _workspace(ctl, "ui-menu-session")
    try:
        sid = _named_sessions(ctl, wid, ["ui-menu-row"])["ui-menu-row"]
        ui.mouse_click(_row(ui, "session-row", "ui-menu-row"), button="right")
        ui.click(ui.menu_item("Flag"))
        ctl.wait_tree(lambda tree: find_session(tree, sid).get("flagged") is True)
    finally:
        ctl.request("session.flag", mode="clear")
        ctl.request("workspace.delete", wid)


@pytest.mark.covers("ui:workspace-row", "session.new", "behavior:sidebar.row-context-menu")
def test_a_workspace_row_context_menu_opens_a_session_in_it(ctl, ui):
    """New Session in a workspace row's menu opens a session in that workspace (`SidebarUITests`)."""
    wid = _workspace(ctl, "ui-menu-workspace")
    try:
        ui.mouse_click(_row(ui, "workspace-row", "ui-menu-workspace"), button="right")
        ui.click(ui.menu_item("New Session"))
        ctl.wait_tree(lambda tree: len(sessions(tree, wid)) == 1)
    finally:
        ctl.request("workspace.delete", wid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("ui:session-row", "session.tint", "behavior:sidebar.row-context-menu")
def test_a_hex_tinted_rows_menu_lists_its_custom_entry(ctl, ui):
    """A hex tint shows in the row's Tint submenu as "Custom #3a7bd5", and a palette name picked there replaces it,
    keeping the placement (`WorkspaceSidebarTintMenuTests.testAHexTintShowsACheckedCustomEntryThatANameReplaces`)."""
    wid = _workspace(ctl, "ui-menu-hex")
    try:
        sid = _named_sessions(ctl, wid, ["ui-hex-row"])["ui-hex-row"]
        assert_ok(ctl.request("session.tint", sid, mode="set", tint="#3A7BD5", tintOn=["name"]))
        ui.mouse_click(_row(ui, "session-row", "ui-hex-row"), button="right")
        ui.click(ui.menu_item("Tint"))
        ui.menu_item("Custom #3a7bd5")
        ui.click(ui.menu_item("Mint"))
        ctl.wait_tree(lambda tree: (find_session(tree, sid).get("tint"), find_session(tree, sid).get("tintOn"))
                      == ("mint", ["name"]))
    finally:
        ctl.request("workspace.delete", wid)
