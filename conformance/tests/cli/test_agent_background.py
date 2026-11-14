"""A Claude Code conversation that moved to a background process: a Rust-only extension (capability `agent-chat`).

Claude Code carries a conversation on in a background job of its daemon on `/background` or on a Left arrow at its
empty prompt: a new conversation id and transcript, the old transcript ending with a `continued-in` row. The job's
hooks run with the daemon's environment, so their `NGTERM_*` name the pane the daemon was started in, not the pane
the conversation came from. `CLAUDE_JOB_DIR` tells such a job: its hooks run themselves again through
`ngtermctl agent rerun`, which asks `agent.locate` for the pane that holds the conversation (or the one it moved
from) and runs the hook as that pane, or runs nothing (conformance/docs/rust-build.md > Rust-only extensions).

The hooks are the installed ones: the staged `ngterm-headless --install-agent-hooks` installs them into a home of
the test's own, and each is run by the command line its `settings.json` entry holds, through the POSIX `sh` Claude
Code runs hooks in (Git for Windows' on Windows, where the entries call the PowerShell scripts), the payload on
standard input. Claude Code itself is not run: the payloads, the transcripts' rows and the job's environment are
what it hands a hook.

The status hook of a job counts the agents among its ancestors, so a suite run under two nested coding agents
would see these hooks stay silent."""

import json
import os
import sys
import uuid
from pathlib import Path

import pytest
from agterm_conformance.impls.base import posix_shell
from agterm_conformance.match import assert_ok, find_session, program_argv
from agterm_conformance.plugin import CliProcess
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.requires(
        "agent-chat",
        reason="a Rust-only extension: `agent.locate`, `ngtermctl agent rerun` and the hooks of a background job",
    ),
    pytest.mark.requires("posix-shell", reason="the pane's shell runs `sleep` as the agent stand-in"),
]

HOOK_TIMEOUT = 60.0
SILENT = (0, "", "")
NO_PANE = "no pane holds this conversation"
WINDOWS = sys.platform == "win32"


def _finished(cli, argv: list[str], env: dict[str, str], input: str | None):
    """`argv` run to its end in the instance's home, as the harness runs a CLI."""
    process = CliProcess(argv, env, cli.instance.home, input, False, cli.program)
    try:
        return process.finish(timeout=HOOK_TIMEOUT)
    finally:
        process.close()


class Hooks:
    """The agent hooks as the build installs them, in a home under the test's directory."""

    def __init__(self, cli, root: Path) -> None:
        self.cli = cli
        self.home = root / "home"
        (self.home / ".claude").mkdir(parents=True)
        ctl = Path(cli.ctl_path)
        installer = ctl.with_name("ngterm-headless" + ctl.suffix)
        assert installer.is_file(), f"the stage holds no {installer.name} beside {ctl.name}"
        env = dict(cli.instance.env)
        env.update(
            HOME=str(self.home),
            USERPROFILE=str(self.home),
            XDG_CONFIG_HOME=str(self.home / ".config"),
            APPDATA=str(self.home / "AppData" / "Roaming"),
            LOCALAPPDATA=str(self.home / "AppData" / "Local"),
        )
        done = _finished(cli, [str(installer), "--install-agent-hooks"], env, None)
        assert done.returncode == 0 and "Agent Status Hooks Installed" in done.stdout, done
        settings = json.loads((self.home / ".claude" / "settings.json").read_text(encoding="utf-8"))
        self.entries = settings["hooks"]

    def command(self, event: str, script: str) -> str:
        """The command line Claude Code runs on `event` whose script is `script` (its name without the extension);
        the mail hook's waiter is another entry."""
        found = [
            hook["command"]
            for group in self.entries[event]
            for hook in group["hooks"]
            if f"{script}." in hook["command"] and not hook.get("asyncRewake")
        ]
        assert len(found) == 1, (event, script, self.entries[event])
        return found[0]

    def run(self, event: str, script: str, payload: dict, env: dict[str, str]):
        """Runs the hook as Claude Code does; (exit status, stdout, stderr)."""
        state_dir = env.get(self.cli.instance.variable("STATE_DIR"), "")
        root = os.path.realpath(self.cli.instance.root)
        assert state_dir and os.path.realpath(state_dir).startswith(root), "the hook could reach another instance"
        done = _finished(self.cli, [posix_shell(), "-c", self.command(event, script)], env, json.dumps(payload))
        return done.returncode, done.stdout, done.stderr


