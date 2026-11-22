"""Custom commands of `keymap.conf` fired by their chords: the chord goes in through the OS's keyboard input, the key
monitor runs the command's line, and the tree reads back what the line did. Each test launches its own instance, so
it knows the whole sidebar; the file is the instance's own `<state>/config/keymap.conf`. Every line calls the build's
own CLI with an explicit `--socket` and writes only into the instance root, in the build's command-line dialect
(`agterm_conformance.command_line`)."""

import os
import re
import shlex

import pytest
from agterm_conformance.command_line import CommandLines, read_written
from agterm_conformance.match import active_session_id, assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

# the panes run /bin/sh, which reports no directory: a typed `cd` is followed by the OSC 7 a shell integration sends
REPORT_PWD = "printf '\\033]7;file://localhost%s\\007' \"$PWD\""
PROMPT = r"\$ "


def _stderr_into(lines, command, path):
    """`command`, its error output into `path` through a temp file renamed into place: the file appears once the
    command has run, empty when it printed no error."""
    tmp, target = lines.quote(f"{path}.tmp"), lines.quote(path)
    move = f"move /y {tmp} {target} >nul" if lines.cmd else f"mv {tmp} {target}"
    return lines.then(f"{command} 2> {tmp}", move)


def _written(path):
    """The text of `path` once it exists, empty text included."""

    def written():
        assert path.exists(), f"{path.name} not written yet"
        return True

    written.__qualname__ = f"{path.name} written"
    wait_until(written)
    return read_written(path)


@pytest.mark.since("0.42.0")
@pytest.mark.covers("session.go", "keymap.reload", "workspace.collapse")
def test_a_chord_bound_to_session_go_to_n_selects_the_nth_session(adapter, launch, ui_for):
    # upstream 487d643b: `session go --to N` makes a chord per sidebar place nine keymap command lines; Control-1 and
    # Control-2 stay the pane shortcuts, so the chords here start at Control-3.
    lines = CommandLines.of(adapter)
    ctl = launch()
    ui = ui_for(ctl)
    wid = assert_ok(ctl.request("workspace.new"))["id"]
    for _ in range(3):
        ctl.wait_mounted(assert_ok(ctl.request("session.new", workspace=wid))["id"], "left")
    ids = session_ids(ctl.tree())
    past = len(ids) + 1
    assert 3 < len(ids) < 9, ids
    assert_ok(ctl.request("workspace.collapse", wid))
    assert_ok(ctl.request("session.select", ids[0]))

    go = f"{lines.quote(adapter.ctl_path)} session go --socket {lines.quote(ctl.instance.socket)} --to"
    markers = {n: ctl.instance.root / f"go-{n}" for n in (3, len(ids), past)}
    keymap = ctl.instance.state_dir / "config" / "keymap.conf"
    keymap.write_text(
        "".join(f'command "go {n}" ctrl+{n} {_stderr_into(lines, f"{go} {n}", path)}\n' for n, path in markers.items())
    )
    assert assert_ok(ctl.request("keymap.reload")) == {"count": 0}

    for n in (3, len(ids)):
        ui.press(f"ctrl+{n}")
        assert _written(markers[n]) == "", n
        ctl.wait_tree(lambda tree, n=n: active_session_id(tree) == ids[n - 1])

    ui.press(f"ctrl+{past}")
    assert _written(markers[past]).strip() == f"error: no session at position {past}: {len(ids)} navigable"
    assert active_session_id(ctl.tree()) == ids[-1], "a place past the end keeps the selection"


def _where_keymap(ctl, lines, chord, marker):
    """A custom command `where` on `chord` recording the session directory, host and pane it ran for."""
    fields = [lines.var(name) for name in ("AGT_SESSION_PWD", "AGT_SESSION_HOST", "AGT_PANE")]
    keymap = ctl.instance.state_dir / "config" / "keymap.conf"
    keymap.write_text(f'command "where" {chord} {lines.write(marker, *fields)}\n')
    assert assert_ok(ctl.request("keymap.reload")) == {"count": 0}


def _ran_for(marker):
    text = _written(marker)
    marker.unlink()
    return tuple(text.split("|"))


def _show_scratch(ctl, ui, sid):
    """The titlebar button shows the selected session's scratch; a typed line echoed in it proves it took the
    keyboard, so a chord typed next is the scratch's."""
    ui.click(ui.find("scratch-toggle", role="button", enabled=True))
    ctl.wait_mounted(sid, "scratch")
    ctl.wait_text(sid, PROMPT, pane="scratch")
    ui.type_text("echo scratch-has-keys")
    ui.press("enter")
    ctl.wait_text(sid, re.compile(r"^scratch-has-keys$", re.MULTILINE), pane="scratch")


