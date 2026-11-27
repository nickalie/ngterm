"""A remote window that mirrors the catalog of a real `ngterm-headless` (`remote_origin`, behind the harness's own sshd
on loopback): the window's `remote` node goes `connected` with the server's sessions as its rows, a session made,
renamed or closed on the server shows in the mirror, a server that stops answering shows `disconnected` and the window
connects again once it answers, and a second open of the server answers the window that already shows it. A command
that makes or changes rows in the remote window runs on the server, and the window shows its outcome once the catalog
brings it; a command that would make a shell or a row on this machine is refused there, and every one of them runs as
before in a local window.

The remote window is upstream's v0.42.0 feature, and test_remote_window.py keeps its since("0.42.0") cases. These cases
need a Live origin in any login shell, which only the Rust build claims (`live-any-shell`), so the reference skips them.
"""

import os
import signal
import time

import pytest
from agterm_conformance.match import assert_error, assert_ok, find_session, session_ids, window_ids
from agterm_conformance.plugin import Cli, Ctl
from agterm_conformance.remote_origin import ORIGIN_HOST
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-any-shell", reason="ngterm-headless runs every session Live, in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]

# The window's first snapshot comes after an ssh `version` probe and an ssh catalog stream.
CONNECT = 60.0
# The catalog stream's silence limit (35 s) with room for a loaded host.
SILENCE = 75.0
# After the server answers again: the next retry (at most 30 s away), its probe and its snapshot.
RECONNECT = 90.0
NOT_AVAILABLE = "{cmd} is not available in a remote window"


def _remote(viewer, window):
    """The `remote` node of `window` in `window.list`."""
    for node in assert_ok(viewer.request("window.list"))["windows"]:
        if node["id"] == window:
            return node.get("remote")
    pytest.fail(f"window {window} is not in window.list")


def _tree(viewer, window):
    return assert_ok(viewer.request("tree", window=window))


def _state(viewer, window):
    return (_remote(viewer, window) or {}).get("state")


def _catalog_id(origin):
    """The id of the catalog the server serves, from its stream's snapshot."""
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        deadline = time.monotonic() + 30
        while (remaining := deadline - time.monotonic()) > 0:
            frame = stream.read(remaining)
            if frame.get("kind") == "snapshot":
                return frame["snapshot"]["catalogID"]
            if frame.get("kind") == "ping":
                stream.send({"kind": "ack"})
    pytest.fail("no catalog snapshot within 30s")


def _remote_windows(viewer):
    return [node["id"] for node in assert_ok(viewer.request("window.list"))["windows"] if node.get("remote")]


def _local_window(viewer):
    windows = [node["id"] for node in assert_ok(viewer.request("window.list"))["windows"] if not node.get("remote")]
    assert windows, "the viewer has no local window"
    return windows[0]


