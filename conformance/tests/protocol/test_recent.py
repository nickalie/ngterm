"""Recent Closed commands (File > Open Recent): `recent.list` as the read-back of `recent.open` and `recent.clear`,
with the tree showing what a reopen brought back. The list is app-global and the module shares one instance, so every
test empties it first and builds the closed entries it reads; the workspaces it made are deleted afterwards."""

import itertools

import pytest
from agterm_conformance.match import (
    assert_error,
    assert_ok,
    assert_subset,
    find_session,
    session_ids,
    window_ids,
    workspace_ids,
)

UNKNOWN = "zzz"
_names = itertools.count(1)


def _name(prefix):
    return f"{prefix}-{next(_names)}"


@pytest.fixture
def workspaces_made(ctl):
    """Empties the Recent Closed list, collects the workspaces a test makes, and deletes them (then empties the list
    again) when it ends."""
    assert_ok(ctl.request("recent.clear"))
    made = []
    yield made
    for wid in made:
        for window in window_ids(ctl.request("window.list")):
            tree = ctl.request("tree", window=window)
            if tree.ok and wid in workspace_ids(tree):
                assert_ok(ctl.request("workspace.delete", wid, window=window))
    assert_ok(ctl.request("recent.clear"))


def _workspace(ctl, made, window=None):
    """A new workspace (in `window`, else the frontmost); its id and name."""
    name = _name("recent")
    wid = assert_ok(ctl.request("workspace.new", name=name, **({} if window is None else {"window": window})))["id"]
    made.append(wid)
    return wid, name


def _session(ctl, wid, window=None):
    """A named session in workspace `wid`; its id and name."""
    name = _name("closed")
    args = {"workspace": wid, "name": name, **({} if window is None else {"window": window})}
    return assert_ok(ctl.request("session.new", **args))["id"], name


def _closed(ctl, wid, window=None):
    """A named session in workspace `wid`, closed for good; its id and name."""
    sid, name = _session(ctl, wid, window)
    assert assert_ok(ctl.request("session.close", sid))["id"] == sid
    return sid, name


def _listed(ctl):
    return assert_ok(ctl.request("recent.list"))["recent"]


def _opened(response):
    return assert_ok(response)["recentOpen"]


def _in_tree(ctl, sid, window=None):
    return sid in session_ids(assert_ok(ctl.request("tree", **({} if window is None else {"window": window}))))


def _active_window(ctl):
    (active,) = [node["id"] for node in assert_ok(ctl.request("window.list"))["windows"] if node.get("active")]
    return active


@pytest.mark.covers("recent.list", "recent.clear", "field:result.recent")
def test_list_reports_closed_sessions_newest_first_and_clear_empties_it(ctl, workspaces_made):
    wid, workspace_name = _workspace(ctl, workspaces_made)
    older, older_name = _closed(ctl, wid)
    newer, newer_name = _closed(ctl, wid)

    entries = _listed(ctl)
    assert [entry["sessionIDs"] for entry in entries] == [[newer], [older]]
    for entry, title in zip(entries, (newer_name, older_name), strict=True):
        assert_subset(entry, {"kind": "session", "title": title, "subtitle": workspace_name, "workspaceID": wid})
        assert isinstance(entry["id"], str) and isinstance(entry["closedAt"], (int, float))
    assert entries[0]["closedAt"] >= entries[1]["closedAt"]
    assert entries[0]["id"] != entries[1]["id"]

    assert assert_ok(ctl.request("recent.clear")) == {}
    assert _listed(ctl) == []
    assert assert_ok(ctl.request("recent.clear")) == {}


@pytest.mark.covers("recent.open", "field:result.recentOpen")
def test_open_without_an_id_reopens_the_newest_and_consumes_it(ctl, workspaces_made):
    window = _active_window(ctl)
    wid, _ = _workspace(ctl, workspaces_made)
    older, _ = _closed(ctl, wid)
    newer, newer_name = _closed(ctl, wid)
    newest = _listed(ctl)[0]

    opened = _opened(ctl.request("recent.open"))

    assert opened == {"item": newest, "window": window, "complete": True}
    assert opened["item"]["title"] == newer_name
    assert _in_tree(ctl, newer)
    assert not _in_tree(ctl, older)
    assert [entry["sessionIDs"] for entry in _listed(ctl)] == [[older]]


