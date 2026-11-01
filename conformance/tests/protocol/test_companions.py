"""Companion sessions: a session created under a parent, nested after it in the parent's workspace, folded with it and
moved with it. The module shares one instance, so every test builds the group it needs in a workspace of its own."""

import pytest
from agterm_conformance.match import (
    assert_absent,
    assert_error,
    assert_ok,
    assert_subset,
    find_session,
    session_ids,
)

UNKNOWN = "zzz"
PLACEMENT = "a companion takes no workspace or placement: it lands under its parent"
MOVE_REFUSED = "a companion moves only among its siblings"

pytestmark = pytest.mark.since("0.39.1")


def _workspace(ctl, sessions=1):
    wid = assert_ok(ctl.request("workspace.new"))["id"]
    return wid, [assert_ok(ctl.request("session.new", workspace=wid, cwd="/tmp"))["id"] for _ in range(sessions)]


def _companion(ctl, parent, **args):
    return assert_ok(ctl.request("session.companion.new", parent, noSelect=True, **args))["id"]


@pytest.mark.covers("session.companion.new", "field:result.parent", "field:tree.session.parent")
def test_a_companion_lands_under_its_parent_and_reads_back_its_parent(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    created = assert_ok(ctl.request("session.companion.new", parent, name="logs", noSelect=True))
    first = created["id"]
    assert created["parent"] == parent
    # a companion target means its parent, and a new companion goes after the group's last one
    second = _companion(ctl, first)
    tree = ctl.tree()
    assert session_ids(tree, wid) == [parent, first, second, after]
    assert_subset(find_session(tree, first), {"name": "logs", "parent": parent, "active": False})
    assert find_session(tree, second)["parent"] == parent
    # with no --cwd a companion starts in its parent's directory
    assert find_session(tree, second)["cwd"] == find_session(tree, parent)["cwd"]
    assert_absent(find_session(tree, parent), "parent", "collapsed", "content")
    assert_absent(find_session(tree, first), "collapsed", "content")
    # a top-level session inserted inside the group lands after it
    inserted = assert_ok(ctl.request("session.new", after=first, noSelect=True))["id"]
    assert session_ids(ctl.tree(), wid) == [parent, first, second, inserted, after]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.companion.new")
def test_companion_new_refusals(ctl):
    wid, (parent,) = _workspace(ctl)
    for placement in ({"workspace": wid}, {"workspaceName": "w"}, {"createWorkspace": True}, {"after": parent}):
        # the placement refusal comes before the --wait and target checks
        assert_error(ctl.request("session.companion.new", UNKNOWN, wait=True, **placement), PLACEMENT)
    assert_error(ctl.request("session.companion.new", parent, wait=True), "--wait requires --command")
    assert_error(ctl.request("session.companion.new", UNKNOWN), f"no such session: {UNKNOWN}")
    assert session_ids(ctl.tree(), wid) == [parent]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.collapse", "session.expand", "field:tree.session.collapsed")
def test_collapse_and_expand_fold_a_parent(ctl):
    wid, (parent, lone) = _workspace(ctl, sessions=2)
    child = _companion(ctl, parent)
    assert assert_ok(ctl.request("session.collapse", parent))["id"] == parent
    assert find_session(ctl.tree(), parent)["collapsed"] is True
    # an unchanged value still answers ok
    assert assert_ok(ctl.request("session.collapse", parent))["id"] == parent
    assert_absent(find_session(ctl.tree(), child), "collapsed")
    assert assert_ok(ctl.request("session.expand", parent))["id"] == parent
    assert_absent(find_session(ctl.tree(), parent), "collapsed")
    for cmd in ("session.collapse", "session.expand"):
        assert_error(ctl.request(cmd, lone), "session has no companions")
        assert_error(ctl.request(cmd, child), "session has no companions")
    # a fold outlives the last companion, but nothing folded is left to report
    assert_ok(ctl.request("session.collapse", parent))
    assert_ok(ctl.request("session.close", child))
    assert_absent(find_session(ctl.tree(), parent), "collapsed")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.move")
def test_a_companion_moves_only_among_its_siblings(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    first, second = _companion(ctl, parent), _companion(ctl, parent)
    other, () = _workspace(ctl, sessions=0)

    assert assert_ok(ctl.request("session.move", second, to="up"))["id"] == second
    assert session_ids(ctl.tree(), wid) == [parent, second, first, after]
    assert_ok(ctl.request("session.move", second, after=first))
    assert session_ids(ctl.tree(), wid) == [parent, first, second, after]
    # a top-level session steps over a whole group
    assert_ok(ctl.request("session.move", after, to="up"))
    assert session_ids(ctl.tree(), wid) == [after, parent, first, second]

    assert_error(ctl.request("session.move", first, workspace=other), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, workspace=other, targets=[first, second]), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, before=after), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, before=parent, targets=[first, second]), MOVE_REFUSED)
    # a batch dropped back where it is moves nothing, which is no refusal
    assert assert_ok(ctl.request("session.move", first, after=second, targets=[first, second]))["affected"] == 0
    assert session_ids(ctl.tree(), wid) == [after, parent, first, second]
    # an ordinary no-op stays ok
    assert assert_ok(ctl.request("session.move", workspace=wid, targets=[after, parent]))["affected"] == 0

    # a parent carries its group
    assert assert_ok(ctl.request("session.move", parent, workspace=other))["id"] == parent
    tree = ctl.tree()
    assert session_ids(tree, wid) == [after]
    assert session_ids(tree, other) == [parent, first, second]
    assert find_session(tree, first)["parent"] == parent
    for workspace in (wid, other):
        assert_ok(ctl.request("workspace.delete", workspace))


@pytest.mark.covers("session.companion.new", "event:session.created")
def test_a_companion_created_event_names_its_parent(ctl):
    wid, (parent,) = _workspace(ctl)
    cursor = ctl.events_baseline()
    child = _companion(ctl, parent, name="side")
    ctl.wait_event(cursor, "session.created", {"session": child, "payload": {"name": "side", "parent": parent}})
    top = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
    created = ctl.wait_event(cursor, "session.created", {"session": top})
    assert_absent(created["payload"], "parent")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.close", "field:result.closed", "event:session.closed")
def test_closing_a_parent_closes_its_group(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    first, second, third = _companion(ctl, parent), _companion(ctl, parent), _companion(ctl, parent)
    # closing a companion closes only it, with the bare receipt
    assert assert_ok(ctl.request("session.close", third)) == {"id": third, "closed": [third]}
    assert session_ids(ctl.tree(), wid) == [parent, first, second, after]

    cursor = ctl.events_baseline()
    receipt = assert_ok(ctl.request("session.close", parent))
    assert receipt == {"id": parent, "affected": 3, "closed": [parent, first, second]}
    assert session_ids(ctl.tree(), wid) == [after]
    # each member announces its own close, the companions last first, then the parent
    read = ctl.request("events.read", run=cursor.run, after=str(cursor.after), kinds=["session.closed"])
    group = {parent, first, second}
    closed = [event["session"] for event in assert_ok(read)["events"]["items"] if event.get("session") in group]
    assert closed == [second, first, parent]

    # a batch naming a companion beside its parent counts each session once
    first, second = _companion(ctl, after), _companion(ctl, after)
    other = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
    batch = assert_ok(ctl.request("session.close", targets=[after, first, other]))
    assert (batch["affected"], batch["closed"]) == (4, [after, first, second, other])
    assert session_ids(ctl.tree(), wid) == []
    assert_ok(ctl.request("workspace.delete", wid))
