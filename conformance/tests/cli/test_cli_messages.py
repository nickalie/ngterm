"""The bundled message CLI's observable send/read/ack records."""

import uuid

import pytest
from agterm_conformance.match import assert_ok, find_session

pytestmark = pytest.mark.since("0.40.0")


@pytest.mark.covers("cli:msg send", "cli:msg read", "cli:msg ack")
def test_send_read_ack_output(cli, ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    tree = ctl.wait_mounted(sid, "left")
    (pane,) = [p for p in find_session(tree, sid)["surfaces"] if p["kind"] == "left"]
    address = f'{sid}:{pane["paneID"]}'
    try:
        sent = cli("msg", "send", "hello", "--to", address, "--as", address)
        assert (sent.returncode, sent.stderr) == (0, "")
        message_id = sent.stdout.rstrip("\n")
        uuid.UUID(message_id)
        assert sent.stdout == message_id + "\n"
        read = cli("msg", "read", "--as", address)
        assert (read.returncode, read.stdout, read.stderr) == (0, f"{message_id} from {address}\nhello\n", "")
        ack = cli("msg", "ack", message_id, "--as", address)
        assert (ack.returncode, ack.stdout, ack.stderr) == (0, "acked 1\n", "")
        empty = cli("msg", "read", "--as", address, "--json")
        assert empty.returncode == 0 and empty.stderr == ""
        assert empty.json()["result"]["messages"] == []
    finally:
        assert_ok(ctl.request("session.close", sid))
