"""A remote window whose server this machine never reaches, the cases a Windows viewer has without an origin: the
harness's remote origin needs an unprivileged sshd, which Windows lacks, so a POSIX host runs the remote window against
a real `ngterm-headless` instead (test_remote_window_mirror.py, test_remote_window_refusal.py).

The viewer's `ssh` is a stand-in (`agterm_conformance.ssh_stand_in`) that reaches no machine. A host that never answers
shows `disconnected` and is tried again; a host whose `version` names an app with windows, or a catalog or presentation
version this build does not speak, is refused for good; a catalog stream that ends shows `disconnected`. While the
window is not connected, a command that makes or changes its rows is refused with the host's name and is never sent
later, a command that makes a local shell or row is refused as in any remote window, and nothing is left to dismiss.
A closed remote window starts no more ssh.

The remote window is upstream's v0.42.0 feature, above the build's parity level; the Rust build's Windows viewer runs
these cases (`cmd-command-line`), and the reference skips them."""

import json
import time

import pytest
from agterm_conformance.match import assert_error, assert_ok, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.ssh_stand_in import SshStandIn
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.requires(
    "cmd-command-line",
    reason="a Windows viewer has no remote origin in the suite: POSIX hosts run the remote window against a real one",
)

# A probe that fails, then the retries after 1 and 2 seconds, on a loaded host.
SETTLE = 60.0
# Longer than the first three retries (1 + 2 + 5 seconds) a window that still tried would make.
QUIET = 10.0
# The catalog and presentation versions this build speaks.
CATALOG_VERSION = 1
PRESENTATION_VERSION = 1
NOT_AVAILABLE = "{cmd} is not available in a remote window"
# The catalog stream's remote command quotes each of its words.
CATALOG = "subscribe"
VERSION = "version"


@pytest.fixture(scope="module")
def stand_in(tmp_path_factory):
    return SshStandIn(tmp_path_factory.mktemp("unreached"))


@pytest.fixture(scope="module")
def viewer(adapter, stand_in):
    instance = adapter.launch(path_prefix=[stand_in.bin])
    try:
        yield Ctl(instance)
    finally:
        adapter.stop(instance)


@pytest.fixture
def opened(viewer, stand_in):
    """`open(host)` opens a remote window of `host` and gives its id; each window opened is closed after the test, and
    the stand-in fails every call again."""
    windows = []

    def open_window(host):
        window = assert_ok(viewer.request("window.remote.open", host=host))["id"]
        windows.append(window)
        return window

    yield open_window
    for window in windows:
        if _open(viewer, window):
            assert_ok(viewer.request("window.close", window))
    stand_in.answer()


def _node(viewer, window):
    for node in assert_ok(viewer.request("window.list"))["windows"]:
        if node["id"] == window:
            return node
    pytest.fail(f"window {window} is not in window.list")


def _open(viewer, window):
    return _node(viewer, window)["open"]


def _remote(viewer, window):
    return _node(viewer, window).get("remote")


def _settled(viewer, window, state):
    return wait_until(lambda: (node := _remote(viewer, window)).get("state") == state and node, timeout=SETTLE)


def _calls(stand_in, host, words):
    """The stand-in's calls to `host` whose remote command holds `words`."""
    return [call for call in stand_in.invocations() if host in call[:-1] and words in call[-1]]


def _version_reply(viewer, **extra):
    """A `version` answer of one line: the viewer's own, an app with windows, with `extra` over it."""
    result = assert_ok(viewer.request("version"))
    return json.dumps({"ok": True, "result": {**result, **extra}}) + "\n"


def _headless(viewer, catalog=CATALOG_VERSION, protocol=PRESENTATION_VERSION):
    versions = {"server": "0.0.1", "protocol": protocol, "catalog": catalog}
    return _version_reply(viewer, headless=True, versions=versions)


@pytest.mark.covers("window.remote.open", "window.list", "tree", "field:window.remote", "field:tree.remote")
def test_a_host_that_never_answers_shows_disconnected_and_is_tried_again(viewer, stand_in, opened):
    window = opened("nowhere")
    remote = _settled(viewer, window, "disconnected")
    assert remote == {"host": "nowhere", "state": "disconnected", "reason": "nowhere did not answer"}, remote
    tree = assert_ok(viewer.request("tree", window=window))
    assert tree["tree"]["remote"] == remote, tree["tree"]
    assert session_ids(tree) == []
    wait_until(lambda: len(_calls(stand_in, "nowhere", VERSION)) >= 2, timeout=SETTLE)
    assert assert_ok(viewer.request("window.remote.open", host="nowhere"))["id"] == window


@pytest.mark.covers("workspace.new", "session.new", "window.remote.dismiss", "field:window.remote")
def test_a_row_command_while_not_connected_is_refused_and_never_sent(viewer, stand_in, opened):
    window = opened("offline")
    _settled(viewer, window, "disconnected")
    refused = "offline is not connected"
    assert_error(viewer.request("workspace.new", window=window), refused)
    assert_error(viewer.request("workspace.new", name="made", window=window), refused)
    assert_ok(viewer.request("window.select", window))
    assert_error(viewer.request("workspace.new", name="selected"), refused)
    probes = len(_calls(stand_in, "offline", VERSION))
    wait_until(lambda: len(_calls(stand_in, "offline", VERSION)) > probes, timeout=SETTLE)
    assert _calls(stand_in, "offline", "workspace") == [], stand_in.invocations()
    remote = _remote(viewer, window)
    assert "operations" not in remote, remote
    assert assert_ok(viewer.request("window.remote.dismiss", window))["text"] == "0 dismissed"


