"""`ngtermctl agent mail`: the pane's mailbox handed to its coding agent, a Rust-only extension (capability `agent-chat`).

The agent-status hooks run it with the hook's payload on stdin (conformance/docs/rust-build.md > Commands). It takes
the pane's mail over upstream's `msg.read --ack` and answers in the hook protocol Claude Code and Codex share: added
context on `PostToolUse` and `UserPromptSubmit`, a block with the mail as its reason on `Stop`. With `--wait` (Claude
Code's asyncRewake hook) it waits for mail, turns the session's status back to active and exits 2 with the mail on
stderr. Only the pane's interactive agent takes mail: a subagent's event, a Claude Code that is not the CLI, or a
Codex rollout not created by its TUI leave it in the mailbox.

A Codex payload's run is also refused by the argv of a headless Codex among the hook's ancestors (Linux and macOS):
the suite run under `codex exec` would see its Codex tests refuse."""

import json
import threading
import uuid
from pathlib import Path

import pytest
from agterm_conformance import transport
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import CliProcess
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.requires(
    "agent-chat",
    reason="a Rust-only extension: `ngtermctl agent mail`, the agent-status hooks' delivery of the pane's mail",
)

# How long a waiter may take: it holds its first read a second and reads every half second.
WAITER_TIMEOUT = 20.0


class Pane:
    """A new session's left pane, its mail address and the env an agent's hook in it runs with."""

    def __init__(self, cli, ctl) -> None:
        self.cli = cli
        self.ctl = ctl
        self.sid = assert_ok(ctl.request("session.new"))["id"]
        tree = ctl.wait_mounted(self.sid, "left")
        pane = next(p for p in find_session(tree, self.sid)["surfaces"] if p["kind"] == "left")
        self.pane_id = pane["paneID"]
        self.address = f"{self.sid}:{self.pane_id}"

    def env(self, entrypoint: str | None = "cli", in_pane: bool = True) -> dict[str, str]:
        """The instance's env as a hook in this pane sees it: the pane's identity and Claude Code's entry point."""
        env = dict(self.cli.instance.env)
        for name in ("SESSION_ID", "PANE_ID", "PANE"):
            env.pop(self.cli.instance.variable(name), None)
        env.pop("CLAUDE_CODE_ENTRYPOINT", None)
        if in_pane:
            env[self.cli.instance.variable("SESSION_ID")] = self.sid
            env[self.cli.instance.variable("PANE_ID")] = self.pane_id
        if entrypoint is not None:
            env["CLAUDE_CODE_ENTRYPOINT"] = entrypoint
        return env

    def send(self, body: str, sender: str | None = None) -> None:
        args = ("msg", "send", body, "--to", self.address, *(("--as", sender) if sender else ()))
        sent = self.cli(*args)
        assert (sent.returncode, sent.stderr) == (0, ""), sent

    def mail(self, agent: str, payload: dict, *, env: dict[str, str] | None = None, extra: tuple[str, ...] = ()):
        return self.cli.local(
            "agent", "mail", agent, *extra, "--socket", str(self.cli.instance.socket),
            env=self.env() if env is None else env, input=json.dumps(payload),
        )

    def waiter(self, env: dict[str, str] | None = None) -> CliProcess:
        argv = [str(self.cli.ctl_path), "agent", "mail", "claude", "--wait", "--socket", str(self.cli.instance.socket)]
        payload = json.dumps({"hook_event_name": "Stop"})
        return CliProcess(argv, self.env() if env is None else env, self.cli.instance.home, payload, False,
                          self.cli.program)

    def waiting(self) -> list[str]:
        """The bodies still in the mailbox, read without taking them."""
        read = self.cli("msg", "read", "--as", self.address, "--json")
        assert read.returncode == 0, read
        return [message["body"] for message in read.json()["result"]["messages"]]

    def close(self) -> None:
        assert_ok(self.ctl.request("session.close", self.sid))


@pytest.fixture
def pane(cli, ctl):
    pane = Pane(cli, ctl)
    yield pane
    pane.close()


def _one(sender: str, body: str) -> str:
    return (
        "ngterm mail: a message arrived in this pane's mailbox (sent with `ngtermctl msg send`).\n\n"
        f"Message 1 of 1, from {sender} (answer with `ngtermctl msg send --to {sender} --stdin`):\n{body}"
    )


