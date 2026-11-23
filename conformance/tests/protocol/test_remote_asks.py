"""Questions (`ask.open`) asked from a pane of a session attached from another machine (`live attach HOST ID`), against
an origin behind the harness's own sshd (`remote_origin`): a second instance of the build, or the staged
`ngterm-headless`. The pane's program runs on the origin and asks the origin's ngterm, which hands the question to the
viewer presenting the session.

A desktop origin hands it over only while the pane follows the viewer's lead, as upstream does. A headless origin draws
nothing, so it hands the question over whatever the lead, as it does a page (upstream v0.42.3's `presentsRemotely`)."""

import json
import shlex

import pytest
from agterm_conformance.match import assert_ok, find_session, held_interactions, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]

ANSWER_FILE = "remote-ask.json"


def _origin_session(origin):
    sid = assert_ok(origin.request("session.new"))["id"]
    origin.wait_mounted(sid, "left")
    return sid


def _attach(viewer, host, sid):
    """`live attach HOST ID` on the viewer: the local row's id, once its pane reads back."""
    local = assert_ok(viewer.request("live.attach", sid, host=host))["id"]
    viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
    return local


def _lead(sid, role):
    def leads(tree):
        (left,) = [surface for surface in find_session(tree, sid)["surfaces"] if surface["kind"] == "left"]
        assert left.get("lead") == role, left
        return True

    leads.__qualname__ = f"the left pane of {sid} reading {role}"
    return leads


def _presented(tree, sid):
    assert find_session(tree, sid).get("presenters", {}).get("presenter") is True, find_session(tree, sid)
    return True


def _ask_from_the_pane(remote_origin, origin, sid, home, style="terminal", pane=None):
    """`ngtermctl ask` typed into the session's pane (`pane`: `right` for its split), aimed at its own session as an
    agent there aims it: its answer goes to a file in the origin's home."""
    (home / ANSWER_FILE).unlink(missing_ok=True)
    command = (
        f"{remote_origin.adapter.ctl_name} ask {shlex.quote('Proceed?')} --button yes --button no --style {style} "
        f'--target "${origin.instance.variable("SESSION_ID")}" > {shlex.quote(ANSWER_FILE)} 2>&1\n'
    )
    assert_ok(origin.request("session.type", sid, text=command, **({"pane": pane} if pane else {})))

    def opened(tree):
        ask = find_session(tree, sid).get("ask")
        assert ask, "no ask on the session"
        return ask

    return find_session(origin.wait_tree(opened), sid)["ask"]


def _answer(home):
    path = home / ANSWER_FILE

    def written():
        text = path.read_text() if path.exists() else ""
        assert text.strip(), f"{path} is empty"
        return text

    return json.loads(wait_until(written))


@pytest.mark.parametrize("remote_origin", ["instance", "headless"], indirect=True)
@pytest.mark.covers("ask.open", "ask.cancel", "field:tree.session.ask")
def test_an_ask_from_an_attached_pane_shows_on_the_viewer(remote_origin, remote_viewer):
    origin = Ctl(remote_origin.instance)
    home = remote_origin.instance.home
    before = set(session_ids(origin.tree()))
    sid = _origin_session(origin)
    # a desktop origin's window leads the session it shows: it shows another one
    assert_ok(origin.request("session.select", sorted(before)[0]))
    local = _attach(remote_viewer, remote_origin.host, sid)
    origin.wait_tree(lambda tree: _presented(tree, sid))
    origin.wait_tree(_lead(sid, "follower"))

    ask = _ask_from_the_pane(remote_origin, origin, sid, home)
    assert ask.get("remote") is True, ask
    remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("ask") == {"id": ask["id"], "replica": True})
    assert_ok(origin.request("ask.cancel", ask["id"]))
    assert _answer(home) == {"result": "cancelled"}
    remote_viewer.wait_tree(lambda tree: "ask" not in find_session(tree, local))

    # the origin's own client takes the lead back, as a desktop origin's window does when the user works there
    assert_ok(origin.request("session.lead", sid))
    origin.wait_tree(_lead(sid, "leader"))
    ask = _ask_from_the_pane(remote_origin, origin, sid, home)
    if remote_origin.kind == "headless":
        assert ask.get("remote") is True, f"a headless origin keeps the ask where nobody sees it: {ask}"
        remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("ask") == {"id": ask["id"], "replica": True})
    else:
        assert "remote" not in ask, f"a desktop origin that leads draws the ask itself: {ask}"
    assert_ok(origin.request("ask.cancel", ask["id"]))
    assert _answer(home) == {"result": "cancelled"}

    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))


