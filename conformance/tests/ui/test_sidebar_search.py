"""The sidebar's session search (capability `sidebar-search`, a Rust-only extension; its ids are under `rust:` in
ui-ids.yaml): Search Sessions…'s chord gives the field the keyboard, typing narrows the rows to the sessions whose name
holds the text, Return opens the first match, a text matching nothing shows the empty hint with its Clear search link,
and Escape empties the field. The tree reads the field's text back as `sidebarSearch`. The test leaves the shared
instance's sessions and search as it found them."""

import pytest
from agterm_conformance.match import active_session_id, assert_ok
from agterm_conformance.wait import wait_until


def _new(ctl, workspace, name):
    sid = assert_ok(ctl.request("session.new", workspace=workspace))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.rename", sid, name=name))
    return sid


def _rows(ui):
    return [ui.text(element) for element in ui.all("session-row")]


def _search_of(tree):
    """The tree's `sidebarSearch`, None while it is omitted."""
    return tree["tree"].get("sidebarSearch")


def _searched(ctl, text):
    ctl.wait_tree(lambda tree: _search_of(tree) == text)


def _listed_chord(ctl):
    listing = assert_ok(ctl.request("keymap.list"))["keymap"]
    (action,) = [action for action in listing["actions"] if action["action"] == "focus_sidebar_search"]
    return action.get("chord")


@pytest.fixture(autouse=True)
def _search_chord_bound(ctl):
    """On an OS whose table ships no chord for Search Sessions… (macOS, as upstream's table has no such action), binds
    it to the chord Linux and Windows ship, and puts the instance's `keymap.conf` back afterwards."""
    if _listed_chord(ctl) is not None:
        yield
        return
    path = ctl.instance.state_dir / "config" / "keymap.conf"
    before = path.read_text() if path.exists() else None
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((before or "") + "\nmap ctrl+alt+shift+f focus_sidebar_search\n")
    assert_ok(ctl.request("keymap.reload"))
    try:
        yield
    finally:
        if before is None:
            path.unlink(missing_ok=True)
        else:
            path.write_text(before)
        assert_ok(ctl.request("keymap.reload"))


def _chord(ctl):
    """Search Sessions…'s chord: the one this OS ships, else the one `_search_chord_bound` binds."""
    chord = _listed_chord(ctl)
    assert chord, "focus_sidebar_search has no chord"
    return chord


@pytest.mark.requires("sidebar-search", reason="upstream's sidebar has no search")
@pytest.mark.covers(
    "ui:sidebar-search",
    "ui:sidebar-search-clear",
    "ui:sidebar-search-empty",
    "ui:sidebar-search-empty-clear",
    "ui:session-row",
)
def test_the_search_field_narrows_the_rows_and_return_opens_the_first_match(ctl, ui):
    wid = assert_ok(ctl.request("workspace.new", name="ui-search-ws"))["id"]
    try:
        _new(ctl, wid, "ui-search-alpha")
        beta = _new(ctl, wid, "ui-search-beta")
        other = _new(ctl, wid, "ui-plain")
        assert_ok(ctl.request("session.select", other))
        chord = _chord(ctl)

        ui.press(chord)
        ui.find("sidebar-search", role="text-field")
        ui.type_text("SEARCH-B")
        _searched(ctl, "SEARCH-B")
        wait_until(lambda: "ui-search-beta" in _rows(ui) and "ui-search-alpha" not in _rows(ui))
        assert "ui-plain" not in _rows(ui), "a session that does not match hides"
        assert active_session_id(ctl.tree()) == other, "typing never moves the selection"

        ui.press("return")
        ctl.wait_tree(lambda tree: active_session_id(tree) == beta)
        assert _search_of(ctl.tree()) == "SEARCH-B", "Return keeps the search"

        # the chord selects the field's text, so typing replaces it
        ui.press(chord)
        ui.type_text("zz-none")
        _searched(ctl, "zz-none")
        ui.find("sidebar-search-empty")
        ui.click(ui.find("sidebar-search-empty-clear"))
        _searched(ctl, None)
        ui.absent("sidebar-search-empty")

        ui.type_text("alpha")
        _searched(ctl, "alpha")
        ui.find("sidebar-search-clear")
        wait_until(lambda: "ui-search-alpha" in _rows(ui) and "ui-search-beta" not in _rows(ui))
        ui.press("escape")
        _searched(ctl, None)
        ui.absent("sidebar-search-clear")
        wait_until(lambda: {"ui-search-alpha", "ui-search-beta", "ui-plain"} <= set(_rows(ui)))
        assert active_session_id(ctl.tree()) == beta, "Escape keeps the selection"
    finally:
        if _search_of(ctl.tree()):
            ui.press(_chord(ctl), "escape")
        assert_ok(ctl.request("workspace.delete", wid))


