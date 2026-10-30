"""Live sessions in the host's own login shell: a Rust-only extension (capability `live-any-shell`).

Upstream wraps a pane in zmx only for a zsh or fish login shell; the Rust build also wraps bash (and PowerShell),
starting each with its shell integration. The reference skips these tests as unsupported; the Rust CI runs them
under zsh, bash and fish login shells, and its Windows job under PowerShell, with `--must-pass live-any-shell`.

The host picks the shell (the password database's), so every line typed here is valid in zsh, bash and fish alike. On
Windows the login shell is PowerShell (`ngterm-session`'s `windows_shell.rs`), which gets lines of its own."""

import ntpath
import sys
import uuid

import pytest
from agterm_conformance.match import assert_ok, find_session

LIVE_ANY_SHELL = pytest.mark.requires(
    "live-any-shell",
    reason="a Rust-only extension: upstream wraps a pane in zmx only for a zsh or fish login shell",
)


WINDOWS = sys.platform == "win32"
# A program that runs until interrupted, and its name in the pane's foreground. PowerShell's `sleep` is its own
# Start-Sleep, which starts no process.
LONG_COMMAND, LONG_PROGRAM = ("ping -n 600 127.0.0.1", "ping") if WINDOWS else ("sleep 600", "sleep")


def _marker():
    return f"LA{uuid.uuid4().hex[:8].upper()}"


def _print_line(marker, posix_variable=None, powershell_variable=None):
    """A line printing `marker` (then `|<the variable>|` when one is named) on a line of its own, typed in two halves so
    the echoed line never matches."""
    head, tail = marker[:4], marker[4:]
    if WINDOWS:
        variable = f" + '|' + ${powershell_variable} + '|'" if powershell_variable else ""
        return f"Write-Output ('{head}' + '{tail}'{variable})\n"
    if posix_variable:
        return f"printf '%s%s|%s|\\n' {head} {tail} \"${posix_variable}\"\n"
    return f"printf '%s%s\\n' {head} {tail}\n"


def _runs(foreground, program):
    """Whether the foreground argv is `program`'s: on Windows by its image's basename, any case, without `.exe`."""
    if not foreground:
        return False
    name = ntpath.basename(foreground[0]).lower().removesuffix(".exe") if WINDOWS else foreground[0]
    return name == program


def _daemon_row(ctl, sid):
    rows = assert_ok(ctl.request(ctl.vocabulary.command("zmx.list")))[ctl.vocabulary.result_key]["entries"]
    (row,) = [row for row in rows if row.get("sessionID") == sid and row["pane"] == "left"]
    return row


@LIVE_ANY_SHELL
@pytest.mark.covers("behavior:restore.live-session-survives-relaunch", "field:tree.session.cwd")
def test_a_live_pane_in_the_login_shell_reports_its_cwd_and_survives_the_relaunch(launch):
    ctl = launch(settings={"restoreMode": "live"})
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)
    # The shell integration reports the directory through OSC 7, which the pane's cwd follows.
    directory = ctl.instance.root / "live-cwd"
    directory.mkdir()
    assert_ok(ctl.request("session.type", sid, text=f"cd '{directory}'\n"))
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("cwd") == str(directory))
    marker = _marker()
    assert_ok(ctl.request("session.type", sid, text=_print_line(marker)))
    ctl.wait_text(sid, rf"(?m)^{marker}$")
    before = _daemon_row(ctl, sid)

    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)

    def same_daemon(response):
        rows = assert_ok(response)[ctl.vocabulary.result_key]["entries"]
        (row,) = [row for row in rows if row.get("sessionID") == sid]
        assert (row["daemon"], row["observation"]) == (before["daemon"], "running"), row
        return row

    ctl.wait_response(ctl.vocabulary.command("zmx.list"), None, same_daemon)
    # The same shell, still in its directory: its leader pid, its screen and its cwd came through the quit.
    assert _daemon_row(ctl, sid)["leaderPID"] == before["leaderPID"]
    ctl.wait_text(sid, rf"(?m)^{marker}$", all=True)
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("cwd") == str(directory))