@pytest.mark.parametrize(
    ("remote_origin", "style"),
    [("instance", "terminal"), ("headless", "terminal"), ("headless", "gui")],
    indirect=["remote_origin"],
)
@pytest.mark.covers("ask.open", "ask.cancel", "session.split", "field:tree.session.ask")
def test_an_ask_from_an_attached_pane_while_a_new_split_on_the_origin_leads(remote_origin, remote_viewer, style):
    """A split made on the origin leads there while the first pane follows the viewer. A headless origin still hands
    the ask over, a terminal or a GUI one, from either pane; a desktop origin keeps it, since the session's panes do
    not all follow (upstream rule)."""
    origin = Ctl(remote_origin.instance)
    home = remote_origin.instance.home
    before = set(session_ids(origin.tree()))
    sid = _origin_session(origin)
    assert_ok(origin.request("session.select", sorted(before)[0]))
    local = _attach(remote_viewer, remote_origin.host, sid)
    origin.wait_tree(lambda tree: _presented(tree, sid))
    origin.wait_tree(_lead(sid, "follower"))
    assert_ok(origin.request("session.split", sid, mode="on"))
    origin.wait_mounted(sid, "left", "right")

    def split_leads(tree):
        roles = {surface["kind"]: surface.get("lead") for surface in find_session(tree, sid)["surfaces"]}
        assert roles == {"left": "follower", "right": "leader"}, roles
        return True

    origin.wait_tree(split_leads)
    for pane in ("left", "right"):
        ask = _ask_from_the_pane(remote_origin, origin, sid, home, style=style, pane=pane)
        if remote_origin.kind == "headless":
            assert ask.get("remote") is True, f"a headless origin keeps the ask from the {pane} pane: {ask}"
            remote_viewer.wait_tree(
                lambda tree, ask=ask: find_session(tree, local).get("ask") == {"id": ask["id"], "replica": True}
            )
        else:
            assert "remote" not in ask, f"a desktop origin with a leading split draws the ask itself: {ask}"
        assert_ok(origin.request("ask.cancel", ask["id"]))
        assert _answer(home) == {"result": "cancelled"}
        remote_viewer.wait_tree(lambda tree: "ask" not in find_session(tree, local))

    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))


@pytest.mark.since("0.42.0")
@pytest.mark.parametrize("remote_origin", ["headless"], indirect=True)
@pytest.mark.covers("ask.open", "ask.result", "ask.cancel", "session.select", "field:tree.session.ask",
                    "field:tree.session.interactions")
def test_a_headless_origins_gui_ask_for_a_row_not_on_screen_waits_hidden_on_the_viewer(remote_origin, remote_viewer):
    """A viewer refuses a GUI ask for a row it does not show, so that a desktop origin shows it itself; a headless origin
    has no screen to take it back to, so the viewer keeps it hidden with the row until the row is shown."""
    origin = Ctl(remote_origin.instance)
    sid = _origin_session(origin)
    local = _attach(remote_viewer, remote_origin.host, sid)
    origin.wait_tree(lambda tree: _presented(tree, sid))
    own = assert_ok(remote_viewer.request("session.new"))["id"]
    assert_ok(remote_viewer.request("session.select", own))

    ask = assert_ok(origin.request("ask.open", sid, title="Proceed?", style="gui",
                                   buttons=[{"id": "yes", "label": "Yes"}]))["id"]
    remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("ask") == {"id": ask, "replica": True})
    assert held_interactions(origin.tree(), sid) == [("ask", "offered")], \
        "the viewer took the ask, and did not refuse it back to the origin"
    assert assert_ok(origin.request("ask.result", ask))["ask"] == {"result": "pending"}
    assert_ok(remote_viewer.request("session.select", local))
    assert find_session(remote_viewer.tree(), local).get("ask") == {"id": ask, "replica": True}
    assert_ok(origin.request("ask.cancel", ask))
    remote_viewer.wait_tree(lambda tree: "ask" not in find_session(tree, local))

    assert_ok(remote_viewer.request("session.close", own))
    assert_ok(remote_viewer.request("session.close", local))
    assert_ok(origin.request("session.close", sid))
