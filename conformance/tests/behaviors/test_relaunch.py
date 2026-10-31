"""What a quit and a relaunch on the same state directory bring back. Each test starts its own instance through the
`launch` fixture and relaunches it with `launch.relaunch`, which SIGTERMs the launched pid and starts the same binary
on the same root."""

import os
import re
import shutil
import sys
import uuid

import pytest
from agterm_conformance.match import assert_absent, assert_ok, find_session, program_argv, workspaces

WINDOWS = sys.platform == "win32"
WINDOWS_SHELLS = pytest.mark.requires(
    "windows-shells", reason="Rust-only, Windows: panes run PowerShell or cmd.exe, which read a replay their own way"
)
# How long a pane's PowerShell may take to start on a loaded Windows: its first start in a fresh profile builds caches.
SHELL_START = 60.0
# A program that runs until it is ended, and arguments each shell reads specially: Python ignores what follows its
# `-c` script, which keeps them in the argv the pane's foreground reports.
PROGRAM_ARGUMENTS = (
    "-c",
    "import time; time.sleep(600)",
    "a b",
    "it's",
    'say "hi"',
    "",
    "$HOME",
    "semi;colon",
    "x&y|z",
    "100%",
    "^caret",
    "(paren)",
)
LIVE = pytest.mark.requires(
    "live-sessions", reason="needs panes in zmx daemons, which upstream creates only for a zsh or fish login shell"
)


def _new(ctl, **args):
    sid = assert_ok(ctl.request("session.new", **args))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _layout(tree):
    """What a relaunch must keep: the workspace/session model, without the live process state."""
    kept = ("id", "name", "flagged", "context", "split", "hasSplit", "splitAxis", "restoreCommand", "active")
    return [
        {
            "id": workspace["id"],
            "name": workspace["name"],
            "collapsed": workspace.get("collapsed", False),
            "sessions": [{key: session.get(key) for key in kept} for session in workspace["sessions"]],
        }
        for workspace in workspaces(tree)
    ]


def _marker():
    return f"RL{uuid.uuid4().hex[:8].upper()}"


@pytest.mark.covers("behavior:restore.relaunch-keeps-layout")
def test_relaunch_keeps_workspaces_sessions_and_their_metadata(launch):
    ctl = launch()
    window = assert_ok(ctl.request("window.list"))["windows"][0]
    assert_ok(ctl.request("window.rename", window["id"], name="kept window"))
    alpha = assert_ok(ctl.request("workspace.new", name="alpha"))["id"]
    first = _new(ctl, workspace=alpha, name="first")
    assert_ok(ctl.request("session.flag", first, mode="on"))
    assert_ok(ctl.request("session.context", first, mode="set", text="what it is for"))
    assert_ok(ctl.request("session.split", first, mode="on", axis="horizontal"))
    ctl.wait_mounted(first, "right")
    second = _new(ctl, workspace=alpha, name="second")
    assert_ok(ctl.request("session.split", second, mode="on"))
    ctl.wait_mounted(second, "right")
    assert_ok(ctl.request("session.split", second, mode="off"))
    assert_ok(ctl.request("session.restore", second, mode="none"))
    # Before a structural change: upstream saves a selection 0.3s later, and SIGTERM ends it with no final save.
    assert_ok(ctl.request("session.select", first))
    folded = assert_ok(ctl.request("workspace.new", name="folded", collapsed=True))["id"]
    before = ctl.tree()
    assert folded in {node["id"] for node in workspaces(before)}
    assert find_session(before, first)["active"] is True

    ctl = launch.relaunch(ctl)
    after = ctl.wait_tree(lambda tree: find_session(tree, first)["realized"] is True)
    assert _layout(after) == _layout(before)
    assert find_session(after, first)["active"] is True
    (restored,) = assert_ok(ctl.request("window.list"))["windows"]
    assert (restored["id"], restored["name"]) == (window["id"], "kept window")


