"""Workspace commands: effect, `tree` read-back and events. The module shares one instance, so every test
builds the workspaces it needs and reads the rest of the tree relative to what is already there."""

import pytest
from agterm_conformance.match import (
    active_session_id,
    active_workspace_id,
    assert_absent,
    assert_error,
    assert_ok,
    assert_subset,
    session_ids,
    window_ids,
    workspace_ids,
    workspaces,
)

UNKNOWN = "zzz"


def _new_workspace(ctl, sessions=0, **args):
    wid = assert_ok(ctl.request("workspace.new", **args))["id"]
    sids = [assert_ok(ctl.request("session.new", workspace=wid))["id"] for _ in range(sessions)]
    return wid, sids


def _node(source, wid):
    (node,) = [node for node in workspaces(source) if node["id"] == wid]
    return node


def _window(ctl):
    (window,) = window_ids(ctl.request("window.list"))
    return window


def _settled_baseline(ctl, cursor):
    """`tree.changed` is debounced per window: wait out the one the preparation since `cursor` scheduled, so
    a tree.changed after the returned baseline can only come from the action under test."""
    ctl.wait_event(cursor, "tree.changed")
    return ctl.events_baseline()


def _clear_focus(ctl):
    for node in workspaces(ctl.tree()):
        if node.get("focused"):
            assert_ok(ctl.request("workspace.focus", node["id"], mode="off"))
    assert ctl.tree()["tree"]["workspaceFilter"] is False


def _focus_state(ctl):
    tree = ctl.tree()
    return {node["id"] for node in workspaces(tree) if node.get("focused")}, tree["tree"]["workspaceFilter"]


def _reordered(ids, wid, to):
    ids = list(ids)
    index = ids.index(wid)
    ids.pop(index)
    destination = {"up": max(index - 1, 0), "down": min(index + 1, len(ids)), "top": 0, "bottom": len(ids)}[to]
    ids.insert(destination, wid)
    return ids


@pytest.mark.covers(
    "workspace.new",
    "field:result.id",
    "field:tree.workspace.name",
    "field:tree.workspace.active",
    "field:tree.workspace.sessions",
    "event:tree.changed",
)
def test_new_appends_an_empty_current_workspace(ctl):
    before = ctl.tree()
    selected = active_session_id(before)
    cursor = ctl.events_baseline()

    wid = assert_ok(ctl.request("workspace.new", name="alpha"))["id"]
    tree = ctl.tree()
    assert workspace_ids(tree) == [*workspace_ids(before), wid]
    node = _node(tree, wid)
    assert_subset(node, {"name": "alpha", "active": True, "sessions": []})
    assert_absent(node, "collapsed", "focused")
    assert active_workspace_id(tree) == wid
    assert active_session_id(tree) == selected
    ctl.wait_event(cursor, "tree.changed", {"window": _window(ctl)})


@pytest.mark.covers("workspace.new", "field:tree.workspace.name")
@pytest.mark.parametrize(
    "name",
    [
        pytest.param(None, id="omitted"),
        pytest.param("   ", id="blank"),
        pytest.param("  beta  ", id="padded"),
    ],
)
def test_new_workspace_name(ctl, name):
    default = f"workspace {len(workspaces(ctl.tree())) + 1}"
    wid, _ = _new_workspace(ctl, **({} if name is None else {"name": name}))
    assert _node(ctl.tree(), wid)["name"] == ((name or "").strip() or default)


@pytest.mark.covers("workspace.new", "field:tree.workspace.collapsed", "field:tree.workspace.focused")
def test_new_collapsed_workspace(ctl):
    _clear_focus(ctl)
    current, _ = _new_workspace(ctl, sessions=1)
    assert_ok(ctl.request("workspace.focus", current, mode="on"))

    collapsed = assert_ok(ctl.request("workspace.new", collapsed=True))["id"]
    assert_subset(_node(ctl.tree(), collapsed), {"collapsed": True, "sessions": []})

    plain = assert_ok(ctl.request("workspace.new", collapsed=False))["id"]
    tree = ctl.tree()
    assert_absent(_node(tree, plain), "collapsed")
    assert active_workspace_id(tree) == plain
    assert plain in _focus_state(ctl)[0]
    _clear_focus(ctl)


