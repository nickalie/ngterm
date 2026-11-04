"""The origin's agent record behind the Android chat view: a Rust-only extension (capability `agent-chat`).

The agent-status hooks pipe Claude Code's and Codex's hook payloads to `ngtermctl agent hook`, which keeps the pane's
agent record in the app (`agent.report`); `agent.get` reads it back; `ngtermctl agent chat` streams the conversation
its transcript holds as NDJSON chat events, and `ngtermctl agent commands` lists the agent's slash commands and skills
(conformance/docs/rust-build.md > Rust-only extensions). Upstream has no such group, so the reference skips these
tests as unsupported; the Rust CI runs them with `--must-pass agent-chat`.

The transcripts and payloads are the recorded ones of `ngterm-core`'s parser tests
(`crates/ngterm-core/tests/fixtures/agent_chat`), copied into the test's own directory as an agent's config tree, their
recorded project directory pointed at one there. The record holds while the pane runs a program, so each pane runs
`sleep` in its shell, as a running agent would hold it."""

import json
import shutil
import uuid
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok, find_session, program_argv
from agterm_conformance.wait import wait_until

AGENT_CHAT = pytest.mark.requires(
    "agent-chat",
    reason="a Rust-only extension: the Android chat view's per-pane agent record and `ngtermctl agent`",
)
POSIX_SHELL = pytest.mark.requires("posix-shell", reason="the pane's shell runs `sleep` as the agent stand-in")
pytestmark = [AGENT_CHAT, POSIX_SHELL]

FIXTURES = Path(__file__).resolve().parents[3] / "crates" / "ngterm-core" / "tests" / "fixtures" / "agent_chat"
# The project directory the recordings name; the copies name the test's own.
RECORDED_PROJECT = "/home/user/project"
CLAUDE_ID = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4"
CODEX_ID = "01a1a300-e2e3-76a1-abcf-961cbbb2eb3c"
CODEX_DAY = ("2026", "11", "03")
CODEX_ROLLOUT = f"rollout-2026-11-03T23-01-46-{CODEX_ID}.jsonl"
NO_AGENT = "no agent in this pane"
# How long a stream's next line may take: the follower polls every 250 ms and reads the record again each second.
STREAM_WAIT = 20.0


