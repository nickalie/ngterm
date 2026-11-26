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
from agterm_conformance.wait import wait_until

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


def _lose_the_live_sessions(instance, origin, sid):
    """Kills the instance's session server once `sid`'s pane runs on it, so the pane cannot start a new one, and waits
    until the session is ended."""
    daemon = _catalog_entry(origin, sid)["panes"][0]["daemon"]
    wait_until(lambda: daemon in _live_daemons(origin), timeout=30)
    servers = wait_until(lambda: base.session_servers(instance.live_dir, instance.vocabulary), timeout=30)
    for pid in servers:
        # Windows has no SIGKILL: os.kill there ends the process with TerminateProcess.
        os.kill(pid, getattr(signal, "SIGKILL", signal.SIGTERM))
    wait_until(lambda: find_session(origin.tree(), sid).get("ended") is True, timeout=30)


@pytest.mark.covers("catalog.subscribe", "session.new", "session.split", "session.companion.new",
                    "field:tree.session.ended", streams=("catalog.subscribe",))
def test_a_session_whose_live_session_is_lost_stays_ended(remote_origin):
    """Upstream's `aLostLastPaneLeavesTheSessionEnded` and `anEndedSessionIsSkippedAndCannotGrowASplit`: a session
    whose last pane's live session is lost, not ended by its shell, stays in the tree and the catalog marked ended with
    no pane, refuses a split, and leaves with its close. It also refuses a companion, as the `!parent.ended` guard in
    upstream's `createCompanion` does. Here the instance's session server dies with its sessions."""
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new"))["id"]
    _lose_the_live_sessions(remote_origin.instance, origin, sid)
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        entry = _entry(_frame(stream, "snapshot")["snapshot"], sid)
        assert entry["lifecycle"] == "ended" and entry["panes"] == [], entry
    assert_error(origin.request("session.split", sid, mode="on"), "session has ended")
    assert_error(origin.request("session.companion.new", sid, noSelect=True), "session has ended")
    assert_ok(origin.request("session.close", sid))


def _live_daemons(origin):
    vocabulary = origin.vocabulary
    rows = assert_ok(origin.request(vocabulary.command("live.list")))[vocabulary.result_key]["entries"]
    return {row["daemon"]: row for row in rows}


def _catalog_entry(origin, sid, page_rows=False):
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1, "pageRows": page_rows}})
        return _entry(_frame(stream, "snapshot")["snapshot"], sid)


def _scratch_of(origin, sid, page_rows=False):
    return _catalog_entry(origin, sid, page_rows).get("scratch")


@pytest.mark.covers("catalog.subscribe", "session.new", "session.scratch", "session.close", "live.list",
                    streams=("catalog.subscribe",))
def test_the_scratch_is_a_live_session_of_its_own_that_a_hide_keeps_and_the_close_ends(remote_origin):
    """Upstream's `onOffOnShowsTheSameDaemon`, `onWithACommandReplacesTheScratch` and
    `closingTheSessionEndsItsScratch`: the server's scratch runs in a live session of its own name shape, which the
    catalog lists as the session's scratch. A hide keeps it, a `--command` replaces it, and the session's close ends
    it."""
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new"))["id"]
    assert _scratch_of(origin, sid) is None

    assert_ok(origin.request("session.scratch", sid, mode="on"))
    entry = _catalog_entry(origin, sid)
    scratch = entry.get("scratch")
    assert scratch is not None and scratch["shown"] is True, entry
    daemon = scratch["pane"]["daemon"]
    assert "scratch-" in daemon and daemon not in {pane["daemon"] for pane in entry["panes"]}, entry
    assert wait_until(lambda: _live_daemons(origin).get(daemon), timeout=30)["observation"] == "running"

    assert_ok(origin.request("session.scratch", sid, mode="off"))
    hidden = _scratch_of(origin, sid)
    assert hidden["pane"]["daemon"] == daemon and hidden["shown"] is False, hidden
    assert daemon in _live_daemons(origin)

    assert_ok(origin.request("session.scratch", sid, mode="on", command="exec cat"))
    replaced = _scratch_of(origin, sid)["pane"]["daemon"]
    assert replaced != daemon
    wait_until(lambda: daemon not in (listed := _live_daemons(origin)) and replaced in listed, timeout=30)

    assert_ok(origin.request("session.close", sid))
    wait_until(lambda: replaced not in _live_daemons(origin), timeout=30)