@pytest.mark.covers("recent.open")
def test_open_by_prefix_reopens_that_entry(ctl, workspaces_made):
    wid, _ = _workspace(ctl, workspaces_made)
    older, _ = _closed(ctl, wid)
    newer, _ = _closed(ctl, wid)
    entry = _listed(ctl)[-1]
    assert entry["sessionIDs"] == [older]

    opened = _opened(ctl.request("recent.open", entry["id"][:12]))

    assert opened["item"]["id"] == entry["id"]
    assert _in_tree(ctl, older)
    assert not _in_tree(ctl, newer)


@pytest.mark.covers("recent.open", "recent.list")
def test_reopened_session_keeps_its_id_split_flag_context_and_pin(ctl, workspaces_made):
    wid, _ = _workspace(ctl, workspaces_made)
    sid, _ = _session(ctl, wid)
    assert_ok(ctl.request("session.split", sid, mode="on"))
    assert_ok(ctl.request("session.flag", sid, mode="on"))
    assert_ok(ctl.request("session.context", sid, mode="set", text="probe context"))
    assert_ok(ctl.request("session.restore", sid, mode="set", command="echo pinned"))
    assert_ok(ctl.request("session.close", sid))
    assert not _in_tree(ctl, sid)

    (entry,) = [entry for entry in _listed(ctl) if entry["sessionIDs"] == [sid]]
    assert entry["kind"] == "session"
    assert _opened(ctl.request("recent.open", entry["id"]))["complete"] is True

    node = find_session(ctl.tree(), sid)
    assert_subset(
        node, {"hasSplit": True, "flagged": True, "context": "probe context", "restoreCommand": "echo pinned"}
    )
    assert entry["id"] not in [listed["id"] for listed in _listed(ctl)]


@pytest.fixture
def second(ctl):
    wid = assert_ok(ctl.request("window.new", name=_name("recent-window")))["id"]
    yield wid
    if wid in window_ids(ctl.request("window.list")):
        assert_ok(ctl.request("window.delete", wid))


@pytest.mark.covers("recent.open")
def test_open_routes_to_the_window_holding_the_workspace_without_moving_frontmost(ctl, workspaces_made, second):
    (source,) = [window for window in window_ids(ctl.request("window.list")) if window != second]
    wid, _ = _workspace(ctl, workspaces_made, window=source)
    sid, _ = _session(ctl, wid, window=source)
    assert_ok(ctl.request("window.select", second))

    def frontmost(response):
        (node,) = [node for node in assert_ok(response)["windows"] if node["id"] == second]
        return node.get("active") is True

    ctl.wait_response("window.list", None, frontmost)
    assert_ok(ctl.request("session.close", sid))

    opened = _opened(ctl.request("recent.open", window=second))

    assert opened["window"] == source
    assert _in_tree(ctl, sid, window=source)
    assert not _in_tree(ctl, sid, window=second)
    assert _active_window(ctl) == second


@pytest.mark.covers("recent.open")
def test_open_during_the_grace_window_restores_the_original_group(ctl, workspaces_made):
    wid, _ = _workspace(ctl, workspaces_made)
    first, _ = _session(ctl, wid)
    second, _ = _session(ctl, wid)
    assert assert_ok(ctl.request("session.close", targets=[first, second]))["affected"] == 2
    assert not _in_tree(ctl, first) and not _in_tree(ctl, second)

    opened = _opened(ctl.request("recent.open"))

    # each session of the batch has its entry; reopening one undoes the whole pending close and consumes both
    assert opened["complete"] is True
    assert opened["item"]["sessionIDs"] in ([first], [second])
    assert session_ids(ctl.tree(), wid) == [first, second]
    assert _listed(ctl) == []


@pytest.mark.covers("recent.list", "recent.clear", "recent.open")
def test_recent_refusals(ctl, workspaces_made):
    for cmd in ("recent.list", "recent.clear"):
        refusal = f"{cmd} takes no target or --window"
        assert_error(ctl.request(cmd, "active"), refusal)
        assert_error(ctl.request(cmd, window="active"), refusal)
    assert_error(ctl.request("recent.open"), "no recently closed items")
    assert_error(ctl.request("recent.open", UNKNOWN), "no recently closed items")

    wid, _ = _workspace(ctl, workspaces_made)
    sid, _ = _closed(ctl, wid)
    before = _listed(ctl)
    assert_error(ctl.request("recent.open", UNKNOWN), f"no such recent item: {UNKNOWN}")
    assert_error(ctl.request("recent.open", "active"), "no such recent item: active")
    assert_error(ctl.request("recent.open", window=UNKNOWN), f"no such window: {UNKNOWN}")
    assert_error(ctl.request("recent.clear", window=UNKNOWN), "recent.clear takes no target or --window")
    assert _listed(ctl) == before
    assert not _in_tree(ctl, sid)