def _create_collapsed(ctl):
    return assert_ok(ctl.request("workspace.new", collapsed=True))["id"]


def _create_by_background_session(ctl):
    name = f"background {len(workspaces(ctl.tree()))}"
    sid = assert_ok(ctl.request("session.new", workspaceName=name, createWorkspace=True, noSelect=True))["id"]
    (node,) = [node for node in workspaces(ctl.tree()) if node["name"] == name]
    assert session_ids(ctl.tree(), node["id"]) == [sid]
    return node["id"]


@pytest.mark.covers("workspace.new", "session.new", "field:tree.workspace.active", "field:tree.workspace.focused")
@pytest.mark.parametrize(
    "create",
    [
        pytest.param(_create_collapsed, id="collapsed"),
        pytest.param(_create_by_background_session, id="session-no-select"),
    ],
)
def test_background_create_keeps_the_current_workspace(ctl, create):
    _clear_focus(ctl)
    current, (selected,) = _new_workspace(ctl, sessions=1)
    assert_ok(ctl.request("workspace.focus", current, mode="on"))

    try:
        create(ctl)
        tree = ctl.tree()
        assert active_workspace_id(tree) == current
        assert active_session_id(tree) == selected
        assert _focus_state(ctl) == ({current}, True)
        assert assert_ok(ctl.request("session.new"))["id"] in session_ids(ctl.tree(), current)
    finally:
        _clear_focus(ctl)


@pytest.mark.covers("workspace.select", "field:tree.workspace.active", "event:session.selected")
def test_select_moves_to_the_first_session(ctl):
    wid, (first, _) = _new_workspace(ctl, sessions=2)
    _, (elsewhere,) = _new_workspace(ctl, sessions=1)
    cursor = ctl.events_baseline()

    assert assert_ok(ctl.request("workspace.select", wid))["id"] == wid
    tree = ctl.tree()
    assert active_workspace_id(tree) == wid
    assert active_session_id(tree) == first
    ctl.wait_event(
        cursor,
        "session.selected",
        {"window": _window(ctl), "workspace": wid, "session": first, "payload": {"previous": elsewhere}},
    )


@pytest.mark.covers("workspace.select", "field:tree.workspace.active")
def test_select_empty_workspace_keeps_the_selection(ctl):
    empty, _ = _new_workspace(ctl)
    _, (selected,) = _new_workspace(ctl, sessions=1)

    assert assert_ok(ctl.request("workspace.select", empty))["id"] == empty
    tree = ctl.tree()
    assert active_workspace_id(tree) == empty
    assert active_session_id(tree) == selected


@pytest.mark.covers("workspace.go", "field:tree.workspace.active")
def test_go_steps_through_workspaces_and_wraps(ctl):
    _clear_focus(ctl)
    _new_workspace(ctl, sessions=1)
    _new_workspace(ctl, sessions=1)
    ids = workspace_ids(ctl.tree())
    assert_ok(ctl.request("workspace.select", ids[-1]))

    current = ids[-1]
    for direction, step in [("next", 1), ("next", 1), ("prev", -1), ("previous", -1), ("prev", -1)]:
        expected = ids[(ids.index(current) + step) % len(ids)]
        selected = active_session_id(ctl.tree())
        assert assert_ok(ctl.request("workspace.go", to=direction))["id"] == expected, direction
        tree = ctl.tree()
        assert active_workspace_id(tree) == expected
        node_sessions = session_ids(tree, expected)
        assert active_session_id(tree) == (node_sessions[0] if node_sessions else selected)
        current = expected


@pytest.mark.covers("workspace.go", "workspace.focus", "workspace.filter")
def test_go_stays_inside_the_focus_filter(ctl):
    _clear_focus(ctl)
    first, (first_session,) = _new_workspace(ctl, sessions=1)
    _new_workspace(ctl, sessions=1)
    last, (last_session,) = _new_workspace(ctl, sessions=1)
    assert_ok(ctl.request("workspace.focus", first, mode="add"))
    assert_ok(ctl.request("workspace.focus", last, mode="add"))
    assert_ok(ctl.request("workspace.filter", mode="on"))
    assert_ok(ctl.request("workspace.select", first))

    assert assert_ok(ctl.request("workspace.go", to="next"))["id"] == last
    assert active_session_id(ctl.tree()) == last_session
    assert assert_ok(ctl.request("workspace.go", to="next"))["id"] == first
    assert active_session_id(ctl.tree()) == first_session
    _clear_focus(ctl)


