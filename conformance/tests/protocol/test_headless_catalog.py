"""The catalog a headless origin serves (upstream's agterm-server, `ngterm-headless` here): `catalog.subscribe` keeps
its connection as a stream of the snapshot and then each change, a structural command sent with an operation id runs
once and answers a receipt, and `catalog.op` reads those receipts and fences an id it has not seen. An app with
windows refuses all of it (test_remote_window.py)."""

import os
import signal
import time

import pytest
from agterm_conformance.client import NoResponse
from agterm_conformance.match import assert_error, assert_ok, find_session
from agterm_conformance.impls import base
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.since("0.42.0"),
    pytest.mark.requires("posix-command-line", reason="the remote origin's sshd is POSIX only"),
    pytest.mark.requires("live-any-shell", reason="ngterm-headless runs every session Live, in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]


def _frame(stream, kind, timeout=30):
    """The next catalog frame of `kind`; the server's pings are acked on the way."""
    deadline = time.monotonic() + timeout
    seen = []
    while (remaining := deadline - time.monotonic()) > 0:
        frame = stream.read(remaining)
        if frame.get("kind") == kind:
            return frame
        if frame.get("kind") == "ping":
            stream.send({"kind": "ack"})
        seen.append(frame.get("kind"))
    pytest.fail(f"no {kind} frame within {timeout}s; skipped {seen}")


def _entry(snapshot, sid):
    for workspace in snapshot["workspaces"]:
        for session in workspace["sessions"]:
            if session["id"] == sid:
                return session
    pytest.fail(f"{sid} is not in the catalog")


def _epoch(origin):
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        return _frame(stream, "hello")["hello"]["epoch"]


@pytest.mark.covers("catalog.subscribe", "session.new", "session.rename", streams=("catalog.subscribe",))
def test_a_subscriber_gets_the_snapshot_then_each_change(remote_origin):
    """Upstream's `aStreamOverTheSocketGetsTheSnapshotThenADelta`: the hello answers with the epoch and the versions,
    the snapshot carries the catalog's id and every session with its live pane, and each change after it is a delta
    one revision on. A session's `name` is its resolved label and `customName` its manual name, `""` without one
    (#993)."""
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new"))["id"]
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        hello = _frame(stream, "hello")["hello"]
        version = assert_ok(origin.request("version"))
        assert hello["version"] == 1 and hello["versions"] == version["versions"], hello
        snapshot = _frame(stream, "snapshot")["snapshot"]
        assert snapshot["epoch"] == hello["epoch"], snapshot
        assert isinstance(snapshot["catalogID"], str) and len(snapshot["catalogID"]) == 36, snapshot
        entry = _entry(snapshot, sid)
        node = find_session(origin.tree(), sid)
        assert entry["customName"] == "" and entry["name"] == node["name"], entry
        assert entry["lifecycle"] == "live" and len(entry["panes"]) == 1, entry
        assert entry["primary"] == entry["panes"][0]["identity"], entry

        assert_ok(origin.request("session.rename", sid, name="build"))
        delta = _frame(stream, "session.changed")
        assert delta["epoch"] == snapshot["epoch"] and delta["rev"] == snapshot["rev"] + 1, delta
        assert delta["session"]["name"] == "build" and delta["session"]["customName"] == "build", delta
    assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("catalog.subscribe", streams=("catalog.subscribe",))
def test_a_stream_that_does_not_open_with_a_hello_is_closed(remote_origin):
    origin = Ctl(remote_origin.instance)
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "ping"})
        # well inside the server's 10 s hello deadline, so only the refused first frame can close it in time
        with pytest.raises(NoResponse):
            stream.read(3)