@pytest.mark.covers("behavior:restore.relaunch-keeps-layout")
def test_relaunch_forgets_what_is_not_persisted(launch):
    ctl = launch()
    sid = _new(ctl)
    assert_ok(ctl.request("session.select", sid))
    assert_ok(ctl.request("session.status", sid, status="blocked"))
    assert_ok(ctl.request("session.scratch", sid, mode="on"))
    ctl.wait_mounted(sid, "scratch")
    # After the select, which clears unseen.
    assert_ok(ctl.request("notify", sid, body="relaunch"))
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("unseen", 0) >= 1)

    ctl = launch.relaunch(ctl)
    node = find_session(ctl.wait_tree(lambda tree: find_session(tree, sid)["realized"] is True), sid)
    assert node["scratch"] is False
    assert_absent(node, "status", "statusChangedAt", "unseen")


@pytest.mark.since("0.39.1")
@pytest.mark.covers("behavior:restore.relaunch-keeps-layout", "field:tree.session.lastSelectedAt")
def test_last_selected_at_survives_a_relaunch(launch):
    ctl = launch()
    first = _new(ctl)
    second = _new(ctl)
    assert_ok(ctl.request("session.select", first))
    assert_ok(ctl.request("session.select", second))
    # A structural change after the selections: upstream saves a selection 0.3s later, and SIGTERM ends it unsaved.
    assert_ok(ctl.request("workspace.new", name="saved", collapsed=True))
    before = ctl.tree()
    stamps = {sid: find_session(before, sid)["lastSelectedAt"] for sid in (first, second)}

    ctl = launch.relaunch(ctl)
    # The restore reselects `second` without restamping either one; approx for the epoch's float round trip.
    after = ctl.wait_tree(lambda tree: find_session(tree, second)["realized"] is True)
    assert find_session(after, second)["active"] is True
    for sid, stamp in stamps.items():
        assert find_session(after, sid)["lastSelectedAt"] == pytest.approx(stamp, abs=1e-3), sid


@pytest.mark.covers("behavior:restore.rerun-types-pinned-command")
@pytest.mark.requires("posix-shell", reason="the pinned line runs in /bin/sh")
def test_rerun_types_the_pinned_line_at_the_next_launch(launch):
    ctl = launch(settings={"restoreMode": "rerun"})
    sid = _new(ctl)
    marker = _marker()
    # The pin prints the marker split in two, so only its output matches.
    assert_ok(ctl.request("session.restore", sid, mode="set", command=f"printf '%s%s\\n' {marker[:4]} {marker[4:]}"))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    # Not anchored at the line's start: the line is typed before the shell's first prompt, and a /bin/sh without a
    # line editor (dash) does not echo it again after that prompt, so the output follows the prompt on its line.
    ctl.wait_text(sid, rf"(?m){marker}$")
    # The pin persists for the launches after this one too.
    assert find_session(ctl.tree(), sid)["restoreCommand"].endswith(marker[4:])


@pytest.mark.covers("behavior:restore.fresh-shells-ignore-pins")
@pytest.mark.requires("posix-shell", reason="the pinned line and the probe run in /bin/sh")
def test_fresh_shells_do_not_run_the_pin(launch):
    ctl = launch()
    assert assert_ok(ctl.request("restore.mode"))["restore"]["configured"] == "none"
    sid = _new(ctl)
    pinned, probe = _marker(), _marker()
    assert_ok(ctl.request("session.restore", sid, mode="set", command=f"printf '%s%s\\n' {pinned[:4]} {pinned[4:]}"))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    # A line typed after the launch shows up; had the pin run, its output would sit above it.
    assert_ok(ctl.request("session.type", sid, text=f"printf '%s%s\\n' {probe[:4]} {probe[4:]}\n"))
    ctl.wait_text(sid, rf"(?m)^{probe}$")
    assert not re.search(rf"(?m)^{pinned}$", assert_ok(ctl.request("session.text", sid, all=True))["text"])