class Conversations:
    """A Claude config tree under the test's directory: the transcripts of the conversations the test names."""

    def __init__(self, root: Path) -> None:
        self.project = root / "project"
        self.project.mkdir()
        self.directory = root / "claude" / "projects" / "-project"
        self.directory.mkdir(parents=True)

    def new(self) -> str:
        """A conversation with one prompt in its transcript; its id."""
        conversation = str(uuid.uuid4())
        self.append(conversation, self.message(conversation))
        return conversation

    def transcript(self, conversation: str) -> Path:
        return self.directory / f"{conversation}.jsonl"

    def append(self, conversation: str, row: dict) -> None:
        with self.transcript(conversation).open("a", encoding="utf-8", newline="\n") as transcript:
            transcript.write(json.dumps(row) + "\n")

    def message(self, conversation: str) -> dict:
        return {
            "parentUuid": None, "isSidechain": False, "type": "user", "uuid": str(uuid.uuid4()),
            "message": {"role": "user", "content": "go on"}, "timestamp": "2026-11-14T10:00:00.000Z",
            "cwd": str(self.project), "sessionId": conversation,
        }

    def move(self, conversation: str) -> str:
        """Claude Code moves `conversation` to a background job: the new conversation's id."""
        moved = self.new()
        self.append(conversation, {"type": "continued-in", "sessionId": conversation, "continuedInSessionId": moved})
        return moved

    def payload(self, event: str, conversation: str, **fields) -> dict:
        return {
            "session_id": conversation, "transcript_path": str(self.transcript(conversation)),
            "cwd": str(self.project), "permission_mode": "default", "hook_event_name": event, **fields,
        }

    def tool(self, conversation: str) -> dict:
        return self.payload("PostToolUse", conversation, tool_name="Bash", tool_input={"command": "true"})

    def stop(self, conversation: str) -> dict:
        return self.payload("Stop", conversation, background_tasks=[])

    def report(self, conversation: str, **fields) -> dict:
        """The `agent.locate` report of `conversation`."""
        return {
            "agent": "claude", "event": "SessionStart", "transcriptPath": str(self.transcript(conversation)),
            "conversationId": conversation, **fields,
        }


class Pane:
    """A new session whose left pane runs `sleep` in its shell, as a running agent would hold it."""

    def __init__(self, cli, ctl) -> None:
        self.cli = cli
        self.ctl = ctl
        self.sid = assert_ok(ctl.request("session.new"))["id"]
        tree = ctl.wait_mounted(self.sid, "left")
        pane = next(p for p in find_session(tree, self.sid)["surfaces"] if p["kind"] == "left")
        self.pane_id = pane["paneID"]
        self.address = f"{self.sid}:{self.pane_id}"
        assert_ok(ctl.request("session.type", self.sid, text="sleep 600\n"))
        ctl.wait_tree(lambda tree: program_argv(find_session(tree, self.sid).get("foreground")) == ["sleep", "600"])

    def env(self) -> dict[str, str]:
        """The environment of Claude Code's hooks in this pane."""
        return _hook_env(self.cli, self.sid, self.pane_id, self.cli.instance.socket)

    def status(self):
        return find_session(self.ctl.tree(), self.sid).get("status")

    def record(self) -> dict:
        return assert_ok(self.ctl.request("agent.get", self.sid, pane="left"))["agent"]

    def has_no_record(self) -> bool:
        response = self.ctl.request("agent.get", self.sid, pane="left")
        return not response.ok and response.error == "no agent in this pane"

    def send(self, body: str) -> None:
        sent = self.cli("msg", "send", body, "--to", self.address)
        assert (sent.returncode, sent.stderr) == (0, ""), sent

    def waiting(self) -> list[str]:
        """The bodies in the pane's mailbox, read without taking them."""
        read = self.cli("msg", "read", "--as", self.address, "--json")
        assert read.returncode == 0, read
        return [message["body"] for message in read.json()["result"]["messages"]]

    def close(self) -> None:
        assert_ok(self.ctl.request("session.close", self.sid))


