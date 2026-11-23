"""A split session attached from another machine opens on its left pane, while the origin's own split focuses its right
pane: attached alone, or as a group member attached in the background and selected later (upstream `faeaf74d`)."""

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.since("0.42.0"),
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", reason="the origin session must be Live"),
]


def _split(origin, sid):
    origin.wait_mounted(sid, "left")
    assert_ok(origin.request("session.split", sid, mode="on", axis="horizontal"))
    origin.wait_mounted(sid, "right")
    assert find_session(origin.tree(), sid)["splitFocused"] is True, "a new split focuses its right pane"


def _listed(origin, *sids):
    origin.wait_response(
        origin.vocabulary.command("live.tree"), None,
        lambda response: set(sids) <= {row["id"] for row in assert_ok(response)["remote"]["sessions"]},
    )


def _opens_on_the_left(viewer, local):
    viewer.wait_mounted(local, "right")
    node = find_session(viewer.tree(), local)
    active = {surface["kind"]: surface.get("active") for surface in node["surfaces"]}
    assert (node["splitFocused"], active["left"], active["right"]) == (False, True, False), node


@pytest.mark.covers("live.attach", "field:tree.session.splitFocused")
def test_an_attached_split_session_opens_on_its_left_pane(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="split-attach"))["id"]
    local = None
    try:
        _split(origin, sid)
        _listed(origin, sid)
        local = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach"), sid, host=remote_origin.host)
        )["id"]
        _opens_on_the_left(remote_viewer, local)
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("live.attach.group", "session.select", "field:tree.session.splitFocused")
def test_a_split_companion_attached_in_the_background_opens_on_its_left_pane_when_selected(
    remote_origin, remote_viewer
):
    origin = Ctl(remote_origin.instance)
    parent = assert_ok(origin.request("session.new", name="group-parent"))["id"]
    local = None
    try:
        origin.wait_mounted(parent, "left")
        companion = assert_ok(origin.request("session.companion.new", parent, name="split-member", noSelect=True))["id"]
        _split(origin, companion)
        _listed(origin, parent, companion)
        attached = assert_ok(
            remote_viewer.request(
                remote_viewer.vocabulary.command("live.attach.group"), parent, host=remote_origin.host
            )
        )
        local = attached["id"]
        (member,) = attached["remoteGroup"]["companions"]
        assert member["origin"] == companion, attached
        assert find_session(remote_viewer.tree(), member["id"])["active"] is False, "attached in the background"
        _opens_on_the_left(remote_viewer, member["id"])
        assert_ok(remote_viewer.request("session.select", member["id"]))
        remote_viewer.wait_tree(lambda tree: find_session(tree, member["id"])["active"])
        _opens_on_the_left(remote_viewer, member["id"])
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", parent))
