"""A key on the exit line of a row whose session ended on its origin closes the pane, and with it the row, as on any held
exit; the viewer starts no ssh to the origin for it. `tests/protocol/test_remote_ended_row.py` has the stream's stop.

The `ended` state is this build's: upstream's row retries its stream for as long as it exists, so the reference gets
an allowance."""

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

HELD_PROMPT = r"Process exited\. Press any key to close the terminal\."

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", "live-any-shell", any=True, reason="the origin's session must be Live"),
    pytest.mark.requires("posix-shell", reason="the test types `exit` into the origin pane's shell"),
    pytest.mark.upstream_bug(
        "ended-row-retries-forever",
        reason="the presentation client (RemotePresentationClient.swift) retries a row whose session ended on its "
        "origin for as long as the row exists, and tree has no `ended` state",
    ),
]


def _presents(remote_origin, sid):
    return [
        argv
        for argv in remote_origin.client.invocations()
        if any("live present" in word and sid in word for word in argv)
    ]


def _state(tree, local):
    return find_session(tree, local).get("presentation", {}).get("state")


@pytest.mark.covers("live.attach", "session.select", "behavior:pane.typing")
def test_a_key_on_the_exit_line_of_an_ended_remote_row_closes_it(remote_origin, remote_viewer, ui_for):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    sid = assert_ok(origin.request("session.new", name="ended-row-key"))["id"]
    ended = False
    local = None
    try:
        origin.wait_mounted(sid, "left")
        origin.wait_response(
            origin.vocabulary.command("live.tree"), None,
            lambda response: any(row["id"] == sid for row in assert_ok(response)["remote"]["sessions"]),
        )
        local = assert_ok(viewer.request(viewer.vocabulary.command("live.attach"), sid, host=remote_origin.host))["id"]
        viewer.wait_mounted(local, "left")
        viewer.wait_tree(lambda tree: _state(tree, local) == "connected")

        assert_ok(origin.request("session.type", sid, text="exit\n"))
        ended = True
        origin.wait_tree(lambda tree: sid not in session_ids(tree))
        viewer.wait_text(local, HELD_PROMPT, pane="left")
        viewer.wait_tree(lambda tree: _state(tree, local) == "ended")
        launches = len(_presents(remote_origin, sid))

        assert_ok(viewer.request("session.select", local))
        viewer.wait_tree(lambda tree: find_session(tree, local)["active"])
        ui_for(viewer).press("x")
        viewer.wait_tree(lambda tree: local not in session_ids(tree))
        local = None
        assert len(_presents(remote_origin, sid)) == launches, _presents(remote_origin, sid)
    finally:
        if local is not None:
            viewer.request("session.close", local)
        if not ended:
            origin.request("session.close", sid)
        wait_until(lambda: sid not in session_ids(origin.tree()))