def _hook_env(cli, session: str | None, pane_id: str | None, socket) -> dict[str, str]:
    """The instance's environment as a Claude Code hook sees it in the pane `session`/`pane_id` of the instance at
    `socket`; none of the three for a hook outside a pane."""
    variable = cli.instance.variable
    env = dict(cli.instance.env)
    for name in ("SESSION_ID", "PANE_ID", "PANE", "SOCKET", "AGENT_LOCATED"):
        env.pop(variable(name), None)
    env.pop("CLAUDE_JOB_DIR", None)
    env["CLAUDE_CODE_ENTRYPOINT"] = "cli"
    if session is not None:
        env[variable("SESSION_ID")] = session
        env[variable("PANE")] = "left"
    if pane_id is not None:
        env[variable("PANE_ID")] = pane_id
    if socket is not None:
        env[variable("SOCKET")] = str(socket)
    return env


def _job_env(cli, daemon: Pane | None, socket="instance") -> dict[str, str]:
    """The environment of a hook in a background job of Claude Code's daemon: `CLAUDE_JOB_DIR`, and the `NGTERM_*`
    of the pane the daemon was started in: `daemon`, or a session and a pane that are gone."""
    session, pane_id = (daemon.sid, daemon.pane_id) if daemon else (str(uuid.uuid4()).upper(), str(uuid.uuid4()).upper())
    env = _hook_env(cli, session, pane_id, cli.instance.socket if socket == "instance" else socket)
    env["CLAUDE_JOB_DIR"] = str(Path(cli.instance.home) / ".claude" / "jobs" / "0b0b0b0b")
    return env


@pytest.fixture
def hooks(cli, tmp_path):
    return Hooks(cli, tmp_path)


@pytest.fixture
def conversations(tmp_path):
    return Conversations(tmp_path)


@pytest.fixture
def panes(cli, ctl):
    opened = []

    def open_pane() -> Pane:
        opened.append(Pane(cli, ctl))
        return opened[-1]

    yield open_pane
    for pane in opened:
        pane.close()


def _started(hooks: Hooks, conversations: Conversations, pane: Pane) -> str:
    """Claude Code starts a conversation in `pane`, its own hooks reporting it; the conversation's id."""
    conversation = conversations.new()
    start = conversations.payload("SessionStart", conversation, source="startup")
    assert hooks.run("SessionStart", "ngterm-agent-restore", start, pane.env()) == SILENT
    assert pane.record()["conversationId"] == conversation
    return conversation


def test_the_hooks_of_claude_in_its_pane_work_as_before(hooks, conversations, panes):
    pane = panes()
    conversation = _started(hooks, conversations, pane)
    assert Path(pane.record()["transcriptPath"]) == conversations.transcript(conversation)
    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(conversation), pane.env()) == SILENT
    assert pane.status() == "active"
    asked = conversations.payload("PermissionRequest", conversation, tool_name="Bash", tool_input={"command": "true"})
    assert hooks.run("PermissionRequest", "ngterm-claude-status", asked, pane.env()) == SILENT
    assert pane.status() == "blocked"
    assert hooks.run("Stop", "ngterm-claude-status", conversations.stop(conversation), pane.env()) == SILENT
    assert pane.status() == "completed"
    pane.send("for the agent")
    code, out, err = hooks.run("PostToolUse", "ngterm-agent-mail", conversations.tool(conversation), pane.env())
    assert (code, err) == (0, ""), (code, out, err)
    assert json.loads(out)["hookSpecificOutput"]["additionalContext"].endswith("for the agent")
    assert pane.waiting() == []


