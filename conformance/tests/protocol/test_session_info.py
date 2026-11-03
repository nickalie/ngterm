"""Ephemeral session info: source read-back, replacement, refusal and clearing."""

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session

pytestmark = pytest.mark.since("0.40.0")


@pytest.mark.covers("session.info", "field:tree.session.info", "field:tree.session.infoMarkdown", "field:tree.session.infoHideAfter")
def test_info_set_reject_and_clear(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    source = '**3** ^[ok](color: "green")'
    try:
        assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")
        assert_ok(ctl.request("session.info", sid, mode="set", text=source, markdown=True, hideAfter=600))
        assert_subset(find_session(ctl.tree(), sid), {"info": source, "infoMarkdown": True, "infoHideAfter": 600})
        assert_error(ctl.request("session.info", sid, mode="set", text='^[x](color: "pink")', markdown=True),
                     "session.info: unknown info color: pink (red|orange|yellow|green|blue|purple|gray)")
        assert find_session(ctl.tree(), sid)["info"] == source
        assert_ok(ctl.request("session.info", sid, mode="set", text="literal **stars**"))
        node = find_session(ctl.tree(), sid)
        assert node["info"] == "literal **stars**"
        assert not node.get("infoMarkdown", False) and node.get("infoHideAfter", 0) == 0
        assert_ok(ctl.request("session.info", sid, mode="clear"))
        assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("session.info")
@pytest.mark.parametrize(("args", "error"), [
    ({}, "invalid info mode:  (set|clear)"),
    ({"mode": "set"}, "session.info set requires text"),
    ({"mode": "clear", "markdown": False}, "session.info clear takes no text, markdown or hide-after"),
    ({"mode": "clear", "hideAfter": 0}, "session.info clear takes no text, markdown or hide-after"),
    ({"mode": "set", "text": "é" * 257}, "session.info: info text is over 512 bytes"),
    ({"mode": "set", "text": "x", "hideAfter": -1}, "session.info: --hide-after must be 0...86400 seconds"),
    ({"mode": "set", "text": "&#32;&#32;", "markdown": True}, "session.info: info text is empty"),
    ({"mode": "set", "text": "a&#10;b", "markdown": True}, "session.info: info text must be one line"),
    ({"mode": "set", "text": "x&#155;y", "markdown": True}, "session.info: info text contains a control character"),
    ({"mode": "set", "text": "^[a](color: red)", "markdown": True},
     'session.info: invalid info markdown; quote attribute values: ^[text](color: "green")'),
])
def test_info_validation(ctl, args, error):
    # Validation wins even over a target which cannot resolve.
    assert_error(ctl.request("session.info", "zzz", **args), error)


def _keeps_info(ctl, sid, text, duration):
    import time
    from agterm_conformance.wait import wait_until
    end = time.monotonic() + duration

    def kept():
        assert find_session(ctl.tree(), sid)["info"] == text
        return time.monotonic() >= end

    wait_until(kept)


@pytest.mark.covers("session.info", "field:tree.session.infoHideAfter")
def test_info_expiry_refresh_and_cancel(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    try:
        assert_ok(ctl.request("session.info", sid, mode="set", text="brief", hideAfter=0.1))
        ctl.wait_tree(lambda tree: "info" not in find_session(tree, sid))
        assert_ok(ctl.request("session.info", sid, mode="set", text="refresh", hideAfter=0.5))
        assert_ok(ctl.request("session.info", sid, mode="set", text="refresh", hideAfter=1))
        _keeps_info(ctl, sid, "refresh", 0.6)
        ctl.wait_tree(lambda tree: "info" not in find_session(tree, sid))
        assert_ok(ctl.request("session.info", sid, mode="set", text="old", hideAfter=0.1))
        assert_ok(ctl.request("session.info", sid, mode="set", text="stays"))
        _keeps_info(ctl, sid, "stays", 0.3)
        assert_ok(ctl.request("session.info", sid, mode="clear"))
        assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("session.info", "session.duplicate", "behavior:restore.relaunch-keeps-layout")
def test_info_not_persisted_or_duplicated(launch):
    ctl = launch()
    sid = assert_ok(ctl.request("session.new", name="ephemeral info"))["id"]
    assert_ok(ctl.request("session.info", sid, mode="set", text="runtime-only"))
    copy = assert_ok(ctl.request("session.duplicate", sid))["id"]
    assert_absent(find_session(ctl.tree(), copy), "info", "infoMarkdown", "infoHideAfter")
    # This structural write persists the session independently of runtime info.
    assert_ok(ctl.request("workspace.new", name="flush layout"))
    ctl = launch.relaunch(ctl)
    assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")


@pytest.mark.covers("session.info", "session.close", "recent.open")
def test_info_soft_close_undo(ctl):
    for timed in (False, True):
        sid = assert_ok(ctl.request("session.new"))["id"]
        assert_ok(ctl.request("session.info", sid, mode="set", text="line", hideAfter=30 if timed else 0))
        assert assert_ok(ctl.request("session.close", sid, undoable=True))["undoable"]
        (entry,) = [item for item in assert_ok(ctl.request("recent.list"))["recent"] if item["sessionIDs"] == [sid]]
        assert_ok(ctl.request("recent.open", entry["id"]))
        node = find_session(ctl.tree(), sid)
        if timed:
            assert_absent(node, "info", "infoMarkdown", "infoHideAfter")
        else:
            assert node["info"] == "line"
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("session.info", "event:tree.changed")
def test_info_tree_changed(ctl):
    cursor = ctl.events_baseline()
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_event(cursor, "tree.changed")
    cursor = ctl.events_baseline()
    try:
        assert_ok(ctl.request("session.info", sid, mode="set", text="changes"))
        ctl.wait_event(cursor, "tree.changed")
        cursor = ctl.events_baseline()
        assert_ok(ctl.request("session.info", sid, mode="set", text="changes"))
        _keeps_info(ctl, sid, "changes", 0.5)
        items = assert_ok(ctl.request("events.read", run=cursor.run, after=str(cursor.after), kinds=["tree.changed"]))["events"]["items"]
        assert items == []
        assert_ok(ctl.request("session.info", sid, mode="clear"))
        ctl.wait_event(cursor, "tree.changed")
    finally:
        assert_ok(ctl.request("session.close", sid))