def _where_badge(ui):
    badges = [ui.text(badge) for badge in ui.all("palette-badge")]
    if badges:
        return badges == ["custom"]
    # SwiftUI gives the badge its custom row's id on the reference.
    rows = re.findall(r"^(palette-item-custom-[^(]+)\(text(?: |\))", ui.tree_dump(), re.MULTILINE)
    return any(
        [ui.text(item) for item in ui.all(row) if item.role == "text"] == ["where", "custom", "ctrl+alt+k"]
        for row in set(rows)
    )


@pytest.mark.since("0.42.1")
@pytest.mark.requires("posix-command-line", reason="the scratch shell is /bin/sh and reports its directory with printf")
@pytest.mark.covers("keymap.reload", "session.scratch", "ui:scratch-toggle", "ui:command-palette", "ui:palette-badge")
def test_a_chord_in_the_scratch_gets_its_directory_and_the_palette_keeps_the_primary(adapter, launch, ui_for):
    # upstream #981: a chord typed in the scratch runs with the scratch's own directory and no host; the palette runs
    # for the primary pane
    lines = CommandLines.of(adapter)
    ctl = launch()
    ui = ui_for(ctl)
    primary = ctl.instance.root / "primary"
    scratch = primary / "scratch"
    scratch.mkdir(parents=True)
    marker = ctl.instance.root / "where"
    _where_keymap(ctl, lines, "ctrl+alt+k", marker)
    sid = assert_ok(ctl.request("session.new", cwd=os.fspath(primary)))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.select", sid))
    _show_scratch(ctl, ui, sid)
    ui.type_text(f"cd {shlex.quote(os.fspath(scratch))} && {REPORT_PWD}")
    ui.press("enter")
    ctl.wait_tree(lambda tree: find_session(tree, sid).get("scratchCwd") == os.fspath(scratch))

    ui.press("ctrl+alt+k")
    assert _ran_for(marker) == (os.fspath(scratch), "", "scratch")

    ui.menu("Navigate", "Command Palette")
    ui.set_text(ui.find("command-palette", role="text-field"), "where")
    wait_until(lambda: _where_badge(ui), timeout=ui.timeout)
    ui.press("enter")
    ui.absent("command-palette")
    assert _ran_for(marker) == (os.fspath(primary), "", "left")
    assert find_session(ctl.tree(), sid)["cwd"] == os.fspath(primary)


@pytest.mark.since("0.42.1")
@pytest.mark.requires(
    "posix-command-line",
    reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
)
@pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell")
@pytest.mark.covers("session.scratch", "field:tree.session.scratchCwd", "ui:scratch-toggle")
def test_an_attached_sessions_scratch_is_a_local_shell(adapter, remote_origin, remote_viewer, ui_for, tmp_path):
    # upstream #981: the scratch of an attached session starts in its directory when that exists here, else in home,
    # and a chord typed in it names no host; one run for the primary pane keeps the origin's host
    lines = CommandLines.of(adapter)
    origin = Ctl(remote_origin.instance)
    ui = ui_for(remote_viewer)
    marker = remote_viewer.instance.root / "where"
    _where_keymap(remote_viewer, lines, "ctrl+alt+k", marker)
    kept, gone = tmp_path / "kept", tmp_path / "gone"
    kept.mkdir()
    gone.mkdir()
    made, attached = [], []
    try:
        for directory in (kept, gone):
            made.append(assert_ok(origin.request("session.new", cwd=os.fspath(directory)))["id"])
            origin.wait_mounted(made[-1], "left")
        gone.rmdir()
        for sid in made:
            attached.append(assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"])
            remote_viewer.wait_tree(lambda tree: find_session(tree, attached[-1]).get("realized"))
        home = os.fspath(remote_viewer.instance.home)
        for local, start in zip(attached, (os.fspath(kept), home), strict=True):
            assert_ok(remote_viewer.request("session.select", local))
            host = find_session(remote_viewer.tree(), local)["remoteHost"]
            assert host is not None
            _show_scratch(remote_viewer, ui, local)
            remote_viewer.wait_tree(lambda tree, local=local, start=start: find_session(tree, local).get("scratchCwd")
                                    == start)
            ui.press("ctrl+alt+k")
            assert _ran_for(marker) == (start, "", "scratch")
            assert assert_ok(remote_viewer.request("keymap.run", local, name="where")) == {"id": local}
            assert _ran_for(marker)[1:] == (host, "left")
            assert_ok(remote_viewer.request("session.scratch", local, mode="off"))
    finally:
        for local in attached:
            remote_viewer.request("session.close", local)
        for sid in made:
            origin.request("session.close", sid)