@pytest.fixture(scope="module")
def mirror(remote_origin, remote_viewer):
    """The viewer's remote window of the origin, connected, with one session the origin made before the open."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    sid = assert_ok(origin.request("session.new", name="before-open"))["id"]
    window = assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"]
    wait_until(lambda: _state(viewer, window) == "connected", timeout=CONNECT)
    wait_until(lambda: sid in session_ids(_tree(viewer, window)), timeout=CONNECT)
    return window, sid


@pytest.mark.covers("window.remote.open", "window.list", "tree", "field:window.remote", "field:tree.remote")
def test_a_remote_window_shows_the_servers_sessions_as_its_rows(remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    remote = _remote(viewer, window)
    assert remote["host"] == ORIGIN_HOST and remote["state"] == "connected", remote
    assert "reason" not in remote, remote
    assert remote["catalogID"].lower() == _catalog_id(origin).lower(), remote
    tree = _tree(viewer, window)
    assert tree["tree"]["remote"] == remote, tree["tree"]
    assert sorted(session_ids(tree)) == sorted(session_ids(origin.tree())), (tree, origin.tree())
    assert find_session(tree, sid)["name"] == find_session(origin.tree(), sid)["name"] == "before-open"
    local = _local_window(viewer)
    assert "remote" not in _tree(viewer, local)["tree"]
    assert sid not in session_ids(_tree(viewer, local))


@pytest.mark.covers("window.remote.open", "session.new", "session.rename", "session.close", "tree")
def test_a_session_made_renamed_or_closed_on_the_server_shows_in_the_mirror(remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    made = assert_ok(origin.request("session.new", name="made-on-server"))["id"]
    try:
        tree = wait_until(lambda: made in session_ids(_tree(viewer, window)) and _tree(viewer, window))
        assert find_session(tree, made)["name"] == "made-on-server"

        assert_ok(origin.request("session.rename", sid, name="renamed-on-server"))
        wait_until(lambda: find_session(_tree(viewer, window), sid)["name"] == "renamed-on-server")
    finally:
        assert_ok(origin.request("session.close", made))
        wait_until(lambda: made not in session_ids(origin.tree()))
    wait_until(lambda: made not in session_ids(_tree(viewer, window)))
    assert sid in session_ids(_tree(viewer, window))


@pytest.mark.covers("window.remote.open", "window.list", "field:window.remote")
def test_a_server_that_stops_answering_shows_disconnected_then_connects_again(remote_origin, remote_viewer, mirror):
    """The server stopped (SIGSTOP) keeps its ssh connection open and says nothing: the stream's silence limit ends
    it, the window shows `disconnected` with a reason and keeps its rows, and the retries reach the server once it
    runs again."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    pid = remote_origin.instance.process.pid
    os.kill(pid, signal.SIGSTOP)
    try:
        remote = wait_until(
            lambda: (node := _remote(viewer, window)).get("state") == "disconnected" and node, timeout=SILENCE
        )
        assert remote["reason"], remote
        assert sid in session_ids(_tree(viewer, window))
    finally:
        os.kill(pid, signal.SIGCONT)
    wait_until(lambda: _state(viewer, window) == "connected", timeout=RECONNECT)
    assert "reason" not in _remote(viewer, window)

    made = assert_ok(origin.request("session.new", name="after-reconnect"))["id"]
    try:
        wait_until(lambda: made in session_ids(_tree(viewer, window)))
    finally:
        assert_ok(origin.request("session.close", made))
        wait_until(lambda: made not in session_ids(origin.tree()))


@pytest.mark.covers("window.remote.open", "window.list")
def test_a_second_open_of_the_server_answers_the_window_that_shows_it(remote_viewer, mirror):
    """The same host answers its window. Another destination that reaches the same server (`USER@HOST`) answers the
    window that already shows that catalog, and the window it opened for it goes."""
    import pwd

    viewer = remote_viewer
    window, _ = mirror
    assert assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"] == window
    alias = f"{pwd.getpwuid(os.getuid()).pw_name}@{ORIGIN_HOST}"
    assert assert_ok(viewer.request("window.remote.open", host=alias))["id"] == window
    wait_until(lambda: _remote_windows(viewer) == [window])
    assert _state(viewer, window) == "connected"


@pytest.mark.covers("window.remote.open")
@pytest.mark.parametrize("host", ["-oProxyCommand=x", "a b"], ids=["option", "spaced"])
def test_a_host_ssh_cannot_be_given_opens_no_window(remote_viewer, mirror, host):
    viewer = remote_viewer
    before = window_ids(viewer.request("window.list"))
    assert_error(viewer.request("window.remote.open", host=host), "window.remote.open: not a host ssh can be given")
    assert window_ids(viewer.request("window.list")) == before


def _workspace_names(tree):
    return [node["name"] for node in tree["tree"]["workspaces"]]


def _settled(viewer, window, origin, check):
    """The remote window's tree once `check(tree, origin_tree)` holds, no row reading back `pending`."""

    def ready():
        tree, origin_tree = _tree(viewer, window), origin.tree()
        rows = [row for workspace in tree["tree"]["workspaces"] for row in workspace["sessions"]]
        return check(tree, origin_tree) and not any("pending" in row for row in rows) and tree

    return wait_until(ready, timeout=CONNECT)


