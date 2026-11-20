"""Keys pressed into a pane whose program pushed the Kitty keyboard flags (`CSI > 1 u`) and `modifyOtherKeys` 2
(`CSI > 4 ; 2 m`), as Claude Code does: Escape, Shift+Tab, Ctrl+C and Alt+V each reach the program.

The pane's program is a probe, the harness's own Python, that pushes both modes, prints `keys-ready` and then a
`key <name> <form>` line for each key it reads. On Linux and macOS it reads the pane's bytes in raw mode: the form is
`kitty` for a Kitty `CSI u` encoding and `legacy` for the legacy one. The terminal sends what the program asked for, the
Kitty encodings (Ghostty's key encoder, upstream's too). On Windows the pseudoconsole stands between the pane and the
program: it reads the pane's bytes and gives the program console key events (`ReadConsoleInputW`, the form `console`),
and its input parser drops the Kitty and `modifyOtherKeys` encodings, so there a pane always sends the legacy ones
(`PaneConfig::legacy_keys`). On the Mac, Ghostty's `macos-option-as-alt` decides whether Option is Alt, and left unset
it changes with the keyboard layout (`test_pane_input`), so Alt+V is not pressed there. The command line is the
probe's words alone, unquoted, as `kitty_clipboard.probe_command`'s.

The same keys reach the probe in a session that a viewer attached from a POSIX origin (`remote_origin`, the staged
`ngterm-headless`): the viewer sends the Kitty encodings there too."""

import os
import re
import sys

import pytest
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.plugin import Ctl

WINDOWS = sys.platform == "win32"
CHORDS = ("escape", "shift+tab", "ctrl+c") + (() if sys.platform == "darwin" else ("alt+v",))
PROBE = r"""
import os, re

PUSH = b"\x1b[>1u\x1b[>4;2m"


def say(text):
    os.write(1, (text + "\r\n").encode())


def name(key, ctrl, alt, shift):
    return "".join(("ctrl+" if ctrl else "", "alt+" if alt else "", "shift+" if shift else "", key))


if os.name == "nt":
    import ctypes
    from ctypes import wintypes

    class Key(ctypes.Structure):
        _fields_ = [("down", wintypes.BOOL), ("repeat", wintypes.WORD), ("vk", wintypes.WORD),
                    ("scan", wintypes.WORD), ("char", wintypes.WCHAR), ("state", wintypes.DWORD)]

    class Record(ctypes.Structure):  # INPUT_RECORD: its union is no larger than KEY_EVENT_RECORD
        _fields_ = [("type", wintypes.WORD), ("key", Key)]

    kernel32 = ctypes.windll.kernel32
    out, inp = kernel32.GetStdHandle(-11), kernel32.GetStdHandle(-10)
    mode = wintypes.DWORD()
    if kernel32.GetConsoleMode(out, ctypes.byref(mode)):
        kernel32.SetConsoleMode(out, mode.value | 0x1 | 0x4)
    # no line input, no echo, no processed input: Ctrl+C is a key event, not a signal
    kernel32.SetConsoleMode(inp, 0)
    os.write(1, PUSH)
    say("keys-ready")
    record, count = Record(), wintypes.DWORD()
    keys = {0x1B: "escape", 0x09: "tab"}
    while kernel32.ReadConsoleInputW(inp, ctypes.byref(record), 1, ctypes.byref(count)):
        key = record.key
        # key events only, their presses, and not the modifier keys themselves (Shift, Ctrl, Alt, Windows)
        if record.type != 1 or not key.down or key.vk in (0x10, 0x11, 0x12, 0x5B, 0x5C):
            continue
        label = keys.get(key.vk, chr(key.vk).lower() if 0x30 <= key.vk <= 0x5A else f"vk{key.vk:02x}")
        state = key.state
        say(f"key {name(label, state & 0xC, state & 0x3, state & 0x10)} console")
else:
    import tty

    tty.setraw(0)
    os.write(1, PUSH)
    say("keys-ready")
    legacy = {b"\x1b": "escape", b"\x1b[Z": "shift+tab", b"\x03": "ctrl+c", b"\x1bv": "alt+v"}
    while data := os.read(0, 64):
        if data in legacy:
            say(f"key {legacy[data]} legacy")
        elif match := re.fullmatch(rb"\x1b\[(\d+)(?:;(\d+))?u", data):
            code, mods = int(match[1]), int(match[2] or 1) - 1
            label = {27: "escape", 9: "tab"}.get(code, chr(code))
            say(f"key {name(label, mods & 4, mods & 2, mods & 1)} kitty")
        else:
            say(f"key unknown {data.hex()}")
"""


@pytest.mark.covers("behavior:pane.typing")
def test_keys_reach_a_program_that_pushed_the_kitty_keyboard_flags(ctl, ui, tmp_path):
    probe = tmp_path / "keys_probe.py"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe)]
    if any(" " in word for word in words):
        pytest.fail(f"the probe's words go unquoted to every pane shell, so none may hold a space: {words}")
    sid = assert_ok(ctl.request("session.new", command=" ".join(words)))["id"]
    ctl.wait_mounted(sid, "left")
    try:
        ctl.wait_text(sid, "keys-ready")
        forms = {}
        for chord in CHORDS:
            ui.press(chord)
            forms[chord] = ctl.wait_text(sid, rf"key {re.escape(chord)} (\w+)")[1]
        expected = "console" if WINDOWS else "kitty"
        assert forms == dict.fromkeys(CHORDS, expected), assert_ok(ctl.request("session.text", sid, all=True))["text"]
    finally:
        ctl.request("session.close", sid)


