"""Pane mailbox wire contract, including read/ack receipts and body-free events."""

import time
import uuid

import pytest
from agterm_conformance.match import assert_absent, assert_ok, assert_subset, find_session

pytestmark = pytest.mark.since("0.40.0")


def _pane(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
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
