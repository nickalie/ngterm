"""Detach and reattach in a remote window that mirrors a real `ngterm-headless` (`remote_origin`, behind the harness's
own sshd on loopback): a detached session or workspace leaves the window, keeps running on the server, and is listed
in the window's `remote` node; a reattach brings it back. A detach outlives the window: closing a remote window
detaches it, and the window opened again still hides what it detached. A close by id reaches a detached row on the
server. A parent takes its companions, a row under a detached parent or workspace waits for it, and the window
detaches and reattaches while its server does not answer, as nothing goes to the server.

The remote window is upstream's v0.42.0 feature, and test_remote_window.py keeps its since("0.42.0") refusals. These
cases need a Live origin in any login shell, which only the Rust build claims (`live-any-shell`), so the reference skips
them.
"""

import os
import signal

import pytest
from agterm_conformance.match import assert_error, assert_ok, find_session, session_ids
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


def _remote(viewer, window):
    for node in assert_ok(viewer.request("window.list"))["windows"]:
        if node["id"] == window:
            return node.get("remote") or {}
    pytest.fail(f"window {window} is not in window.list")


def _tree(viewer, window):
    return assert_ok(viewer.request("tree", window=window))


def _workspace_ids(tree):
    return [node["id"] for node in tree["tree"]["workspaces"]]


def _upper(ids):
    return sorted(value.upper() for value in ids)


def _detached(viewer, window, key):
    return _upper(_remote(viewer, window).get(key, []))


@pytest.fixture(scope="module")
def window(remote_origin, remote_viewer):
    """The viewer's remote window of the origin, connected."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    anchor = assert_ok(origin.request("session.new", name="anchor"))["id"]
    opened = assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"]
    wait_until(lambda: _remote(viewer, opened).get("state") == "connected", timeout=CONNECT)
    wait_until(lambda: anchor in session_ids(_tree(viewer, opened)), timeout=CONNECT)
    return opened


@pytest.fixture
def row(remote_origin, remote_viewer, window):
    """A session the origin makes, shown in the window; closed on the origin afterwards."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    sid = assert_ok(origin.request("session.new", name="detachable"))["id"]
    wait_until(lambda: sid in session_ids(_tree(viewer, window)), timeout=CONNECT)
    yield sid
    if sid in session_ids(origin.tree()):
        assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("session.detach", "session.reattach", "field:window.remote")
def test_a_detached_session_leaves_the_window_keeps_running_and_comes_back(remote_origin, remote_viewer, window, row):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer

    detached = assert_ok(viewer.request("session.detach", row, window=window))

    assert detached.get("affected") == 1, detached
    assert row not in session_ids(_tree(viewer, window))
    assert _detached(viewer, window, "detachedSessions") == _upper([row])
    assert row in session_ids(origin.tree()), "the session keeps running on the server"
    assert_error(viewer.request("session.select", row, window=window), f"no such session: {row}")

    reattached = assert_ok(viewer.request("session.reattach", row[:13]))

    assert reattached["id"].lower() == row.lower()
    assert row in session_ids(_tree(viewer, window))
    assert "detachedSessions" not in _remote(viewer, window)
    assert_error(viewer.request("session.reattach", row), f"no such detached session: {row}")


@pytest.mark.covers("workspace.detach", "workspace.reattach", "field:window.remote")
def test_a_detached_workspace_hides_its_rows_and_comes_back(remote_origin, remote_viewer, window):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    wid = assert_ok(origin.request("workspace.new", name="detachable"))["id"]
    sid = assert_ok(origin.request("session.new", workspace=wid, name="inside"))["id"]
    try:
        wait_until(lambda: sid in session_ids(_tree(viewer, window)), timeout=CONNECT)

        assert_ok(viewer.request("workspace.detach", wid, window=window))

        tree = _tree(viewer, window)
        assert wid not in _workspace_ids(tree) and sid not in session_ids(tree), tree
        assert _detached(viewer, window, "detachedWorkspaces") == _upper([wid])
        assert sid in session_ids(origin.tree())
        assert_error(viewer.request("session.reattach", sid), f"no such detached session: {sid}")

        assert_ok(viewer.request("workspace.reattach", wid, window=window))

        tree = _tree(viewer, window)
        assert wid in _workspace_ids(tree) and sid in session_ids(tree), tree
        assert "detachedWorkspaces" not in _remote(viewer, window)
    finally:
        assert_ok(origin.request("workspace.delete", wid))


