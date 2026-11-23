"""Session-bound picks (`pick.open SESSION`) on an origin whose session a viewer presents (`live attach HOST ID`), against
an origin that is a second instance of the build behind the harness's own sshd (`remote_origin`). The origin hands the
pick to the viewer, which shows a replica in the window holding the attached row; the replica's outcome goes back to the
origin, and the origin's end of the pick takes the replica down. A viewer that goes away, or whose pick slot is busy,
hands the pick back into the origin's window.

No protocol command answers a pick, so the outcomes reached here are `pending` and `cancelled`: a cancel on the viewer
stands for its Esc."""

import pytest
from agterm_conformance.match import assert_absent, assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_response

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]

ITEMS = [{"id": "a", "label": "Alpha"}, {"id": "b", "label": "Beta"}]


def _presented(origin, viewer, host):
    """A new origin session, attached on the viewer and followed there: the origin and viewer session ids."""
    before = set(session_ids(origin.tree()))
    sid = assert_ok(origin.request("session.new"))["id"]
    origin.wait_mounted(sid, "left")
    # a desktop origin's window leads the session it shows: it shows another one
    assert_ok(origin.request("session.select", sorted(before)[0]))
    local = assert_ok(viewer.request("live.attach", sid, host=host))["id"]
    viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))

    def follows(tree):
        node = find_session(tree, sid)
        assert node.get("presenters", {}).get("presenter") is True, node
        (left,) = [surface for surface in node["surfaces"] if surface["kind"] == "left"]
        assert left.get("lead") == "follower", left
        return True

    origin.wait_tree(follows)
    return sid, local


def _replica(viewer, local, pick_id):
    viewer.wait_tree(lambda tree: find_session(tree, local).get("pick") == {"id": pick_id, "replica": True})
    assert viewer.tree()["tree"]["pickPending"] == pick_id


def _no_pick(ctl, sid):
    ctl.wait_tree(lambda tree: "pick" not in find_session(tree, sid))


def _ends_cancelled(origin, pick_id):
    reply = wait_response(origin, "pick.result", pick_id, lambda response: response.result["pick"]["result"] != "pending")
    assert reply.result["pick"] == {"result": "cancelled"}


def _open(origin, sid, **args):
    return assert_ok(origin.request("pick.open", sid, items=ITEMS, prompt="Branch", **args))["id"]


@pytest.mark.since("0.42.0")
@pytest.mark.covers("pick.open", "pick.result", "pick.cancel", "field:tree.session.pick")
def test_a_session_pick_shows_on_the_viewer_and_ends_from_either_side(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid, local = _presented(origin, remote_viewer, remote_origin.host)

    pick_id = _open(origin, sid)
    assert find_session(origin.tree(), sid)["pick"] == {"id": pick_id, "remote": True}
    assert_absent(origin.tree()["tree"], "pickPending")
    _replica(remote_viewer, local, pick_id)
    assert assert_ok(origin.request("pick.result", pick_id))["pick"] == {"result": "pending"}
    assert_ok(remote_viewer.request("pick.cancel", pick_id))
    _ends_cancelled(origin, pick_id)
    _no_pick(origin, sid)
    _no_pick(remote_viewer, local)

    pick_id = _open(origin, sid)
    _replica(remote_viewer, local, pick_id)
    assert_ok(origin.request("pick.cancel", pick_id))
    assert assert_ok(origin.request("pick.result", pick_id))["pick"] == {"result": "cancelled"}
    _no_pick(remote_viewer, local)
    assert_absent(remote_viewer.tree()["tree"], "pickPending")

    # the replica must show on the viewer before the origin's deadline takes it down
    pick_id = _open(origin, sid, timeout=5)
    _replica(remote_viewer, local, pick_id)
    _ends_cancelled(origin, pick_id)
    _no_pick(remote_viewer, local)

    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))


@pytest.mark.since("0.42.0")
@pytest.mark.covers("pick.open", "pick.cancel", "field:tree.session.pick")
def test_a_session_pick_comes_back_to_the_origin_when_the_viewer_goes_away(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid, local = _presented(origin, remote_viewer, remote_origin.host)
    pick_id = _open(origin, sid)
    _replica(remote_viewer, local, pick_id)

    assert_ok(remote_viewer.request("session.close", local))
    origin.wait_tree(lambda tree: find_session(tree, sid).get("pick") == {"id": pick_id})
    assert origin.tree()["tree"]["pickPending"] == pick_id
    assert assert_ok(origin.request("pick.result", pick_id))["pick"] == {"result": "pending"}
    assert_ok(origin.request("pick.cancel", pick_id))
    assert_absent(origin.tree()["tree"], "pickPending")
    assert_ok(origin.request("session.close", sid))


@pytest.mark.since("0.42.0")
@pytest.mark.covers("pick.open", "pick.cancel", "field:tree.session.pick")
def test_a_viewer_with_a_busy_pick_slot_hands_the_pick_back(remote_origin, remote_viewer):
    """The origin's window takes the pick when its slot is free; when that slot is held too, the pick ends cancelled."""
    origin = Ctl(remote_origin.instance)
    sid, local = _presented(origin, remote_viewer, remote_origin.host)
    busy = assert_ok(remote_viewer.request("pick.open", items=ITEMS))["id"]
    try:
        pick_id = _open(origin, sid)
        origin.wait_tree(lambda tree: find_session(tree, sid).get("pick") == {"id": pick_id})
        assert origin.tree()["tree"]["pickPending"] == pick_id
        assert_absent(find_session(remote_viewer.tree(), local), "pick")
        assert remote_viewer.tree()["tree"]["pickPending"] == busy
        assert_ok(origin.request("pick.cancel", pick_id))

        held = assert_ok(origin.request("pick.open", items=ITEMS))["id"]
        try:
            pick_id = _open(origin, sid)
            _ends_cancelled(origin, pick_id)
            _no_pick(origin, sid)
            assert origin.tree()["tree"]["pickPending"] == held
        finally:
            origin.request("pick.cancel", held)
    finally:
        remote_viewer.request("pick.cancel", busy)
    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))
