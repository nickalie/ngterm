"""A click on a workspace row's icon folds the workspace at once, with the row-click setting off too, and a double-click
on the icon folds and opens it again instead of starting a rename (#952, `SidebarUITests`
`testWorkspaceIconClickTogglesWhenRowClickIsOff` and `testWorkspaceIconDoubleClickFoldsTwiceWithoutRename` v0.42.0)."""

import pytest
from agterm_conformance.drivers.base import ReadFailed
from agterm_conformance.match import assert_ok, workspaces
from agterm_conformance.wait import WaitTimeout, wait_until

pytestmark = pytest.mark.since("0.42.0")

# Upstream waits this long for a rename field that must not come.
NO_RENAME_WAIT = 2.0


def _workspace_with_a_session(ctl, name):
    wid = assert_ok(ctl.request("workspace.new", name=name))["id"]
    sid = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
    ctl.wait_mounted(sid, "left")
    return wid


def _collapsed(tree, wid):
    return next(node for node in workspaces(tree) if node["id"] == wid).get("collapsed", False)


def _icon(ui, name):
    """The icon of the workspace row whose text is `name`: every workspace row's icon shares its id."""

    def find():
        row = next((row for row in ui.all("workspace-row") if ui.text(row) == name), None)
        if row is None:
            return None
        _, row_y, _, row_height = ui.frame(row)

        def in_row(icon):
            _, y, _, height = ui.frame(icon)
            return row_y <= y + height / 2 <= row_y + row_height

        return next((icon for icon in ui.all("workspace-icon") if in_row(icon)), None)

    return wait_until(find, timeout=ui.timeout)


def _rename_field(ui):
    """The rename field, if one shows; a read the folding outline invalidates mid-way is read again."""
    try:
        return next(iter(ui.all("edit-field")), None)
    except ReadFailed:
        return None


@pytest.mark.covers("ui:workspace-icon", "ui:workspace-row", "field:tree.workspace.collapsed")
def test_a_workspace_icon_click_folds_it_with_the_row_click_setting_off(launch, ui_for):
    ctl = launch(settings={"workspaceRowClickExpands": False})
    ui = ui_for(ctl)
    wid = _workspace_with_a_session(ctl, "ui-icon-fold")
    ui.mouse_click(_icon(ui, "ui-icon-fold"))
    ctl.wait_tree(lambda tree: _collapsed(tree, wid) is True)
    ui.mouse_click(_icon(ui, "ui-icon-fold"))
    ctl.wait_tree(lambda tree: _collapsed(tree, wid) is False)


@pytest.mark.covers("ui:workspace-icon", "ui:edit-field", "field:tree.workspace.collapsed")
def test_a_workspace_icon_double_click_folds_twice_without_a_rename(launch, ui_for):
    ctl = launch()
    ui = ui_for(ctl)
    wid = _workspace_with_a_session(ctl, "ui-icon-double")
    ui.mouse_double_click(_icon(ui, "ui-icon-double"))
    with pytest.raises(WaitTimeout):
        wait_until(lambda: _rename_field(ui), timeout=NO_RENAME_WAIT)
    ctl.wait_tree(lambda tree: _collapsed(tree, wid) is False)