@LIVE_ANY_SHELL
@pytest.mark.covers("session.new", "session.type")
def test_an_interrupted_live_command_leaves_the_login_shell_prompt(launch):
    ctl = launch(settings={"restoreMode": "live"})
    sid = assert_ok(ctl.request("session.new", command=LONG_COMMAND))["id"]
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)

    def sleeping(tree):
        foreground = find_session(tree, sid).get("foreground") or []
        assert _runs(foreground, LONG_PROGRAM), foreground
        return True

    ctl.wait_tree(sleeping)
    before = _daemon_row(ctl, sid)
    # Ctrl+C ends the command, not the pane: the replay goes on into the login shell (#773)
    assert_ok(ctl.request("session.type", sid, text="\x03"))
    ctl.wait_tree(lambda tree: not _runs(find_session(tree, sid).get("foreground"), LONG_PROGRAM))
    if WINDOWS:
        # the console drops what is typed before PowerShell's prompt is back from the Ctrl+C
        ctl.wait_text(sid, r"(?m)Control-C\s*\nPS [^\n]*> *$")
    marker = _marker()
    assert_ok(ctl.request("session.type", sid, text=_print_line(marker)))
    ctl.wait_text(sid, rf"(?m)^{marker}$")
    after = _daemon_row(ctl, sid)
    assert (after["daemon"], after["observation"]) == (before["daemon"], "running"), after


@LIVE_ANY_SHELL
@pytest.mark.covers("behavior:restore.live-session-survives-relaunch", "config.reload")
def test_a_live_pane_keeps_the_configs_xdg_data_dirs(launch):
    # A ghostty config `env` entry reaches a Live pane's shell. Under fish its integration rides XDG_DATA_DIRS: the
    # session starts with the integration's root in front of the config's value, which the integration then removes
    # again, so fish sees the config's value with the integration active (#761); zsh and bash get it as it is.
    ctl = launch(settings={"restoreMode": "live"})
    config = ctl.instance.state_dir / "config" / "ghostty.conf"
    config.write_text(config.read_text() + "env = XDG_DATA_DIRS=/conf/share\n")
    assert assert_ok(ctl.request("config.reload")) == {"count": 0}
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)
    # the integration is active: the directory comes through OSC 7
    directory = ctl.instance.root / "xdg-cwd"
    directory.mkdir()
    assert_ok(ctl.request("session.type", sid, text=f"cd '{directory}'\n"))
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("cwd") == str(directory))
    _assert_the_configs_data_dirs(ctl, sid)
    before = _daemon_row(ctl, sid)

    # the same shell after a relaunch, its environment as it was
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(sid, "left")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(ctl.vocabulary.backed_key) is True)

    def same_shell(response):
        rows = assert_ok(response)[ctl.vocabulary.result_key]["entries"]
        (row,) = [row for row in rows if row.get("sessionID") == sid]
        assert (row["daemon"], row["observation"], row["leaderPID"]) == (
            before["daemon"],
            "running",
            before["leaderPID"],
        ), row
        return row

    ctl.wait_response(ctl.vocabulary.command("zmx.list"), None, same_shell)
    _assert_the_configs_data_dirs(ctl, sid)


def _assert_the_configs_data_dirs(ctl, sid):
    """The pane's shell has the config's XDG_DATA_DIRS last (a login profile may put its own in front, as flatpak's
    does) and no shell-integration directory left in it."""
    marker = _marker()
    line = _print_line(marker, posix_variable="XDG_DATA_DIRS", powershell_variable="env:XDG_DATA_DIRS")
    assert_ok(ctl.request("session.type", sid, text=line))
    entries = ctl.wait_text(sid, rf"(?m)^{marker}\|([^|\n]*)\|$").group(1).split(":")
    assert entries[-1] == "/conf/share", entries
    assert not any(entry.rstrip("/").endswith("shell-integration") for entry in entries), entries