def test_a_tool_call_hands_the_mail_over_as_added_context(pane, ctl):
    sender = Pane(pane.cli, ctl)
    try:
        pane.send("rebuild the docs", sender=sender.address)
        result = pane.mail("claude", {"hook_event_name": "PostToolUse", "tool_name": "Bash"})
        assert (result.returncode, result.stderr) == (0, ""), result
        assert result.json() == {
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": _one(sender.address, "rebuild the docs"),
            },
        }
        assert pane.waiting() == []
        # the mail went with that read: the next event finds none and prints nothing
        again = pane.mail("claude", {"hook_event_name": "UserPromptSubmit", "prompt": "go on"})
        assert (again.returncode, again.stdout, again.stderr) == (0, "", "")
    finally:
        sender.close()


def test_a_stop_keeps_the_turn_going_with_every_message_as_its_reason(pane):
    pane.send("first\n")
    pane.send("second")
    result = pane.mail("claude", {"hook_event_name": "Stop"})
    assert (result.returncode, result.stderr) == (0, ""), result
    assert result.json() == {
        "decision": "block",
        "reason": (
            "ngterm mail: 2 messages arrived in this pane's mailbox (sent with `ngtermctl msg send`).\n\n"
            "Message 1 of 2, from outside ngterm:\nfirst\n\n"
            "Message 2 of 2, from outside ngterm:\nsecond"
        ),
    }
    assert pane.waiting() == []


@pytest.mark.parametrize(
    ("payload", "entrypoint"),
    [
        pytest.param({"hook_event_name": "SessionStart"}, "cli", id="an event that hands nothing over"),
        pytest.param({"hook_event_name": "Stop", "agent_id": "a1"}, "cli", id="a subagent's event"),
        pytest.param({"hook_event_name": "Stop"}, "sdk-cli", id="a claude -p worker"),
        pytest.param({"hook_event_name": "Stop"}, None, id="no entry point"),
        pytest.param({}, "cli", id="no event"),
    ],
)
def test_a_claude_that_is_not_the_panes_agent_takes_nothing(pane, payload, entrypoint):
    pane.send("for the agent")
    result = pane.mail("claude", payload, env=pane.env(entrypoint=entrypoint))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert pane.waiting() == ["for the agent"]


def test_a_hook_outside_a_pane_takes_nothing(pane):
    pane.send("for the agent")
    result = pane.mail("claude", {"hook_event_name": "Stop"}, env=pane.env(in_pane=False))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert pane.waiting() == ["for the agent"]


def _rollout(directory: Path, source) -> Path:
    path = directory / "rollout.jsonl"
    meta = {"type": "session_meta", "payload": {"id": "s1", "source": source}}
    path.write_text(json.dumps(meta) + "\n" + json.dumps({"type": "event_msg"}) + "\n", encoding="utf-8")
    return path


def test_codexs_tui_takes_the_mail(pane, tmp_path):
    pane.send("for codex")
    payload = {"hook_event_name": "Stop", "transcript_path": str(_rollout(tmp_path, "cli"))}
    result = pane.mail("codex", payload)
    assert (result.returncode, result.stderr) == (0, ""), result
    assert result.json()["reason"].endswith("Message 1 of 1, from outside ngterm:\nfor codex")
    assert pane.waiting() == []


@pytest.mark.parametrize(
    "source",
    [
        pytest.param("exec", id="codex exec"),
        pytest.param("vscode", id="an app-server client"),
        pytest.param({"subagent": "review"}, id="a subagent"),
        pytest.param(None, id="no readable rollout"),
    ],
)
def test_a_codex_run_not_created_by_its_tui_takes_nothing(pane, tmp_path, source):
    pane.send("for codex")
    rollout = _rollout(tmp_path, source) if source is not None else tmp_path / "missing.jsonl"
    result = pane.mail("codex", {"hook_event_name": "Stop", "transcript_path": str(rollout)})
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert pane.waiting() == ["for codex"]


def test_a_read_the_app_refuses_fails_the_hook(pane):
    env = pane.env(in_pane=False)
    result = pane.mail("claude", {"hook_event_name": "Stop"}, env=env, extra=("--as", f"{pane.sid}:no-such-pane"))
    assert result.returncode == 1 and result.stdout == "" and result.stderr.startswith("Error: "), result


