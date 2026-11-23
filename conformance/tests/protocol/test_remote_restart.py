"""A pane of a session attached from another machine, restarted on its origin with `session.restart`, through the
private loopback sshd: the viewer's pane attaches to the new shell by itself, with no detach and no new attach, and
what the viewer types runs there. A viewer that attaches after the restart reads the new shell too."""

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-sessions", reason="the origin session must be Live"),
    pytest.mark.upstream_bug(
        "restart-publishes-no-respawn",
        reason="restartSessionPane (ControlServer+Restart.swift) never calls ownerRespawned, so a viewer stays on its "
        "exit prompt",
    ),
]

RESTARTED = "printf 'restarted-%s\\n' 7"
TYPED = "printf 'after-%s\\n' restart"
REPLAYED = "cat"
ECHOED = "replayed-cat-line"


def _attach(viewer, host, sid, pane):
    local = assert_ok(viewer.request(viewer.vocabulary.command("live.attach"), sid, host=host))["id"]
    viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
    viewer.wait_mounted(local, pane)
    viewer.wait_tree(lambda tree: _lead(tree, local, pane) == "leader")
    return local


def _follow_restart(viewer, local, pane):
    """Waits for the viewer's attach to the new shell, then takes the pane's lead. The origin's new attach leads the
    new shell and the viewer's attach claims nothing, so it follows: a state the viewer's leading attach before the
    restart cannot report."""
    viewer.wait_tree(lambda tree: _lead(tree, local, pane) == "follower")
    assert_ok(viewer.request("session.lead", local, pane=pane))
    viewer.wait_tree(lambda tree: _lead(tree, local, pane) == "leader")


def _lead(tree, local, pane):
    surfaces = find_session(tree, local)["surfaces"]
    return next(surface.get("lead") for surface in surfaces if surface["kind"] == pane)


@pytest.mark.parametrize("pane", ["left", "right"])
@pytest.mark.parametrize("replay", [False, True], ids=["command", "replay"])
@pytest.mark.covers("live.attach", "session.restart", "session.lead", "session.type")
def test_an_attached_pane_follows_a_restart_on_its_origin(remote_origin, remote_viewer, pane, replay):
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new", name=f"restart-{pane}"))["id"]
    local = None
    try:
        origin.wait_mounted(sid, "left")
        if pane == "right":
            assert_ok(origin.request("session.split", sid, mode="on"))
            origin.wait_mounted(sid, "right")
        origin.wait_response(
            origin.vocabulary.command("live.tree"), None,
            lambda response: any(row["id"] == sid for row in assert_ok(response)["remote"]["sessions"]),
        )
        local = _attach(remote_viewer, remote_origin.host, sid, pane)
        if replay:
            assert_ok(origin.request("session.type", sid, text=f"{REPLAYED}\n", pane=pane))
            origin.wait_tree(lambda tree: _foreground(tree, sid, pane) == [REPLAYED])
            assert_ok(origin.request("session.restart", sid, pane=pane))
            origin.wait_tree(lambda tree: _foreground(tree, sid, pane) == [REPLAYED])
            _follow_restart(remote_viewer, local, pane)
            assert_ok(remote_viewer.request("session.type", local, text=f"{ECHOED}\n", pane=pane))
            # the terminal echoes the line, and the replayed program prints it again
            origin.wait_text(sid, rf"(?m)^{ECHOED}\n{ECHOED}$", pane=pane)
        else:
            assert_ok(origin.request("session.restart", sid, pane=pane, command=RESTARTED))
            _follow_restart(remote_viewer, local, pane)
            remote_viewer.wait_text(local, r"(?m)^restarted-7$", pane=pane)
            assert_ok(remote_viewer.request("session.type", local, text=f"{TYPED}\n", pane=pane))
            origin.wait_text(sid, r"(?m)^after-restart$", pane=pane)
            remote_viewer.wait_text(local, r"(?m)^after-restart$", pane=pane)
        assert find_session(remote_viewer.tree(), local)["remoteSessionID"] == sid, "the viewer's row was replaced"

        assert_ok(remote_viewer.request("session.close", local))
        local = _attach(remote_viewer, remote_origin.host, sid, pane)
        remote_viewer.wait_text(local, ECHOED if replay else r"(?m)^after-restart$", pane=pane)
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        assert_ok(origin.request("session.close", sid))


def _foreground(tree, sid, pane):
    node = find_session(tree, sid)
    return node.get("foreground" if pane == "left" else "splitForeground")
