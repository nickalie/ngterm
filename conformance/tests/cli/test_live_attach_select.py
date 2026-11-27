"""`ngtermctl live attach` and the selection of the viewer's window. A plain attach selects the new row, as upstream's
does. `--no-select`, also with `--companions`, adds the rows and leaves the selection where it was, so a viewer hook that
attaches an origin's new session does not show it for a moment. No event shows the keyboard focus of a new row: the
core's unit tests check that `--no-select` does not move it."""

from typing import Any

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids, sessions
from agterm_conformance.plugin import Cli, Ctl
from agterm_conformance.wait import Cursor, WaitTimeout

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", "live-any-shell", any=True, reason="the origin's session must be Live"),
]

# How long a selection that the attach queued has to show as an event.
SETTLE = 3.0


def _cli(adapter: Any, viewer: Ctl) -> Cli:
    return Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)


def _attach(cli: Cli, host: str, sid: str, *flags: str) -> None:
    result = cli(*cli.vocabulary.cli("live", "attach", host, sid, *flags))
    assert result.returncode == 0, result


def _row(viewer: Ctl, origin_sid: str) -> dict[str, Any]:
    def found(tree: dict[str, Any]) -> Any:
        return next((node for node in sessions(tree) if node.get("remoteSessionID") == origin_sid), None)

    return found(viewer.wait_tree(found))


def _listed(origin: Ctl, *sids: str) -> None:
    origin.wait_response(
        origin.vocabulary.command("live.tree"), None,
        lambda response: set(sids) <= {row["id"] for row in assert_ok(response)["remote"]["sessions"]},
    )


def _events(viewer: Ctl, cursor: Cursor, kind: str) -> list[dict[str, Any]]:
    batch = assert_ok(
        viewer.request("events.read", run=cursor.run, after=str(cursor.after), kinds=[kind], limit=500)
    )["events"]
    return list(batch["items"])


def _kept(viewer: Ctl, cursor: Cursor, kept: str, rows: list[str]) -> None:
    """No selection appears for the rows; `kept` stays the selected session."""
    with pytest.raises(WaitTimeout):
        viewer.wait_event(cursor, "session.selected", timeout=SETTLE)
    assert _events(viewer, cursor, "session.selected") == []
    tree = viewer.tree()
    assert find_session(tree, kept)["active"] is True
    assert [find_session(tree, row)["active"] for row in rows] == [False] * len(rows)


def _close(viewer: Ctl, origin: Ctl, rows: list[str], origin_sessions: list[str]) -> None:
    for row in reversed(rows):
        viewer.request("session.close", row)
    for sid in origin_sessions:
        origin.request("session.close", sid)


@pytest.mark.covers("cli:live attach", "event:session.selected", "field:tree.session.active")
def test_a_plain_attach_selects_the_new_row(adapter, remote_origin, remote_viewer):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    sid = assert_ok(origin.request("session.new", name="attach-selects"))["id"]
    rows: list[str] = []
    try:
        origin.wait_mounted(sid, "left")
        _listed(origin, sid)
        cursor = viewer.events_baseline()
        _attach(_cli(adapter, viewer), remote_origin.host, sid)
        row = _row(viewer, sid)["id"]
        rows.append(row)
        viewer.wait_event(cursor, "session.selected", {"session": row})
        assert find_session(viewer.tree(), row)["active"] is True
    finally:
        _close(viewer, origin, rows, [sid])


@pytest.mark.requires("live-attach-no-select", reason="upstream's live attach always selects the new row")
@pytest.mark.covers("cli:live attach", "event:session.selected", "field:tree.session.active")
def test_attach_with_no_select_keeps_the_selection(adapter, remote_origin, remote_viewer):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    kept = session_ids(viewer.tree())[0]
    sid = assert_ok(origin.request("session.new", name="attach-no-select"))["id"]
    rows: list[str] = []
    try:
        origin.wait_mounted(sid, "left")
        _listed(origin, sid)
        assert_ok(viewer.request("session.select", kept))
        viewer.wait_tree(lambda tree: find_session(tree, kept)["active"])
        cursor = viewer.events_baseline()
        _attach(_cli(adapter, viewer), remote_origin.host, sid, "--no-select")
        row = _row(viewer, sid)["id"]
        rows.append(row)
        viewer.wait_mounted(row, "left")
        _kept(viewer, cursor, kept, rows)
    finally:
        _close(viewer, origin, rows, [sid])


@pytest.mark.requires("live-attach-no-select", reason="upstream's live attach always selects the new row")
@pytest.mark.covers("cli:live attach", "event:session.selected", "field:tree.session.active")
def test_attach_with_companions_and_no_select_keeps_the_selection(adapter, remote_origin, remote_viewer):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    kept = session_ids(viewer.tree())[0]
    parent = assert_ok(origin.request("session.new", name="group-no-select"))["id"]
    rows: list[str] = []
    try:
        origin.wait_mounted(parent, "left")
        companion = assert_ok(origin.request("session.companion.new", parent, name="group-member", noSelect=True))[
            "id"
        ]
        origin.wait_mounted(companion, "left")
        _listed(origin, parent, companion)
        assert_ok(viewer.request("session.select", kept))
        viewer.wait_tree(lambda tree: find_session(tree, kept)["active"])
        cursor = viewer.events_baseline()
        _attach(_cli(adapter, viewer), remote_origin.host, parent, "--companions", "--no-select")
        root = _row(viewer, parent)["id"]
        member = _row(viewer, companion)
        rows.extend([root, member["id"]])
        assert member["parent"] == root
        viewer.wait_mounted(root, "left")
        viewer.wait_mounted(member["id"], "left")
        _kept(viewer, cursor, kept, rows)
    finally:
        _close(viewer, origin, rows, [parent])


@pytest.mark.requires("live-attach-one-row", reason="upstream's live attach adds a second row for a shown session")
@pytest.mark.covers("cli:live attach", "event:session.selected", "field:tree.session.active")
def test_a_second_attach_of_a_shown_session_answers_with_its_row(adapter, remote_origin, remote_viewer):
    """A second row would run a second presentation stream and post each of the origin's notifications twice."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    kept = session_ids(viewer.tree())[0]
    sid = assert_ok(origin.request("session.new", name="attach-one-row"))["id"]
    cli = _cli(adapter, viewer)
    rows: list[str] = []
    try:
        origin.wait_mounted(sid, "left")
        _listed(origin, sid)
        assert_ok(viewer.request("session.select", kept))
        viewer.wait_tree(lambda tree: find_session(tree, kept)["active"])
        first = cli(*cli.vocabulary.cli("live", "attach", remote_origin.host, sid, "--no-select"))
        assert first.returncode == 0, first
        row = _row(viewer, sid)["id"]
        rows.append(row)
        viewer.wait_mounted(row, "left")
        cursor = viewer.events_baseline()

        quiet = cli(*cli.vocabulary.cli("live", "attach", remote_origin.host, sid, "--no-select"))
        assert quiet.returncode == 0, quiet
        assert quiet.stdout.strip() == row, quiet
        _kept(viewer, cursor, kept, rows)
        plain = cli(*cli.vocabulary.cli("live", "attach", remote_origin.host, sid))
        assert plain.returncode == 0, plain
        assert plain.stdout.strip() == row, plain

        viewer.wait_event(cursor, "session.selected", {"session": row})
        tree = viewer.tree()
        assert [node["id"] for node in sessions(tree) if node.get("remoteSessionID") == sid] == [row]
        assert find_session(tree, row)["active"] is True
    finally:
        _close(viewer, origin, rows, [sid])
