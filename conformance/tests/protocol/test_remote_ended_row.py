"""A row attached from another machine whose session ends there: the row keeps its exit line, its presentation stream
stops in the final `ended` state with the `mirror` mode, and the viewer starts no more ssh to the origin for it.
`tests/ui/test_remote_ended_row_ui.py` presses a key on the exit line.

Upstream's viewer retries the stream of such a row for as long as the row exists: after 1, 2, 4, 8, 16 then 30
seconds, then every 5 minutes, each try failing with "no such session". The stop is this build's fix, so the reference
gets an allowance. The test counts the stream's launches in the private ssh client's call log (`SshClient.calls`)."""

import time

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

HELD_PROMPT = r"Process exited\. Press any key to close the terminal\."
# Longer than the first four retries of the stream's backoff (1 + 2 + 4 + 8 seconds), so a stream that still retried
# would launch again in it.
QUIET = 20.0

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
    """The viewer's ssh launches of the presentation stream of origin session `sid`."""
    return [
        argv
        for argv in remote_origin.client.invocations()
        if any("live present" in word and sid in word for word in argv)
    ]


def _state(tree, local):
    return find_session(tree, local).get("presentation", {}).get("state")


@pytest.mark.covers("live.attach", "session.type", "field:tree.session.presentation")
def test_a_row_whose_session_ended_on_its_origin_stops_its_stream(remote_origin, remote_viewer):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    sid = assert_ok(origin.request("session.new", name="ended-row"))["id"]
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
        assert _presents(remote_origin, sid), remote_origin.client.invocations()

        assert_ok(origin.request("session.type", sid, text="exit\n"))
        ended = True
        origin.wait_tree(lambda tree: sid not in session_ids(tree))
        viewer.wait_text(local, HELD_PROMPT, pane="left")
        viewer.wait_tree(lambda tree: _state(tree, local) == "ended")
        assert find_session(viewer.tree(), local)["presentation"]["mode"] == "mirror"

        launches = len(_presents(remote_origin, sid))
        time.sleep(QUIET)
        assert len(_presents(remote_origin, sid)) == launches, _presents(remote_origin, sid)
        assert _state(viewer.tree(), local) == "ended"
        viewer.wait_text(local, HELD_PROMPT, pane="left")
    finally:
        if local is not None:
            viewer.request("session.close", local)
        if not ended:
            origin.request("session.close", sid)
        wait_until(lambda: sid not in session_ids(origin.tree()))