@pytest.mark.covers("workspace.go")
@pytest.mark.parametrize("to", [None, "sideways", "first"])
def test_go_requires_a_direction(ctl, to):
    args = {} if to is None else {"to": to}
    assert_error(ctl.request("workspace.go", **args), "workspace.go requires --to next|prev")


@pytest.mark.covers("workspace.go", "workspace.delete")
def test_a_lone_workspace_has_nowhere_to_go_and_cannot_be_deleted(app, ctl):
    (wid,) = workspace_ids(ctl.tree())
    assert_error(ctl.request("workspace.go", to="next"), "no other workspace to navigate to")
    assert_error(ctl.request("workspace.delete", wid), "cannot delete last workspace")
    assert workspace_ids(ctl.tree()) == [wid]


@pytest.mark.covers("workspace.rename", "field:tree.workspace.name", "event:tree.changed")
def test_rename(ctl):
    cursor = ctl.events_baseline()
    wid, _ = _new_workspace(ctl, name="before")
    cursor = _settled_baseline(ctl, cursor)

    assert assert_ok(ctl.request("workspace.rename", wid, name="  after  "))["id"] == wid
    assert _node(ctl.tree(), wid)["name"] == "after"
    ctl.wait_event(cursor, "tree.changed", {"window": _window(ctl)})


@pytest.mark.covers("workspace.rename")
@pytest.mark.parametrize("name", [None, "   "])
def test_rename_requires_a_name(ctl, name):
    args = {} if name is None else {"name": name}
    assert_error(ctl.request("workspace.rename", "active", **args), "workspace.rename requires a name")


@pytest.mark.covers("workspace.delete", "event:session.closed", "event:session.selected", "event:tree.changed")
def test_delete_closes_the_workspace_sessions(ctl):
    window = _window(ctl)
    cursor = ctl.events_baseline()
    wid, (first, last) = _new_workspace(ctl, sessions=2)
    cursor = _settled_baseline(ctl, cursor)

    assert assert_ok(ctl.request("workspace.delete", wid))["id"] == wid
    tree = ctl.tree()
    assert wid not in workspace_ids(tree)
    assert not {first, last} & set(session_ids(tree))
    selected = active_session_id(tree)
    for sid in (first, last):
        ctl.wait_event(cursor, "session.closed", {"window": window, "workspace": wid, "session": sid})
    ctl.wait_event(cursor, "session.selected", {"session": selected, "payload": {"previous": last}})
    ctl.wait_event(cursor, "tree.changed", {"window": window})


@pytest.mark.covers("workspace.delete", "event:tree.changed")
def test_delete_empty_workspace(ctl):
    cursor = ctl.events_baseline()
    wid, _ = _new_workspace(ctl)
    cursor = _settled_baseline(ctl, cursor)

    assert assert_ok(ctl.request("workspace.delete", wid))["id"] == wid
    assert wid not in workspace_ids(ctl.tree())
    ctl.wait_event(cursor, "tree.changed", {"window": _window(ctl)})


@pytest.mark.covers("workspace.move", "event:tree.changed")
def test_move_reorders_among_siblings(ctl):
    cursor = ctl.events_baseline()
    _new_workspace(ctl)
    _new_workspace(ctl)
    wid, _ = _new_workspace(ctl)
    cursor = _settled_baseline(ctl, cursor)

    ids = workspace_ids(ctl.tree())
    # Each end gets one step past it, which leaves the order alone.
    for to in ["up", "up", "top", "up", "down", "bottom", "down", "top"]:
        assert assert_ok(ctl.request("workspace.move", wid, to=to))["id"] == wid, to
        ids = _reordered(ids, wid, to)
        assert workspace_ids(ctl.tree()) == ids, to
    ctl.wait_event(cursor, "tree.changed", {"window": _window(ctl)})


@pytest.mark.covers("workspace.move")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "workspace.move requires --to", id="missing"),
        pytest.param({"to": "left"}, "workspace.move --to must be up|down|top|bottom", id="unknown"),
    ],
)
def test_move_requires_a_direction(ctl, args, error):
    assert_error(ctl.request("workspace.move", "active", **args), error)