class Agents:
    """The two agents' config trees under `root`: Claude's (`claude/`, its `projects/`) and Codex's (`codex/`, its
    `sessions/`), and the project both ran in (`project/`)."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.project = root / "project"
        self.project.mkdir()
        self.claude = root / "claude"
        self.codex = root / "codex"
        skip = shutil.ignore_patterns("*.golden.ndjson")
        shutil.copytree(FIXTURES / "claude" / "projects", self.claude / "projects", ignore=skip)
        shutil.copytree(FIXTURES / "codex" / "sessions", self.codex / "sessions", ignore=skip)
        recorded = json.dumps(RECORDED_PROJECT)[1:-1]
        ours = json.dumps(str(self.project))[1:-1]
        for transcript in root.rglob("*.jsonl"):
            text = transcript.read_text(encoding="utf-8")
            transcript.write_text(text.replace(recorded, ours), encoding="utf-8", newline="\n")
        self.claude_transcript = self.claude / "projects" / "-home-user-project" / f"{CLAUDE_ID}.jsonl"
        self.codex_transcript = self.codex / "sessions" / Path(*CODEX_DAY) / CODEX_ROLLOUT

    def payload(self, agent: str, hook: str, **fields) -> str:
        """The recorded hook payload `hook` (a file under the agent's `hooks/`) naming the copied transcript and
        project, with `fields` over it."""
        directory = FIXTURES / "claude" / "hooks" if agent == "claude" else FIXTURES / "codex" / "hooks" / "main"
        payload = json.loads((directory / hook).read_text(encoding="utf-8"))
        payload["transcript_path"] = str(self.claude_transcript if agent == "claude" else self.codex_transcript)
        payload["cwd"] = str(self.project)
        payload.update(fields)
        return json.dumps(payload)

    def append_prompt(self, text: str) -> str:
        """A prompt appended to Claude's transcript as Claude Code writes one; returns its id."""
        prompt_id = str(uuid.uuid4())
        line = {
            "parentUuid": None,
            "isSidechain": False,
            "type": "user",
            "message": {"role": "user", "content": text},
            "uuid": prompt_id,
            "timestamp": "2026-11-03T20:30:00.000Z",
            "cwd": str(self.project),
            "sessionId": CLAUDE_ID,
        }
        with self.claude_transcript.open("a", encoding="utf-8", newline="\n") as transcript:
            transcript.write(json.dumps(line) + "\n")
        return prompt_id


@pytest.fixture
def agents(tmp_path):
    return Agents(tmp_path)


def _running_pane(ctl):
    """A new session whose left pane runs `sleep` in its shell."""
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.type", sid, text="sleep 600\n"))
    ctl.wait_tree(lambda tree: program_argv(find_session(tree, sid).get("foreground")) == ["sleep", "600"])
    return sid


def _hook(cli, sid, agent, event, payload):
    """`agent hook` as the status hooks run it: the payload on stdin, the pane named; it prints nothing."""
    result = cli("agent", "hook", agent, event, "--target", sid, "--pane", "primary", input=payload)
    assert (result.returncode, result.stdout, result.stderr) == (0, "", ""), result
    return result


def _record(ctl, sid):
    return assert_ok(ctl.request("agent.get", sid, pane="left"))["agent"]


def _no_record(ctl, sid):
    response = ctl.request("agent.get", sid, pane="left")
    assert not response.ok and response.error == NO_AGENT, response.raw
    return True


def _events(text):
    return [json.loads(line) for line in text.splitlines() if line]


def _until_live(stream):
    """The events a stream prints up to and including its next `live`."""
    events = []
    while True:
        event = json.loads(stream.line(timeout=STREAM_WAIT))
        events.append(event)
        if event["type"] == "live":
            return events


def _end(stream):
    """Ends a stream the way the phone's channel closing does: its stdin's end. It exits by itself, with 0."""
    stream.process.stdin.close()
    result = stream.finish()
    assert result.returncode == 0, result


def test_a_claude_hook_sets_the_record_and_its_end_clears_it(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        _no_record(ctl, sid)
        _hook(cli, sid, "claude", "SessionStart", agents.payload("claude", "01-SessionStart.json"))
        record = _record(ctl, sid)
        assert (record["agent"], record["pane"], record["conversationId"]) == ("claude", "left", CLAUDE_ID)
        assert Path(record["transcriptPath"]) == agents.claude_transcript
        assert "approval" not in record
        # a prompt re-asserts the same conversation
        _hook(cli, sid, "claude", "UserPromptSubmit", agents.payload("claude", "02-UserPromptSubmit.json"))
        assert _record(ctl, sid)["conversationId"] == CLAUDE_ID
        # another conversation's end leaves it; its own clears it
        _hook(cli, sid, "claude", "SessionEnd", agents.payload("claude", "59-SessionEnd.json", session_id=str(uuid.uuid4())))
        assert _record(ctl, sid)["conversationId"] == CLAUDE_ID
        _hook(cli, sid, "claude", "SessionEnd", agents.payload("claude", "59-SessionEnd.json"))
        _no_record(ctl, sid)
    finally:
        assert_ok(ctl.request("session.close", sid))


def test_a_permission_prompt_is_an_approval_only_while_the_pane_is_blocked(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        _hook(cli, sid, "claude", "SessionStart", agents.payload("claude", "01-SessionStart.json"))
        _hook(cli, sid, "claude", "PermissionRequest", agents.payload("claude", "33-PermissionRequest.json"))
        assert "approval" not in _record(ctl, sid)
        status = ("session", "status", "--target", sid, "--pane", "primary")
        assert cli(*status[:2], "blocked", *status[2:]).returncode == 0
        approval = _record(ctl, sid)["approval"]
        assert approval["tool"] == "Bash"
        assert "touch allowed.txt" in approval["title"] + approval.get("detail", "")
        assert cli(*status[:2], "active", *status[2:]).returncode == 0
        assert "approval" not in _record(ctl, sid)
    finally:
        assert_ok(ctl.request("session.close", sid))


def test_the_chat_streams_the_claude_transcript_and_follows_it(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        _hook(cli, sid, "claude", "SessionStart", agents.payload("claude", "01-SessionStart.json"))
        stream = cli.spawn("agent", "chat", "--target", sid, interactive=True)
        try:
            backlog = _until_live(stream)
            kinds = {event["type"] for event in backlog}
            assert {"conversation", "user", "assistant", "tool", "live"} <= kinds, kinds
            assert any(event["type"] == "user" and "greeter program" in event["text"] for event in backlog)
            assert any(event["type"] == "tool" and event.get("kind") == "shell" for event in backlog)
            # the subagent's own transcript folds under its task
            assert any(event.get("parent") for event in backlog), backlog
            # a prompt the agent writes now arrives, then a `live` to resume after
            prompt_id = agents.append_prompt("one more thing")
            followed = _until_live(stream)
            (prompt,) = [event for event in followed if event["id"] == prompt_id]
            assert (prompt["type"], prompt["text"]) == ("user", "one more thing")
            resume = followed[-1]["cursor"]
        finally:
            if stream.process.poll() is None:
                _end(stream)
        cursors = sorted({event["cursor"] for event in backlog if not event.get("parent") and event["type"] != "live"})
        middle = cursors[len(cursors) // 2]

        # one page of the lines before a cursor, then the end: no `live`
        page = cli("agent", "chat", "--target", sid, "--before", str(middle), "--limit", "5")
        assert page.returncode == 0, page
        older = _events(page.stdout)
        assert older and all(event["cursor"] < middle for event in older if not event.get("parent")), older
        assert all(event["type"] != "live" for event in older)
        start = cli("agent", "chat", "--target", sid, "--before", "0")
        assert (start.returncode, start.stdout) == (0, ""), start

        # a reconnect after the last `live` sends nothing old, then `live`
        stream = cli.spawn("agent", "chat", "--target", sid, "--after", str(resume), "--conversation", CLAUDE_ID,
                           interactive=True)
        try:
            again = _until_live(stream)
            assert prompt_id not in {event["id"] for event in again}, again
        finally:
            _end(stream)
        # a cursor read in another conversation starts over
        moved = cli("agent", "chat", "--target", sid, "--before", str(resume), "--conversation", str(uuid.uuid4()))
        assert [event["type"] for event in _events(moved.stdout)] == ["reset"], moved
    finally:
        assert_ok(ctl.request("session.close", sid))


def test_the_chat_ends_once_the_pane_is_back_at_its_shell(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        _hook(cli, sid, "claude", "SessionStart", agents.payload("claude", "01-SessionStart.json"))
        stream = cli.spawn("agent", "chat", "--target", sid, "--limit", "3", interactive=True)
        try:
            _until_live(stream)
            assert_ok(ctl.request("session.type", sid, text="\x03"))
            ctl.wait_tree(lambda tree: find_session(tree, sid).get("foregroundShell"))
            # the record clears at its next read, which ends the stream by itself
            result = stream.finish(timeout=STREAM_WAIT)
            assert result.returncode == 0, result
        finally:
            stream.close()
        _no_record(ctl, sid)
        gone = cli("agent", "chat", "--target", sid)
        assert (gone.returncode, gone.stdout) == (1, ""), gone
        assert NO_AGENT in gone.stderr
    finally:
        assert_ok(ctl.request("session.close", sid))


def test_the_chat_streams_a_codex_rollout_with_its_subagent(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        _hook(cli, sid, "codex", "SessionStart", agents.payload("codex", "01-SessionStart.json"))
        record = _record(ctl, sid)
        assert (record["agent"], record["conversationId"]) == ("codex", CODEX_ID)
        stream = cli.spawn("agent", "chat", "--target", sid, interactive=True)
        try:
            backlog = _until_live(stream)
        finally:
            _end(stream)
        kinds = {event["type"] for event in backlog}
        assert {"conversation", "user", "assistant", "tool"} <= kinds, kinds
        assert any(event.get("parent") for event in backlog), backlog
        # Codex's permission prompt, while the pane is blocked
        _hook(cli, sid, "codex", "PermissionRequest", agents.payload("codex", "25-PermissionRequest.json"))
        assert cli("session", "status", "blocked", "--target", sid, "--pane", "primary").returncode == 0
        assert _record(ctl, sid)["approval"]["tool"] == "Bash"
    finally:
        assert_ok(ctl.request("session.close", sid))


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")


def _skill(directory: Path, name: str, description: str) -> None:
    _write(directory / name / "SKILL.md", f"---\nname: {name}\ndescription: {description}\n---\n\nThe steps.\n")


def test_the_commands_list_the_agents_own_and_installed_ones(cli, ctl, agents):
    _skill(agents.claude / "skills", "greet", "Greets the user")
    _write(agents.claude / "commands" / "tools" / "lint.md", "---\ndescription: Lints the tree\n---\nRun the linter.\n")
    _skill(agents.project / ".claude" / "skills", "review", "Reviews the change")
    plugin = agents.root / "plugin"
    _skill(plugin / "skills", "ship", "Ships the release")
    _write(agents.claude / "settings.json", json.dumps({"enabledPlugins": {"helper@market": True}}))
    installed = {"version": 2, "plugins": {"helper@market": [{"scope": "user", "installPath": str(plugin)}]}}
    _write(agents.claude / "plugins" / "installed_plugins.json", json.dumps(installed))
    _skill(agents.codex / "skills", "tidy", "Tidies the code")
    sid = _running_pane(ctl)
    try:
        _hook(cli, sid, "claude", "SessionStart", agents.payload("claude", "01-SessionStart.json"))
        listed = cli("agent", "commands", "--target", sid)
        assert listed.returncode == 0, listed
        kinds = {entry["name"]: entry["kind"] for entry in listed.json()}
        assert kinds["compact"] == "builtin"
        assert (kinds["greet"], kinds["tools:lint"], kinds["review"], kinds["helper:ship"]) == (
            "skill",
            "command",
            "skill",
            "plugin",
        ), kinds
        described = {entry["name"]: entry["description"] for entry in listed.json()}
        assert described["greet"] == "Greets the user"

        _hook(cli, sid, "codex", "SessionStart", agents.payload("codex", "01-SessionStart.json"))
        codex = {entry["name"]: entry["kind"] for entry in cli("agent", "commands", "--target", sid).json()}
        assert codex["$tidy"] == "skill" and "greet" not in codex, codex
    finally:
        assert_ok(ctl.request("session.close", sid))


def test_a_report_the_record_cannot_take_is_refused(cli, ctl, agents):
    sid = _running_pane(ctl)
    try:
        outside = agents.payload("claude", "01-SessionStart.json", transcript_path=str(agents.root / "elsewhere.jsonl"))
        refused = cli("agent", "hook", "claude", "SessionStart", "--target", sid, input=outside)
        assert refused.returncode == 1 and "transcript path is outside a Claude projects/ tree" in refused.stderr
        unknown = cli("agent", "hook", "claude", "SessionStart", "--target", str(uuid.uuid4()),
                      input=agents.payload("claude", "01-SessionStart.json"))
        assert unknown.returncode == 1 and unknown.stderr, unknown
        not_json = cli("agent", "hook", "codex", "SessionStart", "--target", sid, input="nope")
        assert not_json.returncode == 1 and "hook payload is not a JSON object" in not_json.stderr
        usage = cli("agent", "hook", "gemini", "SessionStart", "--target", sid, input="{}")
        assert usage.returncode == 64 and "unknown agent: gemini (claude|codex)" in usage.stderr
        wait_until(lambda: _no_record(ctl, sid))
    finally:
        assert_ok(ctl.request("session.close", sid))
