"""A split that a viewer closes, hides or swaps is a pane of the origin, so the change reaches the origin and the next
attach shows it. Upstream sends a split change to the server only for a row of a remote (catalog) window, from its GUI
actions and from `session.split`/`session.split.close` alike (`performRemote`, `RemoteOperation.structuralCommands`).
An attached row (`live attach`) keeps the change on the viewer, so these tests are Rust-only."""

import re

import pytest
from agterm_conformance.match import assert_ok, find_session, sessions
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]


def _listed(origin, sid):
    origin.wait_response(
        origin.vocabulary.command("live.tree"), None,
        lambda response: sid in {row["id"] for row in assert_ok(response)["remote"]["sessions"]},
    )


def _attach(viewer, remote_origin, sid):
    _listed(Ctl(remote_origin.instance), sid)
    local = assert_ok(viewer.request(viewer.vocabulary.command("live.attach"), sid, host=remote_origin.host))["id"]
    viewer.wait_mounted(local, "left")
    return local


def _detach(viewer, local):
    assert_ok(viewer.request("session.close", local))
    viewer.wait_tree(lambda tree: local not in {node["id"] for node in sessions(tree)})


def _split_on_the_viewer(viewer, local, origin, sid):
    assert_ok(viewer.request("session.split", local, mode="on"))
    origin.wait_tree(lambda tree: find_session(tree, sid).get("split") is True, timeout=30)
    origin.wait_mounted(sid, "right")
    viewer.wait_mounted(local, "right")


def _mark(origin, sid, pane, marker):
    # printed in two halves, so the echoed command line never matches
    half = len(marker) // 2
    assert_ok(origin.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:half]} {marker[half:]}\r", pane=pane))
    origin.wait_text(sid, rf"(?m)^{re.escape(marker)}\s*$", pane=pane)


@pytest.mark.parametrize("remote_origin", ["instance", "headless"], indirect=True)
@pytest.mark.covers("live.attach", "session.split", "session.split.close", "field:tree.session.hasSplit")
def test_a_split_closed_on_the_viewer_is_gone_on_the_origin_and_on_the_next_attach(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="split-close"))["id"]
    local = None
    try:
        origin.wait_mounted(sid, "left")
        local = _attach(remote_viewer, remote_origin, sid)
        _split_on_the_viewer(remote_viewer, local, origin, sid)
        assert_ok(remote_viewer.request("session.split.close", local))
        origin.wait_tree(lambda tree: not find_session(tree, sid).get("hasSplit"), timeout=30)
        _detach(remote_viewer, local)
        local = None
        local = _attach(remote_viewer, remote_origin, sid)
        node = find_session(remote_viewer.tree(), local)
        assert (node["split"], node.get("hasSplit"), [s["kind"] for s in node["surfaces"]]) == (
            False, None, ["left"]
        ), node
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))


@pytest.mark.parametrize("remote_origin", ["instance", "headless"], indirect=True)
@pytest.mark.covers("live.attach", "session.split", "field:tree.session.split", "field:tree.session.hasSplit")
def test_a_split_hidden_on_the_viewer_stays_hidden_on_the_origin_and_on_the_next_attach(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="split-hide"))["id"]
    local = None
    try:
        origin.wait_mounted(sid, "left")
        local = _attach(remote_viewer, remote_origin, sid)
        _split_on_the_viewer(remote_viewer, local, origin, sid)
        assert_ok(remote_viewer.request("session.split", local, mode="off"))
        origin.wait_tree(lambda tree: find_session(tree, sid)["split"] is False, timeout=30)
        assert find_session(origin.tree(), sid).get("hasSplit") is True, "hidden, not closed"
        _detach(remote_viewer, local)
        local = None
        local = _attach(remote_viewer, remote_origin, sid)
        remote_viewer.wait_tree(
            lambda tree: (find_session(tree, local)["split"], find_session(tree, local).get("hasSplit")) == (False, True),
            timeout=30,
        )
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))


@pytest.mark.parametrize("remote_origin", ["instance", "headless"], indirect=True)
@pytest.mark.covers("session.swap", "session.split.close")
def test_closing_a_split_swapped_on_the_viewer_keeps_the_pane_shown_on_the_left(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="split-swap"))["id"]
    local = None
    try:
        origin.wait_mounted(sid, "left")
        local = _attach(remote_viewer, remote_origin, sid)
        _split_on_the_viewer(remote_viewer, local, origin, sid)
        _mark(origin, sid, "left", "QA-FIRST-PANE")
        _mark(origin, sid, "right", "QA-SECOND-PANE")
        assert_ok(remote_viewer.request("session.swap", local))
        assert_ok(remote_viewer.request("session.split.close", local))
        origin.wait_tree(lambda tree: not find_session(tree, sid).get("hasSplit"), timeout=30)
        text = assert_ok(origin.request("session.text", sid, all=True))["text"]
        assert "QA-SECOND-PANE" in text and "QA-FIRST-PANE" not in text, text
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))


@pytest.mark.parametrize("remote_origin", ["instance", "headless"], indirect=True)
@pytest.mark.covers("session.split.close", "field:tree.session.hud")
def test_close_split_on_a_row_whose_origin_session_is_gone_says_why_over_the_row(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name="split-gone"))["id"]
    closed = False
    local = None
    try:
        origin.wait_mounted(sid, "left")
        local = _attach(remote_viewer, remote_origin, sid)
        _split_on_the_viewer(remote_viewer, local, origin, sid)
        assert_ok(origin.request("session.close", sid))
        closed = True
        assert_ok(remote_viewer.request("session.split.close", local))

        def message():
            node = find_session(remote_viewer.tree(), local)
            return ((node or {}).get("hud") or {}).get("message")

        shown = wait_until(message, timeout=30)
        assert shown.startswith("Close Split: ") and sid.lower() in shown.lower(), shown
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        if not closed:
            assert_ok(origin.request("session.close", sid))
