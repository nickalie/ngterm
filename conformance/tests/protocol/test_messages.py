"""Pane mailbox wire contract, including read/ack receipts and body-free events."""

import time
import uuid

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session

pytestmark = pytest.mark.since("0.40.0")


def _pane(ctl, **args):
    sid = assert_ok(ctl.request("session.new", **args))["id"]
    tree = ctl.wait_mounted(sid, "left")
    (pane,) = [p for p in find_session(tree, sid)["surfaces"] if p["kind"] == "left"]
    return sid, pane["paneID"]


def _surface(ctl, sid, pid):
    return next(p for p in find_session(ctl.tree(), sid)["surfaces"] if p.get("paneID") == pid)


@pytest.mark.covers(
    "msg.send", "msg.read", "msg.ack", "field:result.messageID", "field:result.delivered",
    "field:result.rejected", "field:result.messages", "field:result.remaining", "field:result.acked",
)
def test_send_read_ack(ctl):
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    unknown = f"{uuid.uuid4()}:{uuid.uuid4()}"
    try:
        assert_absent(_surface(ctl, sid, pid), "pendingMessages")
        before = time.time()
        receipt = assert_ok(ctl.request("msg.send", body="mail\nsecond line", recipients=[f"{sid}:left", unknown]))
        uuid.UUID(receipt["messageID"])
        assert receipt["delivered"] == [address]
        assert receipt["rejected"] == [{"address": unknown, "reason": "unknown pane"}]
        assert _surface(ctl, sid, pid)["pendingMessages"] == 1
        batch = assert_ok(ctl.request("msg.read", identity=address))
        assert batch["remaining"] == 0
        (message,) = batch["messages"]
        assert_subset(message, {"id": receipt["messageID"], "to": address, "body": "mail\nsecond line"})
        assert_absent(message, "from")
        assert before <= message["ts"] <= time.time()
        assert assert_ok(ctl.request("msg.read", identity=address))["messages"] == batch["messages"]
        assert assert_ok(ctl.request("msg.ack", identity=address, ids=[message["id"]]))["acked"] == [message["id"]]
        assert assert_ok(ctl.request("msg.ack", identity=address, ids=[message["id"]]))["acked"] == []
        assert_absent(_surface(ctl, sid, pid), "pendingMessages")
        assert assert_ok(ctl.request("msg.read", identity=address))["messages"] == []
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("msg.read", "field:result.acked", "field:result.remaining")
def test_read_ack_is_atomic_and_bounded(ctl):
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    try:
        ids = [assert_ok(ctl.request("msg.send", body=str(i), recipients=[address]))["messageID"] for i in range(33)]
        first = assert_ok(ctl.request("msg.read", identity=address, ack=True))
        assert [m["id"] for m in first["messages"]] == first["acked"] == ids[:32]
        assert first["remaining"] == 1
        assert _surface(ctl, sid, pid)["pendingMessages"] == 1
        second = assert_ok(ctl.request("msg.read", identity=address, ack=True))
        assert [m["id"] for m in second["messages"]] == second["acked"] == ids[32:]
        assert second["remaining"] == 0
        assert_absent(_surface(ctl, sid, pid), "pendingMessages")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("event:message")
def test_message_event_has_recipient_location_and_no_body(ctl):
    sender, sender_pane = _pane(ctl)
    sid, pid = _pane(ctl)
    identity = f"{sender}:{sender_pane}"
    try:
        cursor = ctl.events_baseline()
        receipt = assert_ok(ctl.request("msg.send", body="secret body", identity=identity, recipients=[f"{sid}:{pid}"]))
        event = ctl.wait_event(cursor, "message", {"session": sid, "payload": {"paneID": pid}})
        assert event["window"] and event["workspace"]
        assert_subset(event["payload"], {"from": identity, "messageID": receipt["messageID"]})
        assert_absent(event["payload"], "body")
    finally:
        assert_ok(ctl.request("session.close", sender))
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("msg.send", "field:result.rejected")
def test_message_limits_and_partial_rejections(ctl):
    sid, pid = _pane(ctl)
    other, other_pid = _pane(ctl)
    address, other_address = f"{sid}:{pid}", f"{other}:{other_pid}"
    try:
        assert_error(
            ctl.request("msg.send", body="x" * 65537, recipients=[address]),
            "message body too large (65537 bytes, max 65536)",
        )
        for _ in range(256):
            assert assert_ok(ctl.request("msg.send", body="x", recipients=[address]))["delivered"] == [address]
        result = assert_ok(ctl.request("msg.send", body="late", recipients=[address, other_address]))
        assert result["delivered"] == [other_address]
        assert result["rejected"] == [{"address": address, "reason": "mailbox full"}]
        assert _surface(ctl, sid, pid)["pendingMessages"] == 256
        assert assert_ok(ctl.request("msg.read", identity=other_address))["messages"][0]["body"] == "late"
    finally:
        assert_ok(ctl.request("session.close", sid))
        assert_ok(ctl.request("session.close", other))


