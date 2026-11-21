"""Re-run replays and remote lines keep each captured byte, and a csh/tcsh receiver never gets a capture with `!`.

Each Re-run test captures a program a shell started, quits with `launch.relaunch` and reads the argv the pane runs
again from the foreground the tree reports, which is the program's own argv. A csh receiver is a script named `tcsh`
(or `csh`) that runs bash, so the capture can start in it and the pane stays usable when the replay is skipped: the
receiver check reads only the shell's name, and no csh has to be installed."""

import itertools
import json
import shlex
import shutil
import sys
import time

import pytest
from agterm_conformance.command_line import env_has_argv0
from agterm_conformance.match import assert_ok, find_session, program_argv, sessions
from agterm_conformance.plugin import Ctl
from agterm_conformance.shell_profiles import SHELL_PROFILES
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.since("0.42.3"),
    pytest.mark.requires("posix-command-line", reason="the panes run POSIX shells the host installs"),
]

LIVE_ANY_SHELL = pytest.mark.requires("live-sessions", "live-any-shell", any=True, reason="needs Live panes")
# Only the shells this host has: `--must-pass` counts a skipped Live case as not passed (Windows skips them all as
# unsupported, which it excuses).
LIVE_SHELLS = [
    shell for shell in ("bash", "zsh", "fish") if sys.platform == "win32" or shutil.which(shell, path="/usr/bin:/bin")
]
SHELL_START = 60.0
# Longer than a macOS exec chain (the python3 shim, then Python.app), whose first images show a transient argv[0].
SETTLE = 0.5
SLEEP = ("-c", "import time; time.sleep(600)")
# A scalar the line editor or the old quoting broke: an apostrophe a combining mark, an emoji presentation selector or
# a skin-tone modifier follows, as one grapheme with it.
MARKED = ("it'\u0301s; echo INJECTED", "x'\ufe0f", "y'\U0001f3fb", "'\u0301'")
# Bytes a fish replay must keep: fish reads `\\` and a trailing `\` in single quotes its own way.
BACKSLASHES = ("a\\\\b", "trail\\", "\\\\", "\\'", "c:\\path\\")
OTHER = ("it's", "a b", "$HOME", "semi;colon", '"dq"', "")
# Never typed into csh or tcsh: their history expansion reads `!` inside single quotes.
BANGS = ("!needle", "fix!x", "!")
# macOS has bash only at /bin/bash, usrmerged Linux at both paths.
BASH = shutil.which("bash", path="/usr/bin:/bin")
_markers = itertools.count(1)
RAN = object()
LAUNCH_FAILED = "failed to launch the requested command"


def _quoted(word, shell):
    if shell == "fish":
        return "'" + word.replace("\\", "\\\\").replace("'", "\\'") + "'"
    return shlex.quote(word)


def _line(argv, shell):
    return " ".join(_quoted(word, shell) for word in argv) + "\n"


def _python():
    return shutil.which("python3", path="/usr/bin:/bin") or sys.executable


def _program(*arguments):
    return [_python(), *SLEEP, *arguments]


def _args(pane):
    return {"pane": pane}


def _waited(ctl, sid, argv, pane, start=0):
    """Waits until the pane runs `argv` from word `start` on and that whole argv stays the same for SETTLE seconds,
    and gives it."""
    seen, since = None, 0.0

    def runs():
        nonlocal seen, since
        foreground = _foreground(ctl, sid, pane)
        if not isinstance(foreground, list) or foreground[start:] != argv[start:]:
            seen = None
            return None
        now = time.monotonic()
        if foreground != seen:
            seen, since = foreground, now
            return None
        return foreground if now - since >= SETTLE else None

    try:
        return wait_until(runs, timeout=SHELL_START)
    except AssertionError as error:
        text = ctl.request("session.text", sid, all=True, **_args(pane)).raw
        raise AssertionError(f"{error}; runs {_foreground(ctl, sid, pane)!r}; pane text {text}") from None


def _foreground(ctl, sid, pane="left"):
    session = find_session(ctl.tree(), sid)
    return program_argv(session.get("foreground" if pane == "left" else "splitForeground"))


