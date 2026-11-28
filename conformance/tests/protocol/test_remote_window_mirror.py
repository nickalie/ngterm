"""A remote window that mirrors the catalog of a real `ngterm-headless` (`remote_origin`, behind the harness's own sshd
on loopback): the window's `remote` node goes `connected` with the server's sessions as its rows, a session made,
renamed or closed on the server shows in the mirror, a server that stops answering shows `disconnected` and the window
connects again once it answers, and a second open of the server answers the window that already shows it. A command
that makes or changes rows in the remote window runs on the server, and the window shows its outcome once the catalog
brings it; a command that would make a shell or a row on this machine is refused there, and every one of them runs as
before in a local window. The CLI cases give the actions as a user does: a made row is selected when it arrives, an
action while the server is not connected is refused and never sent later, a reply lost in flight reads `unknown`
until `window remote dismiss` clears it, and a scratch shown in the window is the server's scratch, typed into from
either side.

The remote window is upstream's v0.42.0 feature, and test_remote_window.py keeps its since("0.42.0") cases. These cases
need a Live origin in any login shell, which only the Rust build claims (`live-any-shell`), so the reference skips them.
"""

import os
import signal
import time

import pytest
from agterm_conformance.match import (
    active_session_id,
    assert_error,
    assert_ok,
    find_session,
    session_ids,
    window_ids,
)
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


# -- the row actions as a user gives them with the CLI

LOST = "{host} did not answer {action}; its outcome is unknown until {host} can be asked"
NOT_CONNECTED = "{host} is not connected"
# An action whose reply never comes: the viewer's 30 s wait for the ssh run, with room for a loaded host.
LOST_REPLY = 75.0


def _cli(adapter, viewer, window):
    """The viewer's CLI, once the module's shared window is connected: a case before may have stopped the server."""
    wait_until(lambda: _state(viewer, window) == "connected", timeout=RECONNECT)
    return Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)


def _made(result):
    """The id a CLI `--json` reply of a command that makes a row carries."""
    assert (result.returncode, result.stderr) == (0, ""), (result.argv, result.returncode, result.stderr)
    return result.json()["result"]["id"]


def _done(result):
    assert (result.returncode, result.stderr) == (0, ""), (result.argv, result.returncode, result.stderr)


def _refused_by(result, error):
    assert (result.returncode, result.stdout, result.stderr) == (1, "", f"error: {error}\n"), (
        result.argv, result.returncode, result.stdout, result.stderr,
    )


def _workspace_of(tree, sid):
    for workspace in tree["tree"]["workspaces"]:
        if sid in [session["id"] for session in workspace["sessions"]]:
            return workspace["id"]
    return None


def _workspace_ids(tree):
    return [node["id"] for node in tree["tree"]["workspaces"]]


def _panes(tree, sid):
    """Session `sid`'s pane ids by pane kind (`left`, `right`)."""
    surfaces = find_session(tree, sid).get("surfaces") or []
    return {surface["kind"]: surface.get("paneID") for surface in surfaces if surface["kind"] in ("left", "right")}


def _close_on_origin(origin, *sids):
    for sid in sids:
        if sid and sid in session_ids(origin.tree()):
            assert_ok(origin.request("session.close", sid))


def _probe(viewer, window, origin, name):
    """Makes a session in the remote window and waits until the mirror shows it settled: an action sent before it,
    queued or not, has reached the server by then."""
    probe = assert_ok(viewer.request("session.new", window=window, name=name, noSelect=True))["id"]
    _settled(viewer, window, origin, lambda tree, served: probe in session_ids(tree) and probe in session_ids(served))
    return probe