@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
@pytest.mark.requires("posix-shell", reason="the captured program is started from /bin/sh")
def test_rerun_replays_the_captured_foreground_program(launch):
    ctl = launch(settings={"restoreMode": "rerun"})
    sid = _new(ctl)
    # A job of its own: upstream captures the foreground group's leader, and an exec'd program would share the
    # shell's group, led by macOS's setuid `login`. The shell passes the pane's hangup on to it at the quit.
    assert_ok(ctl.request("session.type", sid, text="sleep 600\n"))
    ctl.wait_tree(lambda tree: program_argv(find_session(tree, sid).get("foreground")) == ["sleep", "600"])
    assert assert_ok(ctl.request("restore.capture"))["count"] >= 1
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: program_argv(find_session(tree, sid).get("foreground")) == ["sleep", "600"])


def _windows_shell(name):
    """The full path of the Windows shell `name` (`powershell` 5.1 and `cmd` from the system directory, `pwsh` 7 from
    `PATH`), or None when this machine has none."""
    system = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32")
    path = {
        "powershell": os.path.join(system, "WindowsPowerShell", "v1.0", "powershell.exe"),
        "cmd": os.path.join(system, "cmd.exe"),
    }.get(name) or shutil.which("pwsh")
    return path if path and os.path.isfile(path) else None


def _crt_quoted(argument, always=False):
    """`argument` as one word of a command line the C runtime splits."""
    if not always and argument and not re.search(r'[ \t"]', argument):
        return argument
    quoted, backslashes = '"', 0
    for character in argument:
        if character == "\\":
            backslashes += 1
            continue
        quoted += "\\" * (backslashes * 2 + 1 if character == '"' else backslashes) + character
        backslashes = 0
    return quoted + "\\" * (backslashes * 2) + '"'


def _program_line(shell, argv):
    """The line that starts `argv` at `shell`'s prompt, as a user would type it: in PowerShell under the legacy argument
    passing every version has, each word already quoted for the C runtime; in `cmd.exe` every word in quotes."""
    if shell == "cmd":
        return " ".join(_crt_quoted(word, always=True) for word in argv) + "\n"
    words = " ".join("'" + _crt_quoted(word).replace("'", "''") + "'" for word in argv[1:])
    return f"& {{ $PSNativeCommandArgumentPassing = 'Legacy' ; & '{argv[0]}' {words} }}\n"


def _launch_in(launch, shell):
    path = _windows_shell(shell)
    if path is None:
        pytest.skip(f"no {shell} on this machine")
    return launch(settings={"restoreMode": "rerun"}, pane_shell=path)


def _wait_prompt(ctl, sid, shell):
    """Wait for `shell`'s first prompt: what is typed before PowerShell's line editor reads keys can be lost."""
    prompt = r"(?m)^[A-Z]:\\[^\n]*>\s*\Z" if shell == "cmd" else r"(?m)^PS [^\n]*>\s*\Z"
    ctl.wait_text(sid, prompt, timeout=SHELL_START)


@WINDOWS_SHELLS
@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
@pytest.mark.parametrize("shell", ["powershell", "pwsh", "cmd"])
def test_rerun_replays_the_captured_program_into_a_windows_shell(launch, shell):
    ctl = _launch_in(launch, shell)
    sid = _new(ctl)
    _wait_prompt(ctl, sid, shell)
    argv = [sys.executable, *PROGRAM_ARGUMENTS]
    assert_ok(ctl.request("session.type", sid, text=_program_line(shell, argv)))

    def running(tree):
        foreground = program_argv(find_session(tree, sid).get("foreground"))
        assert foreground and foreground[1:] == list(PROGRAM_ARGUMENTS), foreground
        return foreground

    captured = running(ctl.wait_tree(running, timeout=SHELL_START))
    assert assert_ok(ctl.request("restore.capture"))["count"] >= 1
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    # the replay is typed as the shell starts, quoted for it: the same argv runs again
    ctl.wait_tree(lambda tree: program_argv(find_session(tree, sid).get("foreground")) == captured, timeout=SHELL_START)


