"""A remote window's ssh traffic shares ControlMaster connections (upstream's `SshConnectionPool`), against a real
`ngterm-headless` (`remote_origin`, behind the harness's own sshd on loopback): its version probe, catalog stream,
actions, rows' streams and pane attaches ride one master, whose control socket is under the pool root, and the window's
close ends that master. An action whose master cannot come up is answered "no connection to HOST" and never sent; the
next action once the host answers starts a new master and runs.

The test counts the window's ssh in the private ssh client's call log (`SshClient.calls`) and the TCP connections open
through the proxy in front of the origin's sshd. Windows' OpenSSH shares no connections, so the cases are POSIX only.
The remote window is upstream's v0.42.0 feature; only the Rust build claims `live-any-shell`, so the reference skips
them.
"""

import os
import stat
import sys
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Cli, Ctl
from agterm_conformance.remote_origin import ORIGIN_HOST
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.skipif(sys.platform == "win32", reason="Windows' OpenSSH shares no connections"),
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-any-shell", reason="ngterm-headless runs every session Live, in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]

# The window's first snapshot comes after an ssh `version` probe and an ssh catalog stream.
CONNECT = 60.0
# A master that nothing ends lives on for ControlPersist (60 s) after its last channel: the close must end it sooner.
CLOSED = 30.0
# A refused master start: its ConnectTimeout (10 s) or the pool's command deadline (15 s), with room for a loaded host.
NO_MASTER = 60.0
CONTROL_PATH = "ControlPath="


def _remote(viewer, window):
    for node in assert_ok(viewer.request("window.list"))["windows"]:
        if node["id"] == window:
            return node.get("remote") or {}
    return {}


def _tree(viewer, window):
    return assert_ok(viewer.request("tree", window=window))


def _control_path(argv):
    """The ControlPath an ssh invocation names, or None."""
    for word in argv:
        if word.startswith(CONTROL_PATH):
            return word[len(CONTROL_PATH):]
    return None


def _pool_root():
    return Path(f"/tmp/ngterm-ssh-{os.getuid()}")


def _is_socket(path):
    try:
        return stat.S_ISSOCK(os.stat(path).st_mode)
    except FileNotFoundError:
        return False


def _cli(adapter, viewer):
    return Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)


def _pane_attach(tree, sid):
    """The ssh argv row `sid`'s pane runs to attach the server's pane, once its presentation is connected, or None."""
    session = find_session(tree, sid)
    foreground = session.get("foreground") or []
    attached = session.get("presentation", {}).get("state") == "connected" and _control_path(foreground)
    return foreground if attached else None


def _close_on_origin(origin, *sids):
    for sid in sids:
        if sid and sid in session_ids(origin.tree()):
            assert_ok(origin.request("session.close", sid))


@pytest.fixture
def window(remote_origin, remote_viewer):
    """The viewer's remote window of the origin, opened from no connection at all and connected, with a row the
    origin made before the open; the call log starts at the open."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    wait_until(lambda: remote_origin.proxy.connections() == 0, timeout=CLOSED)
    anchor = assert_ok(origin.request("session.new", name="anchor"))["id"]
    remote_origin.client.calls.unlink(missing_ok=True)
    opened = assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"]
    try:
        wait_until(lambda: _remote(viewer, opened).get("state") == "connected", timeout=CONNECT)
        wait_until(lambda: anchor in session_ids(_tree(viewer, opened)), timeout=CONNECT)
        yield opened, anchor
    finally:
        if _remote(viewer, opened):
            viewer.request("window.close", opened)
        _close_on_origin(origin, anchor)


@pytest.mark.covers("window.remote.open", "window.close", "session.new", "session.rename",
                    "field:tree.session.presentation")
def test_actions_and_pane_attaches_share_one_master_and_the_close_ends_it(
    remote_origin, remote_viewer, window
):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    opened, anchor = window
    made = None
    try:
        made = assert_ok(viewer.request("session.new", window=opened, name="pooled", noSelect=True))["id"]
        wait_until(lambda: made in session_ids(_tree(viewer, opened)) and made in session_ids(origin.tree()),
                   timeout=CONNECT)
        assert_ok(viewer.request("session.rename", made, name="pooled-renamed"))
        wait_until(lambda: find_session(origin.tree(), made)["name"] == "pooled-renamed", timeout=CONNECT)
        attaches = {
            sid: _control_path(wait_until(lambda sid=sid: _pane_attach(_tree(viewer, opened), sid), timeout=CONNECT))
            for sid in (anchor, made)
        }

        calls = remote_origin.client.invocations()
        paths = {_control_path(argv) for argv in calls}
        assert len(paths) == 1 and None not in paths, calls
        path = paths.pop()
        assert Path(path).parent == _pool_root(), path
        assert attaches == {anchor: path, made: path}, attaches
        assert _is_socket(path), f"no master socket at {path}"
        assert len(calls) >= 6, f"the probe, the stream, two actions and two panes ride the master: {calls}"
        assert remote_origin.proxy.connections() == 1, "every channel rides the one master's connection"

        assert_ok(viewer.request("window.close", opened))
        wait_until(lambda: remote_origin.proxy.connections() == 0, timeout=CLOSED)
        assert not _is_socket(path), f"the master socket {path} outlived its window"
        assert {anchor, made} <= set(session_ids(origin.tree())), "closing the window ends nothing on the server"
    finally:
        _close_on_origin(origin, made)


@pytest.mark.covers("cli:session new", "session.new")
def test_an_action_whose_master_cannot_come_up_is_refused_and_never_sent(
    adapter, remote_origin, remote_viewer, window
):
    """The master stops taking channels (`ssh -O stop`) while sshd answers no new login: the window stays connected
    over its open channels, and an action answers "no connection to HOST". Once sshd answers, the next action starts a
    new master and runs."""
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    opened, _ = window
    cli = _cli(adapter, viewer)
    path = _control_path(remote_origin.client.invocations()[-1])
    assert path and _is_socket(path), remote_origin.client.invocations()
    made = None
    try:
        with remote_origin.machine.unanswered():
            stopped = remote_origin.client.stop_master(ORIGIN_HOST, path)
            assert stopped.returncode == 0, stopped
            wait_until(lambda: not _is_socket(path))

            refused = cli("session", "new", "--window", opened, "--name", "never-sent", "--no-select",
                          timeout=NO_MASTER)

            assert (refused.returncode, refused.stdout, refused.stderr) == (
                1, "", f"error: no connection to {ORIGIN_HOST}\n"
            ), refused
            assert _remote(viewer, opened).get("state") == "connected"
            assert "operations" not in _tree(viewer, opened)["tree"]["remote"]

        made = assert_ok(viewer.request("session.new", window=opened, name="after-master", noSelect=True))["id"]
        wait_until(lambda: made in session_ids(origin.tree()), timeout=CONNECT)
        assert _is_socket(path), "the next action starts a new master at the window's path"
        served = origin.tree()
        assert "never-sent" not in [find_session(served, sid)["name"] for sid in session_ids(served)]
    finally:
        _close_on_origin(origin, made)