def test_a_hook_outside_a_pane_does_nothing(hooks, conversations, panes, cli):
    pane = panes()
    conversation = _started(hooks, conversations, pane)
    pane.send("for the agent")
    outside = _hook_env(cli, None, None, None)
    for event, script, payload in (
        ("PostToolUse", "ngterm-claude-status", conversations.tool(conversation)),
        ("PostToolUse", "ngterm-agent-mail", conversations.tool(conversation)),
        ("Stop", "ngterm-claude-status", conversations.stop(conversation)),
        ("SessionEnd", "ngterm-agent-restore", conversations.payload("SessionEnd", conversation, reason="other")),
    ):
        assert hooks.run(event, script, payload, outside) == SILENT, (event, script)
    assert (pane.status(), pane.waiting()) == (None, ["for the agent"])
    assert pane.record()["conversationId"] == conversation


@pytest.mark.parametrize("daemon_pane", ["gone", "live"])
def test_a_background_jobs_hooks_reach_the_pane_its_conversation_came_from(
    hooks, conversations, panes, cli, daemon_pane,
):
    pane = panes()
    daemon = panes() if daemon_pane == "live" else None
    moved = conversations.move(_started(hooks, conversations, pane))
    job = _job_env(cli, daemon)
    if daemon:
        daemon.send("for the daemon's pane")

    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(moved), job) == SILENT
    assert pane.status() == "active"
    record = pane.record()
    assert (record["conversationId"], Path(record["transcriptPath"])) == (moved, conversations.transcript(moved))

    prompt = conversations.payload("UserPromptSubmit", moved, prompt="go on")
    assert hooks.run("UserPromptSubmit", "ngterm-claude-status", prompt, job) == SILENT
    asked = conversations.payload("PermissionRequest", moved, tool_name="Bash", tool_input={"command": "true"})
    assert hooks.run("PermissionRequest", "ngterm-claude-status", asked, job) == SILENT
    assert pane.status() == "blocked"
    assert pane.record()["approval"]["tool"] == "Bash"

    # the mail of the pane it came from is the job's, in the hook's own output, and no refusal reaches the agent
    pane.send("for the moved agent")
    code, out, err = hooks.run("PostToolUse", "ngterm-agent-mail", conversations.tool(moved), job)
    assert (code, err) == (0, ""), (code, out, err)
    assert json.loads(out)["hookSpecificOutput"]["additionalContext"].endswith("for the moved agent")
    assert pane.waiting() == []

    start = conversations.payload("SessionStart", moved, source="resume")
    assert hooks.run("SessionStart", "ngterm-agent-restore", start, job) == SILENT
    assert pane.record()["conversationId"] == moved
    assert hooks.run("Stop", "ngterm-claude-status", conversations.stop(moved), job) == SILENT
    assert pane.status() == "completed"

    if daemon:
        # the pane the daemon was started in is another conversation's: nothing of the job's lands there
        assert (daemon.status(), daemon.waiting()) == (None, ["for the daemon's pane"])
        assert daemon.has_no_record()


@pytest.mark.parametrize("daemon_pane", ["gone", "live"])
def test_a_background_job_no_pane_started_does_nothing_and_prints_nothing(
    hooks, conversations, panes, cli, daemon_pane,
):
    pane = panes()
    daemon = panes() if daemon_pane == "live" else None
    conversation = _started(hooks, conversations, pane)
    nobodys = conversations.new()
    job = _job_env(cli, daemon)
    for waiting in filter(None, (pane, daemon)):
        waiting.send("stays")
    for event, script, payload in (
        ("SessionStart", "ngterm-agent-restore", conversations.payload("SessionStart", nobodys, source="startup")),
        ("UserPromptSubmit", "ngterm-claude-status", conversations.payload("UserPromptSubmit", nobodys, prompt="hi")),
        ("PostToolUse", "ngterm-claude-status", conversations.tool(nobodys)),
        ("PostToolUse", "ngterm-agent-mail", conversations.tool(nobodys)),
        ("Stop", "ngterm-claude-status", conversations.stop(nobodys)),
        ("SessionEnd", "ngterm-agent-restore", conversations.payload("SessionEnd", nobodys, reason="other")),
    ):
        assert hooks.run(event, script, payload, job) == SILENT, (event, script)
    assert (pane.status(), pane.waiting()) == (None, ["stays"])
    assert pane.record()["conversationId"] == conversation
    if daemon:
        assert (daemon.status(), daemon.waiting()) == (None, ["stays"])
        assert daemon.has_no_record()