@pytest.mark.covers("cli:session new", "session.new", "field:tree.session.pending")
def test_a_row_made_in_a_remote_window_is_selected_when_it_arrives(adapter, remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    cli = _cli(adapter, viewer, window)
    picked = quiet = None
    try:
        picked = _made(cli("session", "new", "--window", window, "--name", "picked", "--json"))
        tree = _settled(
            viewer, window, origin, lambda tree, served: picked in session_ids(served) and picked in session_ids(tree)
        )
        wait_until(lambda: active_session_id(_tree(viewer, window)) == picked)
        assert find_session(tree, picked)["name"] == "picked"

        quiet = _made(cli("session", "new", "--window", window, "--name", "quiet", "--no-select", "--json"))
        _settled(
            viewer, window, origin, lambda tree, served: quiet in session_ids(served) and quiet in session_ids(tree)
        )
        assert active_session_id(_tree(viewer, window)) == picked
    finally:
        _close_on_origin(origin, quiet, picked)
    _settled(viewer, window, origin, lambda tree, _: picked not in session_ids(tree) and quiet not in session_ids(tree))
    assert sid in session_ids(_tree(viewer, window))


@pytest.mark.covers(
    "cli:session split", "cli:session swap", "cli:session split close", "cli:session new", "session.split",
    "session.swap", "session.split.close", "session.companion.new",
)
def test_split_swap_and_a_companion_in_a_remote_window_run_on_the_server(
    adapter, remote_origin, remote_viewer, mirror
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    cli = _cli(adapter, viewer, window)
    made = companion = None
    try:
        made = _made(cli("session", "new", "--window", window, "--name", "panes", "--no-select", "--json"))
        _settled(viewer, window, origin, lambda tree, served: made in session_ids(tree) and made in session_ids(served))

        _done(cli("session", "split", "on", "--target", made))
        _settled(
            viewer, window, origin,
            lambda tree, served: find_session(served, made).get("hasSplit") is True
            and find_session(tree, made).get("hasSplit") is True
            and len(_panes(served, made)) == 2,
        )
        before = _panes(origin.tree(), made)
        _done(cli("session", "swap", "--target", made))
        swapped = {"left": before["right"], "right": before["left"]}
        _settled(viewer, window, origin, lambda _, served: _panes(served, made) == swapped)

        _done(cli("session", "split", "close", "--target", made))
        _settled(
            viewer, window, origin,
            lambda tree, served: not find_session(served, made).get("hasSplit")
            and not find_session(tree, made).get("hasSplit"),
        )

        companion = _made(cli("session", "new", "--parent", made, "--no-select", "--json"))
        tree = _settled(
            viewer, window, origin,
            lambda tree, served: companion in session_ids(served) and companion in session_ids(tree),
        )
        assert find_session(origin.tree(), companion).get("parent") == made
        assert find_session(tree, companion).get("parent") == made
    finally:
        _close_on_origin(origin, companion, made)
    _settled(
        viewer, window, origin, lambda tree, _: made not in session_ids(tree) and companion not in session_ids(tree)
    )


@pytest.mark.covers("cli:session scratch", "session.scratch")
def test_a_scratch_shown_or_hidden_in_a_remote_window_runs_on_the_server(adapter, remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    cli = _cli(adapter, viewer, window)
    made = None
    try:
        made = _made(cli("session", "new", "--window", window, "--name", "scratch", "--no-select", "--json"))
        _settled(viewer, window, origin, lambda tree, served: made in session_ids(tree) and made in session_ids(served))
        for mode, shown in (("on", True), ("off", False)):
            _done(cli("session", "scratch", mode, "--target", made))
            _settled(
                viewer, window, origin,
                lambda tree, served, shown=shown: find_session(served, made)["scratch"] is shown
                and find_session(tree, made)["scratch"] is shown,
            )
    finally:
        _close_on_origin(origin, made)
    _settled(viewer, window, origin, lambda tree, _: made not in session_ids(tree))


def _scratch_text(cli, sid, *where):
    result = cli("session", "text", "--all", "--pane", "scratch", "--target", sid, *where, "--json")
    return result.json()["result"]["text"] if result.returncode == 0 else ""


@pytest.mark.covers("cli:session scratch", "cli:session type", "cli:session text", "session.type", "session.text")
def test_a_scratch_in_a_remote_window_is_the_servers_scratch_both_ways(adapter, remote_origin, remote_viewer, mirror):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    cli = _cli(adapter, viewer, window)
    server_cli = Cli(adapter.ctl_path, remote_origin.instance, adapter.ctl_name)
    made = None
    try:
        made = _made(cli("session", "new", "--window", window, "--name", "shared", "--no-select", "--json"))
        _settled(viewer, window, origin, lambda tree, served: made in session_ids(tree) and made in session_ids(served))
        _done(cli("session", "scratch", "on", "--target", made, "--window", window))
        _settled(
            viewer, window, origin,
            lambda tree, served: find_session(served, made)["scratch"] and find_session(tree, made)["scratch"],
        )
        # printf joins the halves, so only the command's output matches, never its echo.
        _done(server_cli("session", "type", "--pane", "scratch", "--target", made, "printf '%s%s\\n' FROM- SERVER\n"))
        wait_until(lambda: "FROM-SERVER" in _scratch_text(cli, made, "--window", window), timeout=CONNECT)
        _done(cli("session", "type", "--pane", "scratch", "--target", made, "--window", window,
                  "printf '%s%s\\n' FROM- VIEWER\n"))
        wait_until(lambda: "FROM-VIEWER" in _scratch_text(server_cli, made), timeout=CONNECT)
        wait_until(lambda: "FROM-VIEWER" in _scratch_text(cli, made, "--window", window), timeout=CONNECT)
    finally:
        _close_on_origin(origin, made)
    _settled(viewer, window, origin, lambda tree, _: made not in session_ids(tree))


@pytest.mark.covers("cli:session move", "cli:workspace move", "cli:workspace new", "session.move", "workspace.move")
def test_a_session_and_a_workspace_moved_in_a_remote_window_move_on_the_server(
    adapter, remote_origin, remote_viewer, mirror
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, _ = mirror
    cli = _cli(adapter, viewer, window)
    workspace = made = None
    try:
        workspace = _made(cli("workspace", "new", "moved-to", "--window", window, "--json"))
        made = _made(cli("session", "new", "--window", window, "--name", "mover", "--no-select", "--json"))
        _settled(
            viewer, window, origin,
            lambda tree, served: workspace in _workspace_ids(served) and workspace in _workspace_ids(tree)
            and made in session_ids(served) and made in session_ids(tree),
        )
        assert _workspace_of(origin.tree(), made) != workspace

        _done(cli("session", "move", workspace, "--target", made))
        _settled(
            viewer, window, origin,
            lambda tree, served: _workspace_of(served, made) == workspace and _workspace_of(tree, made) == workspace,
        )

        for to, index in (("top", 0), ("bottom", -1)):
            _done(cli("workspace", "move", "--to", to, "--target", workspace))
            tree = _settled(
                viewer, window, origin,
                lambda tree, served, index=index: _workspace_ids(served)[index] == workspace
                and _workspace_ids(tree)[index] == workspace,
            )
            assert _workspace_ids(tree) == _workspace_ids(origin.tree())
    finally:
        _close_on_origin(origin, made)
        if workspace and workspace in _workspace_ids(origin.tree()):
            assert_ok(origin.request("workspace.delete", workspace))
    _settled(viewer, window, origin, lambda tree, _: workspace not in _workspace_ids(tree))


@pytest.mark.covers("cli:session duplicate", "session.companion.duplicate", "live.attach.group")
def test_the_still_local_commands_are_refused_in_a_remote_window_and_change_nothing(
    adapter, remote_origin, remote_viewer, mirror
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    cli = _cli(adapter, viewer, window)
    before, origin_before = _tree(viewer, window), origin.tree()

    _refused_by(
        cli("session", "duplicate", "--companion", "--target", sid),
        NOT_AVAILABLE.format(cmd="session.companion.duplicate"),
    )
    for cmd, target, args in [
        ("session.companion.duplicate", sid, {}),
        ("live.attach.group", sid, {"host": ORIGIN_HOST}),
    ]:
        assert_error(viewer.request(cmd, target, window=window, **args), NOT_AVAILABLE.format(cmd=cmd))

    after = _tree(viewer, window)
    assert session_ids(after) == session_ids(before)
    assert _workspace_ids(after) == _workspace_ids(before)
    assert session_ids(origin.tree()) == session_ids(origin_before)
    assert "operations" not in after["tree"]["remote"], after["tree"]["remote"]


@pytest.mark.covers("cli:session new", "cli:session rename", "cli:workspace new", "session.new", "session.rename")
def test_an_action_while_the_server_is_not_connected_is_refused_and_never_sent(
    adapter, remote_origin, remote_viewer, mirror
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    cli = _cli(adapter, viewer, window)
    name = find_session(origin.tree(), sid)["name"]
    refusal = NOT_CONNECTED.format(host=ORIGIN_HOST)
    pid = remote_origin.instance.process.pid
    os.kill(pid, signal.SIGSTOP)
    try:
        wait_until(lambda: _state(viewer, window) == "disconnected", timeout=SILENCE)
        _refused_by(cli("session", "new", "--window", window, "--name", "queued"), refusal)
        _refused_by(cli("session", "rename", "queued-name", "--target", sid), refusal)
        _refused_by(cli("workspace", "new", "queued", "--window", window), refusal)
        tree = _tree(viewer, window)
        assert "operations" not in tree["tree"]["remote"], tree["tree"]["remote"]
        assert "pending" not in find_session(tree, sid), find_session(tree, sid)
    finally:
        os.kill(pid, signal.SIGCONT)
    wait_until(lambda: _state(viewer, window) == "connected", timeout=RECONNECT)

    probe = _probe(viewer, window, origin, "after-refusals")
    try:
        served = origin.tree()
        assert "queued" not in [find_session(served, each)["name"] for each in session_ids(served)]
        assert "queued" not in _workspace_names(served)
        assert find_session(served, sid)["name"] == name
    finally:
        _close_on_origin(origin, probe)


@pytest.mark.covers(
    "cli:session rename", "cli:session new", "cli:window remote dismiss", "window.remote.dismiss",
    "field:tree.session.pending", "field:window.remote",
)
def test_a_lost_reply_reads_unknown_until_dismissed_and_is_never_sent_again(
    adapter, remote_origin, remote_viewer, mirror
):
    """The server stopped after the window took the action: the reply never comes, the row reads `pending` and then
    `unknown`, and `window remote dismiss` clears it. Once the server runs again, the action ran there at most once."""
    import threading

    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    window, sid = mirror
    cli = _cli(adapter, viewer, window)
    name = find_session(origin.tree(), sid)["name"]
    results = {}

    def run(key, *args):
        results[key] = cli(*args, timeout=LOST_REPLY + 30)

    pid = remote_origin.instance.process.pid
    os.kill(pid, signal.SIGSTOP)
    try:
        threads = [
            threading.Thread(target=run, args=("rename", "session", "rename", "lost-name", "--target", sid)),
            threading.Thread(
                target=run, args=("new", "session", "new", "--window", window, "--name", "lost-new", "--no-select")
            ),
        ]
        for thread in threads:
            thread.start()
        pending = wait_until(
            lambda: (node := _remote(viewer, window)) and len(node.get("operations", [])) == 2 and node["operations"]
        )
        assert sorted(operation["action"] for operation in pending) == ["New Session", "Rename Session"], pending
        assert {operation["state"] for operation in pending} == {"pending"}, pending
        assert find_session(_tree(viewer, window), sid).get("pending") == "pending"
        for thread in threads:
            thread.join(LOST_REPLY + 40)
            assert not thread.is_alive(), "the CLI got no answer"
        _refused_by(results["rename"], LOST.format(host=ORIGIN_HOST, action="Rename Session"))
        _refused_by(results["new"], LOST.format(host=ORIGIN_HOST, action="New Session"))

        tree = _tree(viewer, window)
        assert find_session(tree, sid).get("pending") == "unknown", find_session(tree, sid)
        operations = tree["tree"]["remote"]["operations"]
        assert {operation["state"] for operation in operations} == {"unknown"}, operations
        assert [operation for operation in operations if operation["action"] == "Rename Session"][0]["session"] == sid

        dismissed = cli("window", "remote", "dismiss", window)
        assert (dismissed.returncode, dismissed.stdout, dismissed.stderr) == (0, "2 dismissed\n", ""), dismissed
        tree = _tree(viewer, window)
        assert "operations" not in tree["tree"]["remote"], tree["tree"]["remote"]
        assert "pending" not in find_session(tree, sid), find_session(tree, sid)
        again = cli("window", "remote", "dismiss", window)
        assert (again.returncode, again.stdout, again.stderr) == (0, "0 dismissed\n", ""), again
    finally:
        os.kill(pid, signal.SIGCONT)
    wait_until(lambda: _state(viewer, window) == "connected", timeout=RECONNECT)

    probe = _probe(viewer, window, origin, "after-lost")
    lost = []
    try:
        served = origin.tree()
        lost = [each for each in session_ids(served) if find_session(served, each)["name"] == "lost-new"]
        assert len(lost) <= 1, lost
        assert find_session(served, sid)["name"] in (name, "lost-name")
        tree = _tree(viewer, window)
        assert "operations" not in tree["tree"]["remote"], tree["tree"]["remote"]
    finally:
        _close_on_origin(origin, probe, *lost)
        assert_ok(origin.request("session.rename", sid, name=name))
    wait_until(lambda: find_session(_tree(viewer, window), sid)["name"] == name)

