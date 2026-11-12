"""A relaunch attaches again the rows a viewer had attached from a Mac running upstream agterm (`agterm-peer`, a
Rust-only extension): a row from a destination pinned to agterm and one whose listing found agterm both come back
attached to the same daemons through the Mac's own zmx, with the presentation stream reopened by agterm's CLI. The Mac
is a loopback machine answering as agterm v0.41.1 does (`agterm_conformance.fake_agterm`)."""

import re

import pytest
from agterm_conformance.agterm_mac import AgtermWorld
from agterm_conformance.match import find_session, sessions
from agterm_conformance.plugin import Cli
from agterm_conformance.wait import wait_text, wait_until

pytestmark = [
    pytest.mark.requires(
        "agterm-peer",
        reason="attaching an upstream agterm Mac is the Rust build's extension; agterm attaches only agterm",
    ),
    # the loopback machine's sshd is POSIX only; Windows lacks this claim, so `--must-pass agterm-peer` excuses the skip
    # there and holds the Windows viewer to test_agterm_peer_windows.py instead
    pytest.mark.requires(
        "posix-command-line",
        reason="a Windows viewer runs a remote pane's attach through viewer-run, not as a /bin/sh line",
    ),
]


@pytest.fixture
def world(tmp_path):
    world = AgtermWorld(tmp_path)
    try:
        yield world
    finally:
        world.stop()


def _attach(cli, host, session_id):
    result = cli("live", "attach", host, session_id)
    assert result.returncode == 0, (result.stdout, result.stderr)
    return result.stdout.strip()


def _remote_rows(ctl):
    return {s["remoteHost"]: s for s in sessions(ctl.tree()) if s.get("remoteHost") is not None}


def _attaches(mac, daemon):
    return [call for call in mac.calls("zmx") if call["argv"][:2] == ["attach", daemon]]


def _presents(mac, session_id):
    return [call for call in mac.calls("agtermctl") if call["argv"] == ["zmx", "present", session_id]]


def test_a_relaunch_attaches_pinned_and_detected_agterm_rows_again(adapter, launch, world):
    studio = world.agterm_mac("studio")
    pinned = studio.add_session("pinned")
    detected = studio.add_session("detected")
    viewer = launch(path_prefix=[world.client.bin], settings={"restoreMode": "live"})
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    for host, session in (("agterm:studio", pinned), ("studio", detected)):
        local = _attach(cli, host, session["id"])
        viewer.wait_mounted(local, "left")
        wait_text(viewer, local, re.escape(f"attached {session['panes'][0]['daemon']}"), timeout=30)
        wait_until(lambda s=session: _presents(studio, s["id"]))
    assert set(_remote_rows(viewer)) == {"agterm:studio", "studio"}

    viewer = launch.relaunch(viewer)

    rows = wait_until(lambda: (r := _remote_rows(viewer)) and set(r) == {"agterm:studio", "studio"} and r)
    assert {rows["agterm:studio"]["name"], rows["studio"]["name"]} == {"pinned", "detected"}
    for host, session in (("agterm:studio", pinned), ("studio", detected)):
        daemon = session["panes"][0]["daemon"]
        wait_until(lambda d=daemon: len(_attaches(studio, d)) == 2)
        wait_until(lambda s=session: len(_presents(studio, s["id"])) == 2)
        local = rows[host]["id"]
        viewer.wait_mounted(local, "left")
        wait_text(viewer, local, re.escape(f"attached {daemon}"), timeout=30)
        assert find_session(viewer.tree(), local)["remoteHost"] == host
    # every relaunched line still went to agterm: ngterm's CLI was never asked for
    assert all(call["ngterm"] == [] for call in studio.calls())