@pytest.mark.covers("catalog.op", "session.new", "field:result.receipt")
def test_an_operation_runs_once_and_answers_its_receipt(remote_origin):
    """Upstream's `ServerActionsReceiptsTests`: an applied receipt at the revision the change reached, the same receipt
    for a repeat that runs nothing, a refusal for another epoch, and `catalog.op` answering what the table holds."""
    origin = Ctl(remote_origin.instance)
    epoch = _epoch(origin)
    created = assert_ok(origin.request("session.new", opID="op-new", epoch=epoch))
    receipt = created["receipt"]
    assert receipt["opID"] == "op-new" and receipt["epoch"] == epoch, receipt
    assert receipt["outcome"] == "applied" and receipt["rev"] >= 1, receipt
    count = len(origin.tree()["tree"]["workspaces"][0]["sessions"])
    repeated = assert_ok(origin.request("session.new", opID="op-new", epoch=epoch))
    assert repeated == {"receipt": receipt}, repeated
    assert len(origin.tree()["tree"]["workspaces"][0]["sessions"]) == count
    assert assert_ok(origin.request("catalog.op", "op-new")) == {"receipt": receipt}

    stale = origin.request("session.new", opID="op-stale", epoch="an-old-epoch")
    assert_error(stale, f"the catalog is at epoch {epoch}; take its snapshot and send the command again")
    assert stale.result["receipt"]["outcome"] == "refused", stale.raw

    status = origin.request("session.status", created["id"], status="active", opID="op-status", epoch=epoch)
    assert_error(status, "session.status takes no operation id")
    assert_ok(origin.request("session.close", created["id"]))


@pytest.mark.covers("catalog.op", "session.new", "session.split", "field:result.receipt")
def test_a_lookup_fences_an_unseen_id_and_a_changed_target_is_refused(remote_origin):
    """Upstream's `aLookupFencesAnUnseenIdSoItsLateArrivalIsRefused` and
    `aPanePinnedCommandIsRefusedOnceThePanesChanged`."""
    origin = Ctl(remote_origin.instance)
    epoch = _epoch(origin)
    fence = assert_ok(origin.request("catalog.op", "op-late"))
    assert fence == {"receipt": {"opID": "op-late", "epoch": epoch, "outcome": "fenced"}}, fence
    late = origin.request("session.new", opID="op-late", epoch=epoch)
    assert_error(late, "operation op-late was fenced by a lookup and will not run")

    sid = assert_ok(origin.request("session.new"))["id"]
    stale = origin.request(
        "session.split", sid, mode="on", opID="op-pinned", epoch=epoch,
        panes=["00000000-0000-4000-8000-000000000000"],
    )
    assert_error(stale, "the session's panes changed; take the catalog again")
    assert stale.result["receipt"]["outcome"] == "refused", stale.raw
    assert find_session(origin.tree(), sid).get("hasSplit") is not True
    assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("catalog.subscribe", "session.new", "session.split", "session.companion.new",
                    "field:tree.session.ended", streams=("catalog.subscribe",))
def test_a_session_whose_live_session_is_lost_stays_ended(remote_origin):
    """Upstream's `aLostLastPaneLeavesTheSessionEnded` and `anEndedSessionIsSkippedAndCannotGrowASplit`: a session
    whose last pane's live session is lost, not ended by its shell, stays in the tree and the catalog marked ended with
    no pane, refuses a split, and leaves with its close. It also refuses a companion, as the `!parent.ended` guard in
    upstream's `createCompanion` does. Here the instance's session server dies with its sessions."""
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new"))["id"]
    instance = remote_origin.instance
    deadline = time.monotonic() + 30
    while not (servers := base.session_servers(instance.live_dir, instance.vocabulary)) and time.monotonic() < deadline:
        time.sleep(0.2)
    assert servers, "no session server runs the origin's sessions"
    for pid in servers:
        # Windows has no SIGKILL: os.kill there ends the process with TerminateProcess.
        os.kill(pid, getattr(signal, "SIGKILL", signal.SIGTERM))
    deadline = time.monotonic() + 30
    while not find_session(origin.tree(), sid).get("ended") and time.monotonic() < deadline:
        time.sleep(0.2)
    assert find_session(origin.tree(), sid).get("ended") is True
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        entry = _entry(_frame(stream, "snapshot")["snapshot"], sid)
        assert entry["lifecycle"] == "ended" and entry["panes"] == [], entry
    assert_error(origin.request("session.split", sid, mode="on"), "session has ended")
    assert_error(origin.request("session.companion.new", sid, noSelect=True), "session has ended")
    assert_ok(origin.request("session.close", sid))