def test_a_read_that_does_not_confirm_its_removal_hands_nothing_over(pane):
    """An app that ignored the ack (an older one) would leave the mail queued: the hook hands none of it over."""
    path = pane.cli.instance.root / f"m{uuid.uuid4().hex[:6]}.sock"
    server = transport.Listener(path)
    message = {"id": str(uuid.uuid4()), "to": "S:P", "ts": 1, "body": "must not reach the agent"}
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
            reply = {"ok": True, "result": {"messages": [message], "remaining": 0}}
            connection.sendall(json.dumps(reply).encode() + b"\n")

    thread = threading.Thread(target=serve, daemon=True)
    thread.start()
    try:
        result = pane.cli.local(
            "agent", "mail", "claude", "--as", "S:P", "--socket", str(path),
            env=pane.env(in_pane=False), input=json.dumps({"hook_event_name": "UserPromptSubmit", "prompt": "go on"}),
        )
        expected = "Error: the app did not confirm which messages it removed\n"
        assert (result.returncode, result.stdout, result.stderr) == (1, "", expected)
    finally:
        server.close()
        thread.join(5)
    assert not thread.is_alive()
    assert requests == [{"cmd": "msg.read", "args": {"identity": "S:P", "ack": True}}]


def test_a_payload_that_is_not_json_fails_the_hook(pane):
    result = pane.cli.local(
        "agent", "mail", "claude", "--socket", str(pane.cli.instance.socket), env=pane.env(), input="not json",
    )
    assert result.returncode == 1 and result.stdout == "", result
    assert result.stderr.startswith("Error: the hook payload is not JSON: "), result


def test_codex_has_no_waiter(pane):
    result = pane.mail("codex", {"hook_event_name": "Stop"}, extra=("--wait",))
    assert result.returncode == 64 and result.stdout == "", result
    assert "--wait is Claude Code's asyncRewake hook: codex has no way to be woken" in result.stderr


def test_a_waiter_wakes_the_agent_with_its_mail_and_marks_it_active(pane, ctl):
    assert_ok(ctl.request("session.status", pane.sid, status="completed"))
    waiter = pane.waiter()
    try:
        pane.send("wake up")
        result = waiter.finish(timeout=WAITER_TIMEOUT)
    finally:
        waiter.close()
    assert (result.returncode, result.stdout, result.stderr) == (2, "", _one_outside("wake up") + "\n"), result
    assert pane.waiting() == []
    ctl.wait_tree(lambda tree: find_session(tree, pane.sid).get("status") == "active")


def _one_outside(body: str) -> str:
    return (
        "ngterm mail: a message arrived in this pane's mailbox (sent with `ngtermctl msg send`).\n\n"
        f"Message 1 of 1, from outside ngterm:\n{body}"
    )


def _claimed_by(claim: Path, waiter: CliProcess) -> bool:
    """Whether `waiter` holds the claim: the file names its pid (the claim may not be written yet)."""
    try:
        return claim.read_text(encoding="utf-8").startswith(f"{waiter.process.pid}-")
    except FileNotFoundError:
        return False


def _claim(pane: Pane) -> Path:
    stem = "".join(c if c.isascii() and (c.isalnum() or c == "-") else "_" for c in pane.address)
    return Path(pane.cli.instance.socket).parent / "agent-mail" / f"{stem}.waiter"


def test_a_newer_waiter_supersedes_the_older_one(pane):
    older = pane.waiter()
    newer = None
    try:
        claim = _claim(pane)
        wait_until(lambda: _claimed_by(claim, older))
        newer = pane.waiter()
        wait_until(lambda: _claimed_by(claim, newer))
        superseded = older.finish(timeout=WAITER_TIMEOUT)
        assert (superseded.returncode, superseded.stdout, superseded.stderr) == (0, "", ""), superseded
        pane.send("for the newest")
        woken = newer.finish(timeout=WAITER_TIMEOUT)
        assert (woken.returncode, woken.stderr) == (2, _one_outside("for the newest") + "\n"), woken
        # the waiter that took the mail leaves no claim behind
        assert not claim.exists()
    finally:
        older.close()
        if newer is not None:
            newer.close()


def test_a_waiter_ends_quietly_once_its_pane_is_gone(cli, ctl):
    pane = Pane(cli, ctl)
    waiter = pane.waiter()
    try:
        wait_until(_claim(pane).exists)
        pane.close()
        result = waiter.finish(timeout=WAITER_TIMEOUT)
    finally:
        waiter.close()
    assert (result.returncode, result.stdout, result.stderr) == (0, "", ""), result