def _visible(ctl, shown):
    ctl.wait_tree(lambda tree: tree["tree"]["sidebarVisible"] is shown)


def _reset(ctl, ui):
    """Empty a search the test left and give the keyboard back to the terminal."""
    if _search_of(ctl.tree()):
        ui.press(_chord(ctl), "escape")
        _searched(ctl, None)


@pytest.mark.requires("sidebar-search", reason="upstream's sidebar has no search")
@pytest.mark.covers("ui:sidebar-search", "ui:command-palette", "ui:session-row")
def test_the_menu_and_the_palette_show_a_hidden_sidebar_and_the_text_outlives_hiding_it(ctl, ui):
    """View ▸ Search Sessions… and the palette's Search Sessions… show a hidden sidebar and give the field the keyboard
    with its text selected; the text, and the rows it keeps, stay while the sidebar hides and shows again."""
    wid = assert_ok(ctl.request("workspace.new", name="ui-search-menu-ws"))["id"]
    try:
        _new(ctl, wid, "ui-search-menu-kept")
        _new(ctl, wid, "ui-search-menu-other")
        assert_ok(ctl.request("sidebar", mode="hide"))
        _visible(ctl, False)

        ui.menu("View", "Search Sessions…")
        _visible(ctl, True)
        ui.type_text("menu-kept")
        _searched(ctl, "menu-kept")
        wait_until(lambda: "ui-search-menu-kept" in _rows(ui) and "ui-search-menu-other" not in _rows(ui))

        assert_ok(ctl.request("sidebar", mode="hide"))
        _visible(ctl, False)
        assert _search_of(ctl.tree()) == "menu-kept", "hiding the sidebar keeps the text"
        assert_ok(ctl.request("sidebar", mode="show"))
        _visible(ctl, True)
        assert _search_of(ctl.tree()) == "menu-kept", "showing the sidebar keeps the text"
        wait_until(lambda: "ui-search-menu-kept" in _rows(ui) and "ui-search-menu-other" not in _rows(ui))

        assert_ok(ctl.request("sidebar", mode="hide"))
        _visible(ctl, False)
        ui.menu("Navigate", "Command Palette")
        ui.find("command-palette", role="text-field")
        ui.type_text("Search Sessions")
        ui.press("return")
        ui.absent("command-palette")
        _visible(ctl, True)
        # the field's text is selected, so typing replaces it
        ui.type_text("menu-other")
        _searched(ctl, "menu-other")
        wait_until(lambda: "ui-search-menu-other" in _rows(ui) and "ui-search-menu-kept" not in _rows(ui))
    finally:
        assert_ok(ctl.request("sidebar", mode="show"))
        _reset(ctl, ui)
        assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.requires("sidebar-search", reason="upstream's sidebar has no search")
@pytest.mark.covers("ui:sidebar-search", "ui:sidebar-search-clear", "ui:session-row")
def test_up_and_down_move_the_match_return_opens_and_the_clear_button_keeps_the_keyboard(ctl, ui):
    """Down and Up move the cursor among the matches and Return opens the one it marks; the field's clear button
    empties it and the next keys still type into it."""
    wid = assert_ok(ctl.request("workspace.new", name="ui-search-ring-ws"))["id"]
    try:
        _new(ctl, wid, "ui-search-ring-one")
        two = _new(ctl, wid, "ui-search-ring-two")
        _new(ctl, wid, "ui-search-ring-three")
        plain = _new(ctl, wid, "ui-plain-row")
        assert_ok(ctl.request("session.select", plain))

        ui.press(_chord(ctl))
        ui.find("sidebar-search", role="text-field")
        ui.type_text("search-ring")
        _searched(ctl, "search-ring")
        wait_until(lambda: "ui-search-ring-three" in _rows(ui) and "ui-plain-row" not in _rows(ui))
        ui.press("down", "down", "up")
        assert active_session_id(ctl.tree()) == plain, "the cursor never moves the selection"
        ui.press("return")
        ctl.wait_tree(lambda tree: active_session_id(tree) == two)

        ui.press(_chord(ctl))
        ui.type_text("ring-three")
        _searched(ctl, "ring-three")
        ui.click(ui.find("sidebar-search-clear"))
        _searched(ctl, None)
        ui.type_text("ring-one")
        _searched(ctl, "ring-one")
        assert active_session_id(ctl.tree()) == two
    finally:
        _reset(ctl, ui)
        assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.requires("sidebar-search", reason="upstream's sidebar has no search")