@pytest.mark.covers("msg.send", "msg.read")
def test_broadcast_scope_and_identity(ctl):
    work = assert_ok(ctl.request("workspace.new", name="message scope"))["id"]
    sid, pid = _pane(ctl, workspace=work)
    other, other_pid = _pane(ctl, workspace=work)
    identity, recipient = f"{sid}:{pid}", f"{other}:{other_pid}"
    try:
        result = assert_ok(ctl.request("msg.send", body="scoped", all=True, workspace=work, identity=identity))
        assert result["delivered"] == [recipient]
        assert result["rejected"] == []
        assert assert_ok(ctl.request("msg.read", identity=identity))["messages"] == []
        assert assert_ok(ctl.request("msg.read", identity=recipient))["messages"][0]["from"] == identity
        forged = f"{sid}:{other_pid}"
        assert_error(
            ctl.request("msg.send", body="forged", all=True, workspace=work, identity=forged),
            f"pane {other_pid} is not a live pane of session {sid}",
        )
    finally:
        assert_ok(ctl.request("workspace.delete", work))


@pytest.mark.covers("msg.send", "msg.read")
def test_aliases_deliver_once(ctl):
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    try:
        result = assert_ok(ctl.request("msg.send", body="once", recipients=[address, f"{sid}:left", f"{sid}:primary"]))
        assert result["delivered"] == [address]
        assert len(assert_ok(ctl.request("msg.read", identity=address))["messages"]) == 1
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("msg.send", "msg.read", "session.swap")
def test_mailbox_follows_pane_identity(ctl):
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    try:
        assert_ok(ctl.request("session.split", sid, mode="on"))
        ctl.wait_mounted(sid, "right")
        message = assert_ok(ctl.request("msg.send", body="follows pane", recipients=[address]))["messageID"]
        assert_ok(ctl.request("session.swap", sid))
        assert _surface(ctl, sid, pid)["kind"] == "right"
        assert_ok(ctl.request("session.split", sid, mode="off"))
        assert assert_ok(ctl.request("msg.read", identity=address))["messages"][0]["id"] == message
        assert_ok(ctl.request("session.split.close", sid))
        assert_error(ctl.request("msg.read", identity=address), f"pane {pid} is not a live pane of session {sid}")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("msg.read", "session.close", "recent.open")
def test_mailbox_lifecycle(ctl):
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    try:
        assert_ok(ctl.request("msg.send", body="before close", recipients=[address]))
        receipt = assert_ok(ctl.request("session.close", sid, undoable=True))
        assert receipt["undoable"] is True
        (entry,) = [item for item in assert_ok(ctl.request("recent.list"))["recent"] if item["sessionIDs"] == [sid]]
        assert_ok(ctl.request("recent.open", entry["id"]))
        ctl.wait_mounted(sid, "left")
        assert assert_ok(ctl.request("msg.read", identity=address))["messages"] == []
        assert_absent(_surface(ctl, sid, pid), "pendingMessages")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.requires("live-sessions", "posix-command-line", reason="needs an old live program's HUP handler")
@pytest.mark.covers("msg.send", "msg.read", "session.restart")
def test_restart_drops_mail_sent_during_shutdown(launch, cli):
    """The ending program sends its own last message; it must not reach its replacement."""
    import json
    import shlex
    from agterm_conformance.wait import wait_until

    ctl = launch(settings={"restoreMode": "live"})
    sid, pid = _pane(ctl)
    address = f"{sid}:{pid}"
    receipt = ctl.instance.root / "shutdown-message.json"
    ready = ctl.instance.root / "old-program-ready"
    quote = shlex.quote
    send = (
        f"{quote(str(cli.ctl_path))} msg send shutdown --to {quote(address)} --as {quote(address)} "
        f"--json --socket {quote(str(ctl.instance.socket))} > {quote(str(receipt))}; exit 0"
    )
    script = f"trap {quote(send)} HUP; printf ready > {quote(str(ready))}; while :; do sleep 60; done"
    try:
        assert_ok(ctl.request("session.restart", sid, paneID=pid, command=f"sh -c {quote(script)}"))
        wait_until(lambda: ready.exists() and ready.read_text() == "ready")
        assert_ok(ctl.request("msg.send", body="before shutdown", recipients=[address]))
        assert_ok(ctl.request("session.restart", sid, paneID=pid, command="cat"))
        assert assert_ok(json.loads(receipt.read_text()))["delivered"] == [address]
        assert assert_ok(ctl.request("msg.read", identity=address))["messages"] == []
    finally:
        assert_ok(ctl.request("session.close", sid))