def _ready(ctl, sid, shell="bash", refused=None, **pane):
    """Waits until the pane's shell reads lines: a line typed before its line editor starts can be lost. With
    `refused`, gives RAN as soon as the pane runs that argv instead, since its shell then never reads a line."""
    marker = f"RQ{next(_markers):04d}X"
    line = f"printf '%s%s\\n' {marker[:3]} {marker[3:]}\n"

    def printed():
        if refused is not None and _runs(ctl, sid, refused, pane.get("pane", "left")):
            return RAN
        # before a typed key, which closes a pane whose command exited
        _started(ctl, sid, **pane)
        if not ctl.request("session.type", sid, text=line, **pane).ok:
            return None
        try:
            return ctl.wait_text(sid, rf"(?m)^{marker}\s*$", timeout=2, **pane)
        except AssertionError:
            _started(ctl, sid, **pane)
            return None

    return wait_until(printed, timeout=SHELL_START)


def _started(ctl, sid, **pane):
    """Fails the test, never as an AssertionError an `upstream_bug` allowance would excuse, when the pane's command
    did not start: its shell then never reads a line."""
    response = ctl.request("session.text", sid, all=True, **pane)
    text = response.result.get("text", "") if response.ok else ""
    if LAUNCH_FAILED in text:
        pytest.fail(f"the pane's command did not start: {text}", pytrace=False)


def _runs(ctl, sid, argv, pane):
    foreground = _foreground(ctl, sid, pane)
    # a macOS framework Python runs again as its Python.app, under that name
    return isinstance(foreground, list) and foreground[1:] == argv[1:]


def _new(ctl, **args):
    sid = assert_ok(ctl.request("session.new", **args))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _run(ctl, sid, argv, shell="bash", pane="left"):
    """Starts `argv` from the pane's shell, waits until the pane runs it and gives the argv the pane reports, which a
    later replay must run again. The typed line is a short one that execs `argv` read from a file: what is under test
    is the replay ngterm types, not a long line typed through the editor."""
    _ready(ctl, sid, shell, **_args(pane))
    spec = ctl.instance.root / f"argv-{next(_markers)}.json"
    spec.write_text(json.dumps(argv))
    exec_line = [_python(), "-c", "import json,os,sys; a=json.load(open(sys.argv[1])); os.execv(a[0], a)", str(spec)]
    assert_ok(ctl.request("session.type", sid, text=_line(exec_line, shell), **_args(pane)))
    # a macOS framework Python runs again as its Python.app, under that name
    return _waited(ctl, sid, argv, pane, start=1)


def _relaunch(launch, ctl, *sids):
    assert assert_ok(ctl.request("restore.capture"))["count"] >= len(sids)
    return launch.relaunch(ctl)


def _show(ctl, sid, *panes):
    """Selects `sid`: a relaunch starts the terminals of a session the window shows."""
    assert_ok(ctl.request("session.select", sid))
    ctl.wait_mounted(sid, *(panes or ("left",)), timeout=SHELL_START)


def _replays(ctl, sid, argv, pane="left", start=0):
    _show(ctl, sid, pane)
    _waited(ctl, sid, argv, pane, start)


def _plain(ctl, sid, argv, pane="left"):
    """The pane runs its shell and nothing of the capture: the shell answers, the captured program is not running,
    and no part of its line ran."""
    _show(ctl, sid, pane)
    assert _ready(ctl, sid, refused=argv, **_args(pane)) is not RAN, f"the pane runs the refused capture {argv!r}"
    assert _foreground(ctl, sid, pane) != argv
    text = assert_ok(ctl.request("session.text", sid, all=True, **_args(pane)))["text"]
    assert "needle" not in text and "INJECTED" not in text, text


def _fake_csh(directory, name="tcsh"):
    """A program named `name` in `directory` that runs bash: the receiver check knows a shell only by its name."""
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / name
    path.write_text(f'#!/bin/sh\nexec {shlex.quote(BASH)} "$@"\n')
    path.chmod(0o755)
    return str(path)


def _shell_home(ctl):
    home = ctl.instance.home
    # without a startup file, an interactive zsh starts its new-user menu instead of a prompt
    (home / ".zshrc").write_text("PS1='$ '\n")


@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
@pytest.mark.parametrize("shell", ["bash", "zsh", "fish"])
def test_rerun_replays_the_same_bytes_into_each_shell(launch, shell):
    path = shutil.which(shell, path="/usr/bin:/bin")
    if path is None:
        pytest.skip(f"no {shell} on this machine")
    ctl = launch(settings={"restoreMode": "rerun"}, pane_shell=path)
    _shell_home(ctl)
    sid = _new(ctl)
    argv = _run(ctl, sid, _program(*BANGS, *MARKED, *BACKSLASHES, *OTHER), shell)
    ctl = _relaunch(launch, ctl, sid)
    _replays(ctl, sid, argv)
    text = assert_ok(ctl.request("session.text", sid, all=True))["text"]
    assert "INJECTED\n" not in text.replace("\r", "")


