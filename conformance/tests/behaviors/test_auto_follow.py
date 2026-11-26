"""Auto-follow attention on a viewer: after its idle timeout the window selects a blocked session that runs something,
and a blocked session or attached row that ends while it is not selected never takes the selection, not even for a
moment. The jump is upstream's (`AppStore+AutoFollow.swift`). The skip of an ended row is the Rust build's: it closes
a terminal companion whose session ended on its origin, so a jump there would come back at once."""

from collections.abc import Iterator
from typing import Any

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import Cursor, WaitTimeout, wait_until

# The longest grace Settings offers below a minute: a row that ends right after its status arrives holds its exit well
# before the fire, so a jump the test sees is one to an ended row.
FOLLOW = {"autoFollowAttention": "s10"}
FOLLOW_SECONDS = 10.0
# The fire lands a full timeout after the last change of the attention set; this covers it with a margin.
PAST_THE_FIRE = FOLLOW_SECONDS + 5.0
LIVE_ORIGIN = pytest.mark.requires(
    "live-sessions", "live-any-shell", any=True, reason="the origin's session must be Live"
)
EXITS = pytest.mark.requires("posix-shell", reason="the test types `exit` into a pane's shell")


@pytest.fixture(scope="module")
def follow_viewer(adapter: Any, remote_origin: Any) -> Iterator[Ctl]:
    """A viewer of `remote_origin` with auto-follow on."""
    instance = adapter.launch(path_prefix=remote_origin.viewer_path, settings=FOLLOW)
    try:
        yield Ctl(instance)
    finally:
        adapter.stop(instance)


def _first_session(ctl: Ctl) -> str:
    return session_ids(ctl.tree())[0]


def _select(ctl: Ctl, sid: str) -> None:
    assert_ok(ctl.request("session.select", sid))
    ctl.wait_tree(lambda tree: find_session(tree, sid)["active"])


def _selections(ctl: Ctl, cursor: Cursor) -> list[dict[str, Any]]:
    batch = assert_ok(
        ctl.request("events.read", run=cursor.run, after=str(cursor.after), kinds=["session.selected"], limit=500)
    )["events"]
    return list(batch["items"])


def _no_selection_past_the_fire(ctl: Ctl, cursor: Cursor, kept: str) -> None:
    """Waits a full auto-follow timeout and more: no `session.selected` event appears, and `kept` stays selected."""
    with pytest.raises(WaitTimeout):
        ctl.wait_event(cursor, "session.selected", timeout=PAST_THE_FIRE)
    assert _selections(ctl, cursor) == []
    assert find_session(ctl.tree(), kept)["active"] is True


def _listed(origin: Ctl, *sids: str) -> None:
    origin.wait_response(
        origin.vocabulary.command("live.tree"), None,
        lambda response: set(sids) <= {row["id"] for row in assert_ok(response)["remote"]["sessions"]},
    )


def _attach(viewer: Ctl, host: str, sid: str) -> str:
    local = assert_ok(viewer.request(viewer.vocabulary.command("live.attach"), sid, host=host))["id"]
    viewer.wait_mounted(local, "left")
    return local


@pytest.mark.covers("session.status", "events.read", "event:session.selected", "field:tree.session.active")
def test_auto_follow_selects_a_blocked_session_that_runs(launch):
    ctl = launch(settings=FOLLOW)
    kept = _first_session(ctl)
    blocked = assert_ok(ctl.request("session.new", name="blocked"))["id"]
    ctl.wait_mounted(blocked, "left")
    _select(ctl, kept)
    cursor = ctl.events_baseline()
    assert_ok(ctl.request("session.status", blocked, status="blocked"))
    followed = ctl.wait_event(cursor, "session.selected", {"session": blocked}, timeout=PAST_THE_FIRE + 10)
    assert followed["payload"]["previous"] == kept
    assert find_session(ctl.tree(), blocked)["active"] is True


@EXITS
@pytest.mark.covers("session.status", "events.read", "event:session.selected", "field:tree.session.active")
def test_a_blocked_session_whose_shell_exits_never_takes_the_selection(launch):
    """Upstream's behavior too: the session closes, so no fire can select it. The baseline of the remote cases."""
    ctl = launch(settings=FOLLOW)
    kept = _first_session(ctl)
    blocked = assert_ok(ctl.request("session.new", name="blocked-exits"))["id"]
    ctl.wait_mounted(blocked, "left")
    _select(ctl, kept)
    cursor = ctl.events_baseline()
    assert_ok(ctl.request("session.status", blocked, status="blocked"))
    ctl.wait_tree(lambda tree: find_session(tree, blocked).get("status") == "blocked")
    assert_ok(ctl.request("session.type", blocked, text="exit\n"))
    ctl.wait_tree(lambda tree: blocked not in session_ids(tree))
    _no_selection_past_the_fire(ctl, cursor, kept)


@LIVE_ORIGIN
@EXITS
@pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell")
@pytest.mark.requires("auto-follow-skips-ended", reason="upstream follows a blocked row that holds its exit")
@pytest.mark.parametrize("companion", [False, True], ids=["top-level-row", "companion-row"])
@pytest.mark.covers("live.attach", "session.status", "events.read", "event:session.selected")
def test_a_blocked_remote_row_that_ends_never_takes_the_selection(remote_origin, follow_viewer, companion):
    """The row holds its exit past the fire. A companion row with nothing local closes before the fire, so the companion
    case keeps a local scratch on the row: the row stays, and the scratch did not set the status."""
    origin, viewer = Ctl(remote_origin.instance), follow_viewer
    kept = _first_session(viewer)
    parent = assert_ok(origin.request("session.new", name="follow-parent"))["id"]
    ended = False
    rows: list[str] = []
    try:
        origin.wait_mounted(parent, "left")
        target = parent
        if companion:
            target = assert_ok(origin.request("session.companion.new", parent, name="follow-worker", noSelect=True))[
                "id"
            ]
            origin.wait_mounted(target, "left")
        _listed(origin, parent, target)
        rows.append(_attach(viewer, remote_origin.host, parent))
        if companion:
            rows.append(_attach(viewer, remote_origin.host, target))
            assert find_session(viewer.tree(), rows[-1])["parent"] == rows[0]
        row = rows[-1]
        if companion:
            assert_ok(viewer.request("session.scratch", row, mode="on"))
            viewer.wait_mounted(row, "scratch")
        _select(viewer, kept)
        cursor = viewer.events_baseline()

        # set on the row: a status from the origin goes when its session ends, and a row without one is no target
        assert_ok(viewer.request("session.status", row, status="blocked"))
        viewer.wait_tree(lambda tree: find_session(tree, row).get("status") == "blocked")
        assert_ok(origin.request("session.type", target, text="exit\n"))
        ended = not companion
        origin.wait_tree(lambda tree: target not in session_ids(tree))
        viewer.wait_text(row, r"Process exited", pane="left")

        _no_selection_past_the_fire(viewer, cursor, kept)
        assert row in session_ids(viewer.tree())
    finally:
        for local in reversed(rows):
            viewer.request("session.close", local)
        if not ended:
            origin.request("session.close", parent)
        wait_until(lambda: parent not in session_ids(origin.tree()))