@pytest.mark.parametrize("daemons_socket", ["a dead instance's", "none"])
def test_a_background_job_whose_daemon_names_no_running_instance_asks_the_default_one(
    hooks, conversations, panes, cli, daemons_socket,
):
    pane = panes()
    moved = conversations.move(_started(hooks, conversations, pane))
    socket = Path(cli.instance.root) / "gone.sock" if daemons_socket != "none" else None
    job = _job_env(cli, None, socket=socket)
    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(moved), job) == SILENT
    assert pane.status() == "active"
    assert pane.record()["conversationId"] == moved


def test_a_conversation_the_user_went_back_to_is_its_first_process_again(hooks, conversations, panes, cli):
    pane = panes()
    conversation = _started(hooks, conversations, pane)
    moved = conversations.move(conversation)
    # a message after the move: Claude Code reads the conversation as never moved
    conversations.append(conversation, conversations.message(conversation))
    job = _job_env(cli, None)
    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(moved), job) == SILENT
    assert pane.status() is None
    assert pane.record()["conversationId"] == conversation


def test_a_conversation_moved_twice_is_still_its_first_panes(hooks, conversations, panes, cli):
    pane = panes()
    first = conversations.move(_started(hooks, conversations, pane))
    second = conversations.move(first)
    job = _job_env(cli, None)
    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(second), job) == SILENT
    assert pane.status() == "active"
    assert pane.record()["conversationId"] == second


def test_locate_answers_the_pane_that_holds_a_conversation_or_the_one_it_moved_from(
    hooks, conversations, panes, ctl,
):
    pane = panes()
    other = panes()
    conversation = _started(hooks, conversations, pane)
    elsewhere = _started(hooks, conversations, other)

    found = assert_ok(ctl.request("agent.locate", report=conversations.report(conversation)))
    assert found["id"] == pane.sid
    agent = found["agent"]
    assert (agent["agent"], agent["pane"], agent["paneID"]) == ("claude", "left", pane.pane_id)
    assert (agent["conversationId"], Path(agent["transcriptPath"])) == (
        conversation, conversations.transcript(conversation),
    )
    assert assert_ok(ctl.request("agent.locate", report=conversations.report(elsewhere)))["id"] == other.sid

    moved = conversations.move(conversation)
    found = assert_ok(ctl.request("agent.locate", report=conversations.report(moved)))
    assert (found["id"], found["agent"]["paneID"], found["agent"]["conversationId"]) == (
        pane.sid, pane.pane_id, moved,
    )
    # the record followed the move: the chat view reads the conversation where it is, and the old id is nobody's
    assert pane.record()["conversationId"] == moved
    gone = ctl.request("agent.locate", report=conversations.report(conversation))
    assert (gone.ok, gone.error) == (False, NO_PANE), gone.raw
    assert other.record()["conversationId"] == elsewhere


def test_locate_refuses_what_names_no_pane(hooks, conversations, panes, ctl):
    pane = panes()
    conversation = _started(hooks, conversations, pane)
    unknown = conversations.new()

    def refusal(**args):
        response = ctl.request("agent.locate", **args)
        assert not response.ok, response.raw
        return response.error

    assert refusal(report=conversations.report(unknown)) == NO_PANE
    # the same conversation is another agent's record
    assert refusal(report=conversations.report(conversation, agent="codex")).startswith("transcript path is outside")
    assert refusal() == "agent.locate requires a report"
    assert refusal(report=conversations.report(conversation, agent="gemini")) == "invalid agent: gemini (claude|codex)"
    for missing in ("transcriptPath", "conversationId"):
        report = conversations.report(conversation)
        del report[missing]
        assert refusal(report=report) == "agent.locate requires transcriptPath and conversationId"
    outside = conversations.report(conversation, transcriptPath=str(conversations.project / "elsewhere.jsonl"))
    assert refusal(report=outside).startswith("transcript path is outside a Claude projects/ tree: ")
    assert pane.record()["conversationId"] == conversation


PRINT_ENV = (
    "import json, os, sys; "
    "json.dump({'env': {k: v for k, v in os.environ.items() if k.startswith('NGTERM_') or k == 'CLAUDE_JOB_DIR'}, "
    "'argv': sys.argv[1:], 'stdin': sys.stdin.read()}, sys.stdout)"
)