FIRST_WORD_RECEIVER = pytest.mark.upstream_bug(
    "csh-receiver-is-the-first-word",
    reason="upstream takes the receiving shell from the first whitespace word of ghostty's command "
    "(ZmxSupport.commandProgram(fromCommand:), agtermApp.swift launchSeedPolicy), so behind env or a quoted path "
    "with a space CommandRestore.shouldRestore types a `!` capture into tcsh",
)
CSH_RECEIVERS = {
    "path": "{tcsh}",
    "csh": "{csh}",
    "login-flag": "{tcsh} -l",
    "env": "/usr/bin/env {tcsh}",
    "env-quoted-assignment": "/usr/bin/env NOTE='use bash' {tcsh}",
    "env-split-string": "/usr/bin/env -S 'NOTE=\"use bash\" {tcsh} -l'",
    "env-argv0": "/usr/bin/env --argv0 bash {tcsh}",
    "env-unset": "/usr/bin/env -u bash {tcsh}",
    "env-nbsp": "/usr/bin/env NOTE=1\u00a0bash {tcsh}",
    "comment": "{tcsh} # bash",
    "space-and-x64-path": "'{spaced}'",
    # upstream on macOS runs the value as `exec -l <value>`: a receiver after the `;` starts only behind a bare exec.
    # ngterm refuses it at the `>`; shell_dialect's unit tests cover the refusal at a `;`.
    "separator": "2>&2; {tcsh}",
}
FIRST_WORD_MISSES = {
    "env", "env-quoted-assignment", "env-split-string", "env-argv0", "env-unset", "env-nbsp", "space-and-x64-path",
    "separator",
}
# A value whose receiver ngterm cannot tell (another command may follow) gets no replay at all.
UNCERTAIN = {"separator"}


def _receiver(name):
    marks = [FIRST_WORD_RECEIVER] if name in FIRST_WORD_MISSES else []
    if name == "env-argv0" and not env_has_argv0():
        marks.append(pytest.mark.skip(reason="this /usr/bin/env has no --argv0 (macOS's has none)"))
    return pytest.param(name, marks=marks)


@pytest.mark.covers("behavior:restore.rerun-replays-captured-command", "behavior:restore.rerun-types-pinned-command")
@pytest.mark.parametrize(
    "receiver",
    [_receiver(name) for name in CSH_RECEIVERS],
)
def test_rerun_starts_a_plain_csh_for_a_capture_with_a_bang(launch, tmp_path, receiver):
    shells = tmp_path / "shells"
    command = CSH_RECEIVERS[receiver].format(
        tcsh=_fake_csh(shells),
        csh=_fake_csh(shells, "csh"),
        spaced=_fake_csh(tmp_path / "my shells (x64)"),
    )
    ctl = launch(settings={"restoreMode": "rerun"}, pane_shell=command)
    refused, replayed, pinned = _new(ctl), _new(ctl), _new(ctl)
    banged = _run(ctl, refused, _program("!needle", "it's"))
    safe = _run(ctl, replayed, _program("plain", "it'\u0301s"))
    _run(ctl, pinned, _program("!needle", "it's"))
    marker = f"PIN{next(_markers):04d}"
    pin = f"printf '%s%s|%s|\\n' {marker[:3]} {marker[3:]} '!'"
    assert_ok(ctl.request("session.restore", pinned, mode="set", command=pin))
    ctl = _relaunch(launch, ctl, refused, replayed, pinned)
    # first, so the reference fails on the receiver its allowance names
    _plain(ctl, refused, banged)
    if receiver in UNCERTAIN:
        _plain(ctl, replayed, safe)
    else:
        _replays(ctl, replayed, safe)
    _show(ctl, pinned)
    ctl.wait_text(pinned, rf"(?m)^{marker}\|!\|\s*$", timeout=SHELL_START)


