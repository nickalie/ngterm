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


def _pane(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    tree = ctl.wait_mounted(sid, "left")
    pane = next(p for p in find_session(tree, sid)["surfaces"] if p["kind"] == "left")
    return sid, f'{sid}:{pane["paneID"]}'


@pytest.mark.covers("cli:msg send", "cli:msg read")
def test_identity_environment(cli, ctl):
    sid, address = _pane(ctl)
    session, pane = address.split(":")
    env = dict(cli.instance.env)
    env[cli.instance.variable("SESSION_ID")] = session
    env[cli.instance.variable("PANE_ID")] = pane
    try:
        sent = cli.local("msg", "send", "environment", "--to", address, "--socket", str(cli.instance.socket), env=env)
        assert (sent.returncode, sent.stderr) == (0, "")
        read = cli("msg", "read", "--as", address)
        assert read.stdout == f"{sent.stdout.rstrip()} from {address}\nenvironment\n"
        del env[cli.instance.variable("PANE_ID")]
        refused = cli.local("msg", "send", "incomplete", "--to", address, "--socket", str(cli.instance.socket), env=env)
        text = cli.vocabulary.text(
            "incomplete pane identity: NGTERM_SESSION_ID and NGTERM_PANE_ID must both be set; pass --as SESSION:PANE-ID"
        )
        assert (refused.returncode, refused.stdout, refused.stderr) == (1, "", f"Error: {text}\n")
        override = cli.local("msg", "send", "override", "--to", address, "--as", address, "--socket", str(cli.instance.socket), env=env)
        assert (override.returncode, override.stderr) == (0, "")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:msg send")
def test_send_partial_failure(cli, ctl):
    sid, address = _pane(ctl)
    unknown = f"{uuid.uuid4()}:{uuid.uuid4()}"
    try:
        sent = cli("msg", "send", "partial", "--to", address, "--to", unknown)
        assert (sent.returncode, sent.stderr) == (1, f"rejected {unknown}: unknown pane\n")
        uuid.UUID(sent.stdout.rstrip())
        read = cli("msg", "read", "--as", address)
        sender = cli.vocabulary.text("outside ngterm")
        assert read.stdout == f"{sent.stdout.rstrip()} from {sender}\npartial\n"
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:msg read")
def test_read_wait_timeout(cli, ctl):
    sid, address = _pane(ctl)
    try:
        read = cli("msg", "read", "--as", address, "--wait", "--timeout", "0.1")
        assert (read.returncode, read.stdout, read.stderr) == (124, "", "timed out\n")
    finally:
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:msg read")
@pytest.mark.parametrize("acked", [None, [], ["00000000-0000-4000-8000-000000000000"]])
def test_read_ack_requires_confirmation(cli, acked):
    """The CLI must refuse an old server that ignored --ack, even under --json."""
    import dataclasses
    import json
    import threading
    from agterm_conformance import transport

    path = cli.instance.root / f"m{uuid.uuid4().hex[:6]}.sock"
    server = transport.Listener(path)
    message = {"id": str(uuid.uuid4()), "to": "S:P", "ts": 1, "body": "must not print"}
    payload = {"messages": [message], "remaining": 0}
    if acked is not None:
        payload["acked"] = acked
    requests = []

    def serve():
        with server.accept() as connection:
            line = b""
            while not line.endswith(b"\n"):
                chunk = connection.recv(4096)
                if not chunk:
                    return
                line += chunk
            requests.append(json.loads(line))
            connection.sendall(json.dumps({"ok": True, "result": payload}).encode() + b"\n")

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    try:
        fake = cli.on(dataclasses.replace(cli.instance, socket=path))
        read = fake("msg", "read", "--as", "S:P", "--ack", "--json")
        assert (read.returncode, read.stdout, read.stderr) == (
            1, "", "error: the app did not confirm which messages it removed; nothing was printed\n"
        )
    finally:
        server.close()
        thread.join(5)
    assert not thread.is_alive()
    assert requests == [{"cmd": "msg.read", "args": {"identity": "S:P", "ack": True}}]