@pytest.mark.covers("session.detach", "session.reattach", "window.close", "window.remote.open")
def test_a_detach_outlives_the_window_and_a_close_leaves_every_session_running(
    remote_origin, remote_viewer, window, row
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    assert_ok(viewer.request("session.detach", row, window=window))
    served = sorted(session_ids(origin.tree()))

    assert_ok(viewer.request("window.close", window))
    assert sorted(session_ids(origin.tree())) == served, "closing the window ends nothing on the server"
    assert assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"] == window
    wait_until(lambda: _remote(viewer, window).get("state") == "connected", timeout=CONNECT)
    wait_until(lambda: "anchor" in [find_session(_tree(viewer, window), sid)["name"]
                                    for sid in session_ids(_tree(viewer, window))], timeout=CONNECT)

    assert row not in session_ids(_tree(viewer, window)), "the reopened window still hides it"
    assert _detached(viewer, window, "detachedSessions") == _upper([row])
    assert_ok(viewer.request("session.reattach", row))
    wait_until(lambda: row in session_ids(_tree(viewer, window)))


@pytest.mark.covers("session.close", "session.detach")
def test_a_close_by_id_ends_a_detached_session_on_the_server(remote_origin, remote_viewer, window, row):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    assert_ok(viewer.request("session.detach", row, window=window))

    assert_ok(viewer.request("session.close", row))

    wait_until(lambda: row not in session_ids(origin.tree()), timeout=CONNECT)
    wait_until(lambda: "detachedSessions" not in _remote(viewer, window), timeout=CONNECT)


@pytest.mark.covers("cli:session detach", "cli:session reattach", "session.detach", "session.reattach")
def test_the_cli_detaches_and_reattaches_a_remote_row(adapter, remote_viewer, window, row):
    viewer = remote_viewer
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)

    detached = cli("session", "detach", "--target", row, "--window", window, "--json")
    assert (detached.returncode, detached.stderr) == (0, ""), detached
    assert detached.json()["result"]["affected"] == 1
    assert row not in session_ids(_tree(viewer, window))

    reattached = cli("session", "reattach", "--target", row, "--json")
    assert (reattached.returncode, reattached.stderr) == (0, ""), reattached
    assert reattached.json()["result"]["id"].lower() == row.lower()
    assert row in session_ids(_tree(viewer, window))


def _close_on_origin(origin, *sids):
    for sid in sids:
        if sid and sid in session_ids(origin.tree()):
            assert_ok(origin.request("session.close", sid))


def _local_window(viewer):
    windows = [node["id"] for node in assert_ok(viewer.request("window.list"))["windows"] if not node.get("remote")]
    assert windows, "the viewer has no local window"
    return windows[0]


@pytest.mark.covers("session.detach", "session.reattach", "field:window.remote")
def test_a_parent_takes_its_companions_and_a_companion_waits_for_its_parent(remote_origin, remote_viewer, window):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    parent = assert_ok(origin.request("session.new", name="parent"))["id"]
    companion = None
    try:
        companion = assert_ok(origin.request("session.companion.new", parent, name="companion", noSelect=True))["id"]
        assert find_session(origin.tree(), companion).get("parent") == parent
        wait_until(lambda: {parent, companion} <= set(session_ids(_tree(viewer, window))), timeout=CONNECT)

        assert assert_ok(viewer.request("session.detach", parent, window=window)).get("affected") == 1

        shown = session_ids(_tree(viewer, window))
        assert parent not in shown and companion not in shown, shown
        assert _detached(viewer, window, "detachedSessions") == _upper([parent])
        assert {parent, companion} <= set(session_ids(origin.tree())), "both keep running on the server"
        assert_error(viewer.request("session.reattach", companion), f"no such detached session: {companion}")

        assert_ok(viewer.request("session.reattach", parent))
        assert {parent, companion} <= set(session_ids(_tree(viewer, window)))

        both = assert_ok(viewer.request("session.detach", window=window, targets=[companion, parent]))
        assert both.get("affected") == 2, both
        assert _detached(viewer, window, "detachedSessions") == _upper([parent, companion])
        assert_error(viewer.request("session.reattach", companion), "its parent is detached; reattach the parent")

        assert_ok(viewer.request("session.reattach", parent))
        shown = session_ids(_tree(viewer, window))
        assert parent in shown and companion not in shown, shown
        assert_ok(viewer.request("session.reattach", companion))
        assert companion in session_ids(_tree(viewer, window))
        assert "detachedSessions" not in _remote(viewer, window)
    finally:
        _close_on_origin(origin, companion, parent)