@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
def test_a_refused_capture_still_drops_the_older_creation_command(launch, tmp_path):
    ctl = launch(settings={"restoreMode": "rerun"}, pane_shell=_fake_csh(tmp_path / "shells"))
    sid = _new(ctl, command=f"/usr/bin/env QA_CREATED=yes {shlex.quote(BASH)}")
    banged = _run(ctl, sid, _program("!needle"))
    ctl = _relaunch(launch, ctl, sid)
    _plain(ctl, sid, banged)
    marker = f"CR{next(_markers):04d}X"
    line = f"printf '%s%s|%s|\\n' {marker[:3]} {marker[3:]} \"${{QA_CREATED-none}}\"\n"
    assert_ok(ctl.request("session.type", sid, text=line, pane="left"))
    ctl.wait_text(sid, rf"(?m)^{marker}\|none\|\s*$", pane="left")


@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
def test_a_split_pane_gets_the_same_check(launch, tmp_path):
    ctl = launch(settings={"restoreMode": "rerun"}, pane_shell=_fake_csh(tmp_path / "shells"))
    sid = _new(ctl)
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")
    safe = _run(ctl, sid, _program("y'\U0001f3fb"))
    banged = _run(ctl, sid, _program("!needle"), pane="right")
    ctl = _relaunch(launch, ctl, sid)
    _replays(ctl, sid, safe)
    _plain(ctl, sid, banged, "right")


@SHELL_PROFILES
@pytest.mark.covers("behavior:restore.rerun-replays-captured-command")
@pytest.mark.parametrize("profiled", ["tcsh", "bash"])
def test_the_pane_profile_names_the_receiver_before_the_launch_shell(launch, tmp_path, profiled):
    """A `tcsh` profile refuses under a bash `command`; a `bash` profile, in a directory named for tcsh, replays under
    a tcsh `command`."""
    tcsh = _fake_csh(tmp_path / "my shells (x64)")
    bash = _fake_csh(tmp_path / "tcsh tools", "bash")
    executable, command = (tcsh, BASH) if profiled == "tcsh" else (bash, tcsh)
    profile = {"id": "qa-receiver", "name": "QA Receiver", "executable": executable, "args": [], "env": {}}
    settings = {"restoreMode": "rerun", "shellProfiles": [profile], "defaultShellProfile": profile}
    ctl = launch(settings=settings, pane_shell=command)
    sid = _new(ctl)
    banged = _run(ctl, sid, _program("!needle", "x'\ufe0f"))
    ctl = _relaunch(launch, ctl, sid)
    if profiled == "tcsh":
        _plain(ctl, sid, banged)
    else:
        _replays(ctl, sid, banged)


@LIVE_ANY_SHELL
@pytest.mark.covers("session.new")
@pytest.mark.parametrize("shell", LIVE_SHELLS)
def test_a_live_creation_command_keeps_apostrophes_beside_marks(launch, shell):
    path = shutil.which(shell, path="/usr/bin:/bin")
    ctl = launch(settings={"restoreMode": "live"}, pane_shell=path)
    _shell_home(ctl)
    argv = _program(*MARKED, *OTHER)
    sid = _new(ctl, command=_line(argv, "sh").rstrip("\n"))
    _replays(ctl, sid, argv, start=1)
    assert find_session(ctl.tree(), sid).get(ctl.vocabulary.backed_key) is True


@pytest.mark.requires("live-sessions", reason="the origin session must be Live")
@pytest.mark.covers("live.attach", "session.companion.new")
def test_a_remote_companion_line_keeps_apostrophes_beside_marks(remote_origin, remote_viewer):
    """A companion asked of an attached session runs on the origin through ssh: its name and command arrive whole."""
    origin = Ctl(remote_origin.instance)
    parent = assert_ok(origin.request("session.new", name="origin-parent"))["id"]
    local = None
    try:
        origin.wait_mounted(parent, "left")
        attach = remote_viewer.vocabulary.command("live.attach")
        local = assert_ok(remote_viewer.request(attach, parent, host=remote_origin.host))["id"]
        argv = _program(*MARKED, *OTHER)
        name = "it'\u0301s; echo INJECTED x'\ufe0f y'\U0001f3fb"
        command = _line(argv, "sh").rstrip("\n")
        assert_ok(remote_viewer.request("session.companion.new", local, command=command, name=name))

        def companion(tree):
            rows = [row for row in sessions(tree) if row.get("parent") == parent]
            return rows[0] if rows else None

        made = origin.wait_tree(companion, timeout=SHELL_START)
        row = companion(made)
        assert row["name"] == name
        origin.wait_mounted(row["id"], "left", timeout=SHELL_START)
        _waited(origin, row["id"], argv, "left", start=1)
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        origin.request("session.close", parent)