@pytest.mark.covers("workspace.focus", "field:tree.workspace.focused", "field:tree.workspaceFilter")
def test_focus_modes(ctl):
    _clear_focus(ctl)
    first, (first_session,) = _new_workspace(ctl, sessions=1)
    second, _ = _new_workspace(ctl, sessions=1)

    steps = [
        # Narrowing the view moves the selection into it.
        ("on", first, ({first}, True)),
        ("add", second, ({first, second}, True)),
        ("off", first, ({second}, True)),
        ("off", second, (set(), False)),
        (None, first, ({first}, True)),
        ("toggle", first, (set(), False)),
        ("add", first, ({first}, False)),
    ]
    for mode, wid, expected in steps:
        args = {} if mode is None else {"mode": mode}
        assert assert_ok(ctl.request("workspace.focus", wid, **args))["id"] == wid, mode
        assert _focus_state(ctl) == expected, mode
        if mode == "on":
            assert active_session_id(ctl.tree()) == first_session
    assert_absent(_node(ctl.tree(), second), "focused")
    _clear_focus(ctl)


@pytest.mark.covers("workspace.focus")
def test_focus_rejects_an_unknown_mode(ctl):
    before = ctl.tree()
    assert_error(ctl.request("workspace.focus", "active", mode="pin"), "invalid focus mode: pin (on|off|toggle|add)")
    assert ctl.tree()["tree"]["workspaceFilter"] == before["tree"]["workspaceFilter"]


@pytest.mark.covers("workspace.filter", "field:tree.workspaceFilter", "field:tree.workspace.focused")
def test_filter_switches_the_view_without_touching_the_marks(ctl):
    _clear_focus(ctl)
    wid, _ = _new_workspace(ctl, sessions=1)

    assert_ok(ctl.request("workspace.filter", mode="on"))
    assert _focus_state(ctl) == (set(), False), "an empty set cannot be filtered to"
    assert_ok(ctl.request("workspace.focus", wid, mode="add"))
    for mode, enabled in [("on", True), ("toggle", False), (None, True), ("off", False)]:
        assert_ok(ctl.request("workspace.filter", **({} if mode is None else {"mode": mode})))
        assert _focus_state(ctl) == ({wid}, enabled), mode
    _clear_focus(ctl)


@pytest.mark.covers("workspace.filter")
def test_filter_rejects_an_unknown_mode(ctl):
    assert_error(ctl.request("workspace.filter", mode="show"), "invalid workspace filter mode: show")


def _flags(ctl, wid):
    return [node["flagged"] for node in _node(ctl.tree(), wid)["sessions"]]


@pytest.mark.covers("workspace.flag", "field:tree.session.flagged")
def test_flag_sets_every_session_to_one_value(ctl):
    wid, sids = _new_workspace(ctl, sessions=3)
    other, (stranger,) = _new_workspace(ctl, sessions=1)
    try:
        assert_ok(ctl.request("session.flag", sids[0], mode="on"))
        assert_ok(ctl.request("session.flag", stranger, mode="on"))
        # `toggle` keys on every session being flagged: a mixed workspace ends up all flagged, not inverted.
        for mode, flagged in [("toggle", True), (None, False), ("on", True), ("on", True), ("off", False)]:
            args = {} if mode is None else {"mode": mode}
            assert assert_ok(ctl.request("workspace.flag", wid, **args)) == {"id": wid}, mode
            assert _flags(ctl, wid) == [flagged] * len(sids), mode
            assert _flags(ctl, other) == [True], "another workspace's sessions are not touched"
    finally:
        ctl.request("workspace.flag", wid, mode="off")
        ctl.request("workspace.flag", other, mode="off")