@pytest.mark.covers("session.detach", "session.reattach", "workspace.detach", "workspace.reattach")
def test_a_session_detached_in_a_detached_workspace_waits_for_the_workspace(remote_origin, remote_viewer, window):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    wid = assert_ok(origin.request("workspace.new", name="outer"))["id"]
    try:
        sid = assert_ok(origin.request("session.new", workspace=wid, name="inner"))["id"]
        wait_until(lambda: sid in session_ids(_tree(viewer, window)), timeout=CONNECT)
        assert_ok(viewer.request("session.detach", sid, window=window))
        assert_ok(viewer.request("workspace.detach", wid, window=window))

        assert_error(viewer.request("session.reattach", sid), "its workspace is detached; reattach the workspace")

        assert_ok(viewer.request("workspace.reattach", wid[:13]))
        tree = _tree(viewer, window)
        assert wid in _workspace_ids(tree) and sid not in session_ids(tree), tree
        assert_ok(viewer.request("session.reattach", sid))
        assert sid in session_ids(_tree(viewer, window))
    finally:
        assert_ok(origin.request("workspace.delete", wid))


@pytest.mark.covers("session.detach", "session.reattach", "workspace.detach", "workspace.reattach")
def test_detach_and_reattach_refuse_what_they_cannot_take(remote_origin, remote_viewer, window, row):
    viewer = remote_viewer
    local = _local_window(viewer)
    local_session = session_ids(_tree(viewer, local))[0]
    local_workspace = _workspace_ids(_tree(viewer, local))[0]

    assert_error(
        viewer.request("session.detach", local_session, window=local), "session.detach acts only on a remote window"
    )
    assert_error(
        viewer.request("workspace.detach", local_workspace, window=local),
        "workspace.detach acts only on a remote window",
    )
    assert_error(viewer.request("session.detach", window=window, targets=[]),
                 "session.detach requires at least one --target")
    assert_error(
        viewer.request("session.detach", targets=[row, local_session]), "session.detach takes sessions of one window"
    )
    for target in (None, "active"):
        assert_error(viewer.request("session.reattach", target), "session.reattach requires a detached session's id")
        assert_error(
            viewer.request("workspace.reattach", target), "workspace.reattach requires a detached workspace's id"
        )
    assert_error(viewer.request("session.reattach", row), f"no such detached session: {row}")
    assert_error(viewer.request("workspace.reattach", "nothing"), "no such detached workspace: nothing")

    assert row in session_ids(_tree(viewer, window)) and local_session in session_ids(_tree(viewer, local))
    assert "detachedSessions" not in _remote(viewer, window)


@pytest.mark.covers("session.detach", "session.reattach", "field:window.remote")
def test_a_detach_and_a_reattach_work_while_the_server_does_not_answer(remote_origin, remote_viewer, window, row):
    """The server stopped (SIGSTOP) keeps its ssh connection open and says nothing until the stream's silence limit
    shows the window `disconnected`: a detach and a reattach change only this window, and the detach holds once the
    server answers again."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    pid = remote_origin.instance.process.pid
    os.kill(pid, signal.SIGSTOP)
    try:
        wait_until(lambda: _remote(viewer, window).get("state") == "disconnected", timeout=SILENCE)

        assert assert_ok(viewer.request("session.detach", row, window=window)).get("affected") == 1
        assert row not in session_ids(_tree(viewer, window))
        assert_ok(viewer.request("session.reattach", row))
        assert row in session_ids(_tree(viewer, window))
        assert_ok(viewer.request("session.detach", row, window=window))
        assert _detached(viewer, window, "detachedSessions") == _upper([row])
    finally:
        os.kill(pid, signal.SIGCONT)
    wait_until(lambda: _remote(viewer, window).get("state") == "connected", timeout=RECONNECT)

    probe = assert_ok(origin.request("session.new", name="after-resume"))["id"]
    try:
        wait_until(lambda: probe in session_ids(_tree(viewer, window)), timeout=CONNECT)
        assert row not in session_ids(_tree(viewer, window)), "the detach holds once the server answers"
        assert row in session_ids(origin.tree())
        assert_ok(viewer.request("session.reattach", row))
        assert row in session_ids(_tree(viewer, window))
    finally:
        _close_on_origin(origin, probe)