@pytest.mark.covers(
    "session.new", "session.rename", "session.split", "session.duplicate", "session.close", "workspace.new",
    "workspace.rename", "workspace.delete", "tree", "field:tree.session.pending", "field:window.remote",
)
def test_a_row_command_in_a_remote_window_runs_on_the_server(remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    assert_ok(viewer.request("window.select", window))
    made = assert_ok(viewer.request("session.new", window=window, name="made-here"))["id"]
    copy = None
    workspace = None
    try:
        assert made in session_ids(origin.tree())
        _settled(viewer, window, origin, lambda tree, _: made in session_ids(tree))

        assert_ok(viewer.request("session.rename", made, name="-renamed-here"))
        assert_ok(viewer.request("session.split", made, mode="on"))
        tree = _settled(
            viewer, window, origin,
            lambda tree, served: find_session(served, made)["name"] == "-renamed-here"
            and find_session(served, made).get("hasSplit") is True
            and find_session(tree, made)["name"] == "-renamed-here"
            and find_session(tree, made).get("hasSplit") is True,
        )
        assert tree["tree"]["remote"]["state"] == "connected"
        assert "operations" not in tree["tree"]["remote"], tree["tree"]["remote"]

        copy = assert_ok(viewer.request("session.duplicate", made))["id"]
        workspace = assert_ok(viewer.request("workspace.new", window=window, name="made-here"))["id"]
        assert_ok(viewer.request("workspace.rename", workspace, name="renamed-here"))
        _settled(
            viewer, window, origin,
            lambda tree, served: copy in session_ids(tree) and copy in session_ids(served)
            and "renamed-here" in _workspace_names(tree) and "renamed-here" in _workspace_names(served),
        )
    finally:
        for closed in (copy, made):
            if closed:
                assert_ok(viewer.request("session.close", closed, window=window))
        if workspace:
            assert_ok(viewer.request("workspace.delete", workspace, window=window))
    _settled(
        viewer, window, origin,
        lambda tree, served: made not in session_ids(served) and copy not in session_ids(served)
        and made not in session_ids(tree) and copy not in session_ids(tree)
        and "renamed-here" not in _workspace_names(tree) and "renamed-here" not in _workspace_names(served),
    )
    assert _state(viewer, window) == "connected"


@pytest.mark.covers("session.companion.convert", "recent.open", "live.attach")
def test_a_command_that_makes_a_local_row_is_refused_in_a_remote_window(remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    before, origin_before = session_ids(_tree(viewer, window)), session_ids(origin.tree())
    for cmd, target, args in [
        ("session.companion.convert", sid, {}),
        ("recent.open", None, {}),
        ("live.attach", None, {"name": "x"}),
    ]:
        assert_error(viewer.request(cmd, target, window=window, **args), NOT_AVAILABLE.format(cmd=cmd))
    assert session_ids(_tree(viewer, window)) == before
    assert session_ids(origin.tree()) == origin_before


@pytest.mark.covers("cli:session convert", "cli:window remote dismiss", "window.remote.dismiss")
def test_the_cli_reports_a_local_only_refusal_and_dismisses_nothing_unknown(adapter, remote_viewer, mirror):
    viewer = remote_viewer
    window, sid = mirror
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    result = cli("session", "convert", "--companion", "--target", sid)
    assert (result.returncode, result.stdout) == (1, ""), (result.argv, result.returncode, result.stdout)
    assert result.stderr == f"error: {NOT_AVAILABLE.format(cmd='session.companion.convert')}\n", result.stderr
    dismissed = cli("window", "remote", "dismiss", window)
    assert (dismissed.returncode, dismissed.stdout, dismissed.stderr) == (0, "0 dismissed\n", ""), dismissed.argv
    assert sid in session_ids(_tree(viewer, window))


@pytest.mark.covers(
    "session.new", "session.rename", "session.close", "session.split", "session.duplicate", "workspace.new",
    "workspace.rename", "workspace.delete",
)
def test_the_same_row_commands_run_in_a_local_window(remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    remote_before, origin_before = session_ids(_tree(viewer, window)), session_ids(origin.tree())
    local = _local_window(viewer)
    assert_ok(viewer.request("window.select", local))
    sid = assert_ok(viewer.request("session.new", window=local))["id"]
    viewer.wait_mounted(sid, "left")
    assert_ok(viewer.request("session.rename", sid, name="renamed-here"))
    assert_ok(viewer.request("session.split", sid, mode="on"))
    copy = assert_ok(viewer.request("session.duplicate", sid))["id"]
    workspace = assert_ok(viewer.request("workspace.new", name="made-here", window=local))["id"]
    assert_ok(viewer.request("workspace.rename", workspace, name="renamed-here"))
    tree = _tree(viewer, local)
    assert find_session(tree, sid)["name"] == "renamed-here"
    assert find_session(tree, sid).get("hasSplit") is True
    assert copy in session_ids(tree)
    assert "renamed-here" in [node["name"] for node in tree["tree"]["workspaces"]]
    assert session_ids(_tree(viewer, window)) == remote_before
    assert session_ids(origin.tree()) == origin_before
    for closed in (copy, sid):
        assert_ok(viewer.request("session.close", closed))
    assert_ok(viewer.request("workspace.delete", workspace))
    tree = _tree(viewer, local)
    assert sid not in session_ids(tree) and copy not in session_ids(tree)
    assert workspace not in [node["id"] for node in tree["tree"]["workspaces"]]