@pytest.mark.requires(
    "posix-command-line",
    reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
)
@pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell")
@pytest.mark.parametrize("remote_origin", ["headless"], indirect=True)
@pytest.mark.covers("behavior:pane.typing")
def test_keys_reach_a_program_in_a_session_attached_from_a_posix_origin(remote_origin, remote_viewer, ui_for):
    """A viewer of a POSIX origin sends the Kitty encodings the program asked for: only a Windows origin's
    pseudoconsole needs the legacy ones."""
    origin = Ctl(remote_origin.instance)
    probe = remote_origin.instance.home / "keys_probe.py"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe)]
    if any(" " in word for word in words):
        pytest.fail(f"the probe's words go unquoted to every pane shell, so none may hold a space: {words}")
    sid = assert_ok(origin.request("session.new", command=" ".join(words)))["id"]
    origin.wait_mounted(sid, "left")
    local = None
    try:
        origin.wait_text(sid, "keys-ready")
        local = assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"]
        remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
        remote_viewer.wait_text(local, "keys-ready")
        ui = ui_for(remote_viewer)
        forms = {}
        for chord in CHORDS:
            ui.press(chord)
            forms[chord] = origin.wait_text(sid, rf"key {re.escape(chord)} (\w+)")[1]
        assert forms == dict.fromkeys(CHORDS, "kitty"), assert_ok(origin.request("session.text", sid, all=True))["text"]
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        origin.request("session.close", sid)


def _lead(tree, sid):
    return next(s for s in find_session(tree, sid)["surfaces"] if s["kind"] == "left").get("lead")


def _pressed(ui, origin, sid, presses):
    """Presses each chord and returns the form of its `presses`-th report (from 0) in the origin's whole pane text."""
    forms = {}
    for chord in CHORDS:
        ui.press(chord)
        earlier = rf"(?:key {re.escape(chord)} \w+[\s\S]*?){{{presses}}}"
        forms[chord] = origin.wait_text(sid, rf"{earlier}key {re.escape(chord)} (\w+)", all=True)[1]
    return forms


@pytest.mark.requires(
    "posix-command-line",
    reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
)
@pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell")
@pytest.mark.parametrize("remote_origin", ["headless"], indirect=True)
@pytest.mark.covers("behavior:pane.typing", "ui:pane-lead-cover-{pane}")
def test_a_viewer_of_a_posix_origin_takes_the_lead_back_and_its_keys_reach_the_program(
    remote_origin, remote_viewer, ui_for
):
    """The origin's own client takes the lead: the viewer's pane is covered. Take Lead (`session.lead` on the viewer)
    clears the cover. After a lost connection the pane attaches again behind a cover, as the origin's client leads
    again, and a key press takes it back. Each time the keys reach the program in the Kitty encodings it asked for:
    only a Windows origin's pseudoconsole needs the legacy ones, and its role reports take the attach bridge too."""
    origin = Ctl(remote_origin.instance)
    probe = remote_origin.instance.home / "keys_probe.py"
    probe.write_text(PROBE, encoding="utf-8")
    words = [sys.executable, "-I", os.fspath(probe)]
    if any(" " in word for word in words):
        pytest.fail(f"the probe's words go unquoted to every pane shell, so none may hold a space: {words}")
    sid = assert_ok(origin.request("session.new", command=" ".join(words)))["id"]
    origin.wait_mounted(sid, "left")
    local = None
    try:
        origin.wait_text(sid, "keys-ready")
        local = assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"]
        remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
        remote_viewer.wait_text(local, "keys-ready")
        remote_viewer.wait_tree(lambda tree: _lead(tree, local) == "leader")
        ui = ui_for(remote_viewer)
        cover = ui.ui_ids.expand("pane-lead-cover-{pane}", pane="left")
        ui.absent(cover)

        assert_ok(origin.request("session.lead", sid))
        remote_viewer.wait_tree(lambda tree: _lead(tree, local) == "follower")
        ui.find(cover)
        assert_ok(remote_viewer.request("session.lead", local))
        remote_viewer.wait_tree(lambda tree: _lead(tree, local) == "leader")
        ui.absent(cover)
        forms = _pressed(ui, origin, sid, 0)
        assert forms == dict.fromkeys(CHORDS, "kitty"), assert_ok(origin.request("session.text", sid, all=True))["text"]

        remote_origin.sever()
        remote_viewer.wait_tree(lambda tree: _lead(tree, local) == "follower")
        ui.find(cover)
        ui.press("x")
        remote_viewer.wait_tree(lambda tree: _lead(tree, local) == "leader")
        ui.absent(cover)
        forms = _pressed(ui, origin, sid, 1)
        assert forms == dict.fromkeys(CHORDS, "kitty"), assert_ok(origin.request("session.text", sid, all=True))["text"]
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        origin.request("session.close", sid)
