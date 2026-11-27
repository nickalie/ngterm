"""A remote window of a host that runs an app with windows, not `ngterm-headless` (`remote_origin` of kind `instance`,
behind the harness's own sshd on loopback): the host advertises no catalog, so the window is refused for good with the
build's version refusal and starts no more ssh to it.

Upstream's refusal names a Mac app and agterm-server; this build's names an app with windows and ngterm-headless
(conformance/docs/rust-build.md > Other strings of the build's own). The case needs a Live origin in any login shell,
which only the Rust build claims (`live-any-shell`), so the reference skips it. test_remote_window.py keeps the
since("0.42.0") cases of the feature."""

import time

import pytest
from agterm_conformance.match import assert_ok
from agterm_conformance.plugin import Ctl
from agterm_conformance.remote_origin import ORIGIN_HOST
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires("posix-command-line", reason="the private sshd uses a POSIX shell"),
    pytest.mark.requires("live-any-shell", reason="the origin's sessions must be Live in any login shell"),
    pytest.mark.parametrize("remote_origin", ["instance"], indirect=True),
]

SETTLE = 60.0
# Longer than the first three retries (1 + 2 + 5 seconds) a lost stream would make.
QUIET = 10.0


def _remote(viewer, window):
    for node in assert_ok(viewer.request("window.list"))["windows"]:
        if node["id"] == window:
            return node.get("remote")
    pytest.fail(f"window {window} is not in window.list")


@pytest.mark.covers("window.remote.open", "window.list", "tree", "field:window.remote", "field:tree.remote")
def test_a_host_without_a_catalog_is_refused_for_good(remote_origin, remote_viewer):
    origin, viewer = Ctl(remote_origin.instance), remote_viewer
    version = assert_ok(origin.request("version"))["app"]["version"]
    reason = (
        f"{ORIGIN_HOST} runs ngterm {version} without a catalog, an app with windows rather than ngterm-headless; "
        "a remote window needs ngterm-headless"
    )
    window = assert_ok(viewer.request("window.remote.open", host=ORIGIN_HOST))["id"]
    remote = wait_until(lambda: (node := _remote(viewer, window)).get("state") == "refused" and node, timeout=SETTLE)
    assert remote == {"host": ORIGIN_HOST, "state": "refused", "reason": reason}, remote
    tree = assert_ok(viewer.request("tree", window=window))
    assert tree["tree"]["remote"] == remote, tree["tree"]
    assert [session for workspace in tree["tree"]["workspaces"] for session in workspace["sessions"]] == []

    launches = len(remote_origin.client.invocations())
    time.sleep(QUIET)
    assert len(remote_origin.client.invocations()) == launches, remote_origin.client.invocations()[launches:]
    assert _remote(viewer, window) == remote