@pytest.mark.covers("session.new", "workspace.new")
@pytest.mark.parametrize(
    ("cmd", "args", "error"),
    [
        pytest.param("session.new", {"wait": True}, "--wait is not available in a remote window", id="wait"),
        pytest.param(
            "session.new", {"workspaceName": "w"}, "--workspace-name is not available in a remote window",
            id="workspace-name",
        ),
        pytest.param(
            "session.new", {"createWorkspace": True}, "--create-workspace is not available in a remote window",
            id="create-workspace",
        ),
        pytest.param("session.new", {"tint": "red"}, "--tint is not available in a remote window", id="tint"),
        pytest.param(
            "session.new", {"after": "a", "before": "b"}, "use either --after or --before, not both", id="after-before"
        ),
        pytest.param(
            "session.new", {"opID": "op-1", "epoch": "E"}, "operation receipts are served only by ngterm-headless",
            id="operation-id",
        ),
        pytest.param(
            "workspace.new", {"collapsed": True}, "--collapsed is not available in a remote window", id="collapsed"
        ),
        pytest.param("workspace.new", {"tint": "red"}, "--tint is not available in a remote window", id="ws-tint"),
    ],
)
def test_a_field_the_server_cannot_be_asked_for_is_refused_by_name(viewer, opened, cmd, args, error):
    window = opened("fields")
    assert_error(viewer.request(cmd, window=window, **args), error)
    assert session_ids(assert_ok(viewer.request("tree", window=window))) == []


@pytest.mark.covers("live.attach", "recent.open")
def test_a_command_that_makes_a_local_row_is_refused(viewer, opened):
    window = opened("local-only")
    for cmd, args in [("live.attach", {"name": "x"}), ("recent.open", {})]:
        assert_error(viewer.request(cmd, window=window, **args), NOT_AVAILABLE.format(cmd=cmd))
    assert session_ids(assert_ok(viewer.request("tree", window=window))) == []


@pytest.mark.covers("window.remote.open", "window.list", "field:window.remote")
def test_an_app_with_windows_is_refused_for_good(viewer, stand_in, opened):
    stand_in.answer((VERSION, _version_reply(viewer), 0))
    version = assert_ok(viewer.request("version"))["app"]["version"]
    window = opened("app-host")
    remote = _settled(viewer, window, "refused")
    reason = (
        f"app-host runs ngterm {version} without a catalog, an app with windows rather than ngterm-headless; "
        "a remote window needs ngterm-headless"
    )
    assert remote == {"host": "app-host", "state": "refused", "reason": reason}, remote
    launches = len(_calls(stand_in, "app-host", ""))
    time.sleep(QUIET)
    assert len(_calls(stand_in, "app-host", "")) == launches, stand_in.invocations()
    assert_error(viewer.request("workspace.new", window=window), "app-host is not connected")


@pytest.mark.covers("window.remote.open", "window.list", "field:window.remote")
@pytest.mark.parametrize(
    ("versions", "reason"),
    [
        pytest.param(
            {"catalog": 99}, f"old-host speaks catalog 99, this Mac speaks catalog {CATALOG_VERSION}", id="catalog"
        ),
        pytest.param(
            {"protocol": 0},
            f"old-host speaks presentation 0, this Mac speaks presentation {PRESENTATION_VERSION}",
            id="presentation",
        ),
    ],
)
def test_a_server_of_another_version_is_refused(viewer, stand_in, opened, versions, reason):
    stand_in.answer((VERSION, _headless(viewer, **versions), 0))
    window = opened("old-host")
    remote = _settled(viewer, window, "refused")
    assert remote == {"host": "old-host", "state": "refused", "reason": reason}, remote
    assert _calls(stand_in, "old-host", CATALOG) == [], stand_in.invocations()


@pytest.mark.covers("window.remote.open", "window.list", "field:window.remote")
def test_a_catalog_stream_that_ends_shows_disconnected_and_is_tried_again(viewer, stand_in, opened):
    stand_in.answer((CATALOG, "", 0), (VERSION, _headless(viewer), 0))
    window = opened("ending")
    remote = _settled(viewer, window, "disconnected")
    assert remote["reason"].startswith("the catalog stream from ending ended: "), remote
    wait_until(lambda: len(_calls(stand_in, "ending", CATALOG)) >= 2, timeout=SETTLE)
    assert_error(viewer.request("workspace.new", window=window), "ending is not connected")


@pytest.mark.covers("window.close", "window.remote.open")
def test_a_closed_remote_window_starts_no_more_ssh(viewer, stand_in, opened):
    window = opened("closing")
    wait_until(lambda: len(_calls(stand_in, "closing", VERSION)) >= 2, timeout=SETTLE)
    assert_ok(viewer.request("window.close", window))
    wait_until(lambda: not _open(viewer, window))
    assert _remote(viewer, window) == {"host": "closing"}
    launches = len(_calls(stand_in, "closing", VERSION))
    time.sleep(QUIET)
    assert len(_calls(stand_in, "closing", VERSION)) == launches, stand_in.invocations()