@WINDOWS_SHELLS
@pytest.mark.covers("behavior:restore.rerun-types-pinned-command")
@pytest.mark.parametrize("shell", ["powershell", "pwsh", "cmd"])
def test_rerun_types_the_pinned_line_into_a_windows_shell(launch, shell):
    ctl = _launch_in(launch, shell)
    sid = _new(ctl)
    marker = _marker()
    # The pin prints the marker joined from two halves, so only its output matches.
    line = f"echo {marker[:4]}^{marker[4:]}" if shell == "cmd" else f"Write-Output ('{marker[:4]}' + '{marker[4:]}')"
    assert_ok(ctl.request("session.restore", sid, mode="set", command=line))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    ctl.wait_text(sid, rf"(?m)^{marker}\s*$", timeout=SHELL_START)


@LIVE
@pytest.mark.covers("behavior:restore.live-session-survives-relaunch")
def test_live_session_survives_the_relaunch(launch):
    ctl = launch(settings={"restoreMode": "live"})
    sid = _new(ctl)
    vocabulary = ctl.vocabulary
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(vocabulary.backed_key) is True)
    marker = _marker()
    assert_ok(ctl.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:4]} {marker[4:]}\n"))
    ctl.wait_text(sid, rf"(?m)^{marker}$")
    rows = assert_ok(ctl.request(vocabulary.command("zmx.list")))[vocabulary.result_key]["entries"]
    (before,) = [row for row in rows if row.get("sessionID") == sid and row["pane"] == "left"]

    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(vocabulary.backed_key) is True)

    def same_daemon(response):
        rows = assert_ok(response)[vocabulary.result_key]["entries"]
        (row,) = [row for row in rows if row.get("sessionID") == sid]
        assert (row["daemon"], row["observation"]) == (before["daemon"], "running"), row
        return row

    after = ctl.wait_response(vocabulary.command("zmx.list"), None, same_daemon)
    (row,) = [row for row in after.result[vocabulary.result_key]["entries"] if row.get("sessionID") == sid]
    # The same shell, not a new one: its leader pid and its screen came through the quit.
    assert row["leaderPID"] == before["leaderPID"]
    ctl.wait_text(sid, rf"(?m)^{marker}$", all=True)


@LIVE
@pytest.mark.covers("behavior:restore.live-session-survives-relaunch", "zmx.tree", "field:result.remote")
def test_a_live_session_with_a_hidden_split_stays_attachable_across_the_relaunch(launch):
    """A split hidden across the relaunch builds no terminal until shown, yet its daemon runs on claimed: the session
    still reads as backed, pane and all, and stays on offer to another machine (upstream #762)."""
    ctl = launch(settings={"restoreMode": "live"})
    sid = _new(ctl)
    vocabulary = ctl.vocabulary
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")

    def both_running(response):
        rows = assert_ok(response)[vocabulary.result_key]["entries"]
        observed = {row["pane"]: row["observation"] for row in rows if row.get("sessionID") == sid}
        return observed == {"left": "running", "right": "running"}

    ctl.wait_response(vocabulary.command("zmx.list"), None, both_running)
    assert_ok(ctl.request("session.split", sid, mode="off"))
    ctl.wait_tree(lambda tree: find_session(tree, sid)["split"] is False)

    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")

    def backed(tree):
        node = find_session(tree, sid)
        (right,) = [surface for surface in node.get("surfaces", []) if surface["kind"] == "right"]
        return node.get(vocabulary.backed_key) is True and right.get(vocabulary.backed_key) is True

    ctl.wait_tree(backed)
    assert find_session(ctl.tree(), sid)["hasSplit"] is True

    def row(response):
        sessions = assert_ok(response)["remote"]["sessions"]
        return next((session for session in sessions if session["id"] == sid), None)

    offered = row(ctl.wait_response(vocabulary.command("zmx.tree"), None, lambda response: row(response) is not None))
    assert [pane["pane"] for pane in offered["panes"]] == ["left", "right"]
    ctl.wait_response(vocabulary.command("zmx.list"), None, both_running)
    assert_ok(ctl.request("session.close", sid))