def _assert_scratch_daemon_name(origin, daemon, identity):
    """Upstream's `scratchDaemonName`: the vocabulary's prefix, `scratch-`, then the identity's 32 lowercase hex
    digits."""
    digits = identity.replace("-", "").lower()
    assert len(digits) == 32 and daemon == f"{origin.vocabulary.daemon_prefix}scratch-{digits}", daemon


@pytest.mark.covers("catalog.subscribe", "session.new", "session.scratch", "session.companion.page",
                    "field:result.receipt", streams=("catalog.subscribe",))
def test_a_stale_scratch_identity_is_refused_and_a_page_has_no_scratch(remote_origin, tmp_path):
    """Upstream's `aStaleExpectedScratchIsRefusedAndLeavesTheReplacementRunning` and
    `anExpectedScratchThatMatchesIsServedWithAReceipt`: an operation decided against another scratch, or against none
    while one runs, is refused with a receipt and changes nothing; one that names the running scratch is applied. A page
    session has no terminal for a scratch."""
    origin = Ctl(remote_origin.instance)
    epoch = _epoch(origin)
    sid = assert_ok(origin.request("session.new"))["id"]
    assert_ok(origin.request("session.scratch", sid, mode="on"))
    scratch = _scratch_of(origin, sid)
    identity, daemon = scratch["pane"]["identity"], scratch["pane"]["daemon"]
    _assert_scratch_daemon_name(origin, daemon, identity)

    for op, expected in (("op-scratch-other", "00000000-0000-4000-8000-000000000000"), ("op-scratch-none", "none")):
        stale = origin.request("session.scratch", sid, mode="on", command="exec cat", opID=op, epoch=epoch,
                               scratch=expected)
        assert_error(stale, "the session's scratch changed; take the catalog again")
        assert stale.result["receipt"]["outcome"] == "refused", stale.raw
    assert _scratch_of(origin, sid) == scratch

    served = assert_ok(origin.request("session.scratch", sid, mode="off", opID="op-scratch-same", epoch=epoch,
                                      scratch=identity))
    assert served["receipt"]["outcome"] == "applied", served
    hidden = _scratch_of(origin, sid)
    assert hidden["pane"] == scratch["pane"] and hidden["shown"] is False, hidden

    page_file = tmp_path / "page.html"
    page_file.write_text("<!doctype html><title>Page</title><p>page</p>")
    page = assert_ok(origin.request("session.companion.page", sid, html=str(page_file)))["id"]
    assert_error(origin.request("session.scratch", page, mode="on"), "session.scratch: not available on a page session")
    refused = origin.request("session.scratch", page, mode="on", opID="op-scratch-page", epoch=epoch, scratch="none")
    assert_error(refused, "session.scratch: not available on a page session")
    assert refused.result["receipt"]["outcome"] == "refused", refused.raw
    assert _catalog_entry(origin, page, page_rows=True).get("page") and _scratch_of(origin, page, True) is None

    assert_ok(origin.request("session.close", sid))
    wait_until(lambda: daemon not in _live_daemons(origin), timeout=30)


@pytest.mark.covers("catalog.subscribe", "session.new", "session.scratch", "session.close", "live.list",
                    "field:tree.session.ended", streams=("catalog.subscribe",))
def test_an_ended_session_keeps_its_scratch_until_its_close(remote_origin):
    """Upstream's plan for an ended session: its last pane is lost, it stays in the catalog ended, and its scratch
    runs on there in a live session of its own until the session's close. Here the instance's session server dies
    with the session's live sessions, so the scratch starts after the session ended."""
    origin = Ctl(remote_origin.instance)
    sid = assert_ok(origin.request("session.new"))["id"]
    _lose_the_live_sessions(remote_origin.instance, origin, sid)

    assert_ok(origin.request("session.scratch", sid, mode="on"))
    entry = _catalog_entry(origin, sid)
    assert entry["lifecycle"] == "ended" and entry["panes"] == [], entry
    scratch = entry.get("scratch")
    assert scratch is not None and scratch["shown"] is True, entry
    daemon = scratch["pane"]["daemon"]
    _assert_scratch_daemon_name(origin, daemon, scratch["pane"]["identity"])
    assert wait_until(lambda: _live_daemons(origin).get(daemon), timeout=30)["observation"] == "running"

    assert_ok(origin.request("session.scratch", sid, mode="off"))
    entry = _catalog_entry(origin, sid)
    assert entry["lifecycle"] == "ended" and entry["scratch"]["pane"]["daemon"] == daemon, entry
    assert entry["scratch"]["shown"] is False and daemon in _live_daemons(origin), entry

    assert_ok(origin.request("session.close", sid))
    wait_until(lambda: daemon not in _live_daemons(origin), timeout=30)