@pytest.mark.covers("workspace.flag", "sidebar.mode")
def test_flag_covers_the_sessions_the_flagged_view_hides(ctl):
    top = ctl.tree()["tree"]
    wid, sids = _new_workspace(ctl, sessions=2)
    try:
        assert_ok(ctl.request("session.flag", sids[0], mode="on"))
        assert_ok(ctl.request("sidebar.mode", mode="flagged"))
        assert_ok(ctl.request("sidebar.flagged-layout", mode="tree"))
        assert assert_ok(ctl.request("workspace.flag", wid, mode="toggle")) == {"id": wid}
        assert _flags(ctl, wid) == [True, True], "the session the flagged view hid is flagged too"
        assert_ok(ctl.request("workspace.flag", wid, mode="toggle"))
        assert _flags(ctl, wid) == [False, False]
    finally:
        ctl.request("workspace.flag", wid, mode="off")
        ctl.request("sidebar.mode", mode=top["sidebarMode"])
        ctl.request("sidebar.flagged-layout", mode=top["sidebarFlaggedLayout"])


@pytest.mark.covers("workspace.flag")
def test_flag_rejects_an_unknown_mode(ctl):
    wid, sids = _new_workspace(ctl, sessions=1)
    assert_error(ctl.request("workspace.flag", wid, mode="clear"), "invalid workspace flag mode: clear")
    assert _flags(ctl, wid) == [False] * len(sids)


@pytest.mark.covers("workspace.collapse", "workspace.expand", "field:tree.workspace.collapsed")
def test_collapse_and_expand(ctl):
    wid, _ = _new_workspace(ctl, sessions=1)
    for cmd, collapsed in [
        ("workspace.collapse", True),
        ("workspace.collapse", True),
        ("workspace.expand", False),
        ("workspace.expand", False),
    ]:
        assert assert_ok(ctl.request(cmd, wid))["id"] == wid, cmd
        node = _node(ctl.tree(), wid)
        if collapsed:
            assert node["collapsed"] is True, cmd
        else:
            assert_absent(node, "collapsed")


@pytest.mark.since("0.39.1")
@pytest.mark.covers("workspace.tint", "workspace.new", "field:tree.workspace.tint", "field:tree.workspace.tintOn")
def test_workspace_tint_colors_only_its_row(ctl):
    wid, (sid,) = _new_workspace(ctl, sessions=1, tint="red", tintOn=["name"])
    node = _node(ctl.tree(), wid)
    assert (node["tint"], node["tintOn"]) == ("red", ["name"])
    assert_absent(node["sessions"][0], "tint", "tintOn")

    # A set without a placement recolors and keeps it; one with a placement moves it.
    assert assert_ok(ctl.request("workspace.tint", wid, mode="set", tint="Purple"))["id"] == wid
    node = _node(ctl.tree(), wid)
    assert (node["tint"], node["tintOn"]) == ("purple", ["name"])
    assert_ok(ctl.request("workspace.tint", wid, mode="set", tint="gray", tintOn=["icon", "name"]))
    node = _node(ctl.tree(), wid)
    assert (node["tint"], node["tintOn"]) == ("gray", ["icon", "name"])
    assert [session["id"] for session in node["sessions"]] == [sid]
    assert_absent(node["sessions"][0], "tint", "tintOn")

    assert_error(
        ctl.request("workspace.tint", wid, mode="clear", tintOn=["icon"]),
        "workspace.tint clear takes no color or placement",
    )
    assert_error(ctl.request("workspace.tint", wid, mode="set"), "workspace.tint set requires a color")
    assert _node(ctl.tree(), wid)["tint"] == "gray"
    assert_ok(ctl.request("workspace.tint", wid, mode="clear"))
    assert_absent(_node(ctl.tree(), wid), "tint", "tintOn")
    assert_error(ctl.request("workspace.tint", UNKNOWN, mode="clear"), f"no such workspace: {UNKNOWN}")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers(
    "workspace.select",
    "workspace.rename",
    "workspace.delete",
    "workspace.move",
    "workspace.focus",
    "workspace.collapse",
    "workspace.expand",
    "workspace.flag",
)
@pytest.mark.parametrize(
    ("cmd", "args"),
    [
        ("workspace.select", {}),
        ("workspace.rename", {"name": "renamed"}),
        ("workspace.delete", {}),
        ("workspace.move", {"to": "up"}),
        ("workspace.focus", {"mode": "on"}),
        ("workspace.collapse", {}),
        ("workspace.expand", {}),
        ("workspace.flag", {"mode": "on"}),
    ],
)
def test_unknown_workspace_target(ctl, cmd, args):
    assert_error(ctl.request(cmd, UNKNOWN, **args), f"no such workspace: {UNKNOWN}")