def test_rerun_runs_the_hook_as_the_conversations_pane(hooks, conversations, panes, cli):
    pane = panes()
    moved = conversations.move(_started(hooks, conversations, pane))
    payload = json.dumps(conversations.tool(moved))
    result = cli("agent", "rerun", "claude", "--", sys.executable, "-c", PRINT_ENV, "active", "--blink", input=payload)
    assert (result.returncode, result.stderr) == (0, ""), result
    ran = result.json()
    variable = cli.instance.variable
    for name, value in (
        ("SESSION_ID", pane.sid), ("PANE", "left"), ("PANE_ID", pane.pane_id), ("AGENT_LOCATED", "1"),
    ):
        assert ran["env"].get(variable(name)) == value, (name, ran["env"])
    assert Path(ran["env"][variable("SOCKET")]) == Path(cli.instance.socket)
    assert (ran["argv"], ran["stdin"]) == (["active", "--blink"], payload)


def test_rerun_passes_the_hooks_output_and_exit_status_on(hooks, conversations, panes, cli):
    pane = panes()
    conversation = _started(hooks, conversations, pane)
    hook = "import sys; print('to stdout'); print('to stderr', file=sys.stderr); sys.exit(7)"
    result = cli(
        "agent", "rerun", "claude", "--", sys.executable, "-c", hook, input=json.dumps(conversations.tool(conversation)),
    )
    assert (result.returncode, result.stdout.splitlines(), result.stderr.splitlines()) == (
        7, ["to stdout"], ["to stderr"],
    ), result


@pytest.mark.parametrize(
    "payload",
    [
        pytest.param("unknown", id="a conversation no pane holds"),
        pytest.param("not json", id="a payload that is not JSON"),
        pytest.param("{}", id="a payload that names no conversation"),
        pytest.param("", id="no payload"),
    ],
)
def test_rerun_runs_nothing_and_prints_nothing_without_a_pane(hooks, conversations, panes, cli, tmp_path, payload):
    pane = panes()
    _started(hooks, conversations, pane)
    if payload == "unknown":
        payload = json.dumps(conversations.tool(conversations.new()))
    ran = tmp_path / "ran"
    hook = f"import pathlib; pathlib.Path({str(ran)!r}).write_text('ran'); print('ran')"
    result = cli("agent", "rerun", "claude", "--", sys.executable, "-c", hook, input=payload)
    assert (result.returncode, result.stdout, result.stderr) == SILENT, result
    assert not ran.exists()


def test_rerun_refuses_an_unknown_agent_and_a_missing_hook(conversations, cli):
    payload = json.dumps(conversations.tool(conversations.new()))
    unknown = cli("agent", "rerun", "gemini", "--", sys.executable, "-c", "print('ran')", input=payload)
    assert unknown.returncode == 64 and unknown.stdout == "", unknown
    assert "unknown agent: gemini (claude|codex)" in unknown.stderr
    missing = cli("agent", "rerun", "claude", input=payload)
    assert missing.returncode == 64 and missing.stdout == "", missing
    assert missing.stderr.startswith("Error: Missing expected argument '<hook> ...'\n"), missing
    assert "Usage: ngtermctl agent rerun <agent> -- <hook> ..." in missing.stderr


def test_a_record_that_followed_a_move_clears_once_the_pane_is_back_at_its_shell(
    hooks, conversations, panes, cli, ctl,
):
    pane = panes()
    moved = conversations.move(_started(hooks, conversations, pane))
    job = _job_env(cli, None)
    assert hooks.run("PostToolUse", "ngterm-claude-status", conversations.tool(moved), job) == SILENT
    assert pane.record()["conversationId"] == moved
    assert_ok(ctl.request("session.type", pane.sid, text="\x03"))
    ctl.wait_tree(lambda tree: find_session(tree, pane.sid).get("foregroundShell"))
    wait_until(pane.has_no_record)
    # the job goes on, but its pane holds no agent any more
    assert hooks.run("PostToolUse", "ngterm-agent-mail", conversations.tool(moved), job) == SILENT