@pytest.mark.covers("ui:sidebar-search", "ui:session-row", "ui:workspace-row")
def test_the_search_keeps_grouped_matches_opens_folds_and_narrows_the_other_filters(adapter, ctl, ui):
    """A workspace with no match hides; a matching companion shows under its parent, inside a folded workspace and a
    folded parent, which fold again when the search ends; the focus filter and the flagged view still apply."""
    one = assert_ok(ctl.request("workspace.new", name="ui-grp-ws-one"))["id"]
    two = assert_ok(ctl.request("workspace.new", name="ui-grp-ws-two"))["id"]
    try:
        parent = _new(ctl, one, "ui-grp-parent")
        assert_ok(ctl.request("session.companion.new", parent, name="ui-grp-kid", noSelect=True))
        _new(ctl, two, "ui-grp-other")
        _new(ctl, two, "ui-grp-kid-two")
        assert_ok(ctl.request("session.collapse", parent))
        assert_ok(ctl.request("workspace.collapse", one))

        def workspaces():
            return [ui.text(element) for element in ui.all("workspace-row")]

        def kid_shown():
            if adapter.has("ui-string-values"):
                return "ui-grp-kid" in _rows(ui)
            return any(row.name == "companion" for row in ui.all("session-row"))

        wait_until(lambda: not kid_shown() and "ui-grp-parent" not in _rows(ui))
        ui.press(_chord(ctl))
        ui.type_text("grp-kid")
        _searched(ctl, "grp-kid")
        wait_until(lambda: kid_shown() and "ui-grp-parent" in _rows(ui) and "ui-grp-kid-two" in _rows(ui))
        assert "ui-grp-other" not in _rows(ui), "a session that does not match hides"

        ui.press("delete", "delete", "delete")
        ui.type_text("other")
        _searched(ctl, "grp-other")
        wait_until(lambda: "ui-grp-other" in _rows(ui) and "ui-grp-ws-two" in workspaces())
        wait_until(lambda: "ui-grp-ws-one" not in workspaces())

        ui.press("escape")
        _searched(ctl, None)
        wait_until(lambda: "ui-grp-ws-one" in workspaces() and "ui-grp-parent" not in _rows(ui))
        assert not kid_shown(), "the folds come back when the search ends"

        assert_ok(ctl.request("workspace.focus", two, mode="on"))
        assert_ok(ctl.request("workspace.filter", mode="on"))
        ui.press(_chord(ctl))
        ui.type_text("grp-kid")
        _searched(ctl, "grp-kid")
        wait_until(lambda: "ui-grp-kid-two" in _rows(ui) and not kid_shown())
        assert "ui-grp-ws-one" not in workspaces(), "the focus filter still hides the workspace it leaves out"
        assert_ok(ctl.request("workspace.filter", mode="off"))
        assert_ok(ctl.request("workspace.focus", two, mode="off"))

        flagged = [s["id"] for s in ctl.tree()["tree"]["workspaces"] for s in s.get("sessions", [])
                   if s["name"] in ("ui-grp-other", "ui-grp-kid-two")]
        for sid in flagged:
            assert_ok(ctl.request("session.flag", sid, mode="on"))
        assert_ok(ctl.request("sidebar.mode", mode="flagged"))
        wait_until(lambda: any("ui-grp-kid-two" in row for row in _rows(ui)))
        assert not any("ui-grp-other" in row for row in _rows(ui)), "the search narrows the flagged view"
        assert not kid_shown(), "an unflagged match stays out of the flagged view"
    finally:
        ctl.request("sidebar.mode", mode="tree")
        ctl.request("session.flag", mode="clear")
        ctl.request("workspace.filter", mode="off")
        ctl.request("workspace.focus", two, mode="off")
        _reset(ctl, ui)
        assert_ok(ctl.request("workspace.delete", one))
        assert_ok(ctl.request("workspace.delete", two))
