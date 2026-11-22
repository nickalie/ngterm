"""The macOS default chords are upstream's: with no `keymap.conf` line of its own, `keymap.list` reports upstream's
table (`BuiltinAction.defaultChord`), and the chords typed through the OS's keyboard input run their actions: ⇧⌘↩
zooms the focused terminal and shows it again, ⌘N opens one new session. Each test launches its own instance."""

import sys

import pytest
from agterm_conformance.match import assert_absent, assert_ok, session_ids
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.skipif(sys.platform != "darwin", reason="upstream's macOS default chords")

# upstream v0.42.3's `BuiltinAction.defaultChord`, as `keymap.list` spells it (`Chord.displayString`)
UPSTREAM_CHORDS = {
    "new_window": "cmd+opt+n",
    "new_workspace": "cmd+shift+n",
    "new_session": "cmd+n",
    "open_directory": "cmd+o",
    "close_session": "cmd+w",
    "reopen_recent": "cmd+shift+t",
    "undo_close": "cmd+z",
    "increase_font_size": "cmd++",
    "decrease_font_size": "cmd+-",
    "reset_font_size": "cmd+0",
    "toggle_split": "cmd+d",
    "toggle_horizontal_split": "cmd+shift+d",
    "toggle_scratch": "cmd+j",
    "toggle_overlay": "cmd+shift+j",
    "toggle_terminal_zoom": "cmd+shift+return",
    "toggle_search": "cmd+f",
    "toggle_sidebar": "ctrl+cmd+s",
    "toggle_fullscreen": "ctrl+cmd+f",
    "toggle_flag": "cmd+shift+f",
    "quick_terminal": "ctrl+`",
    "session_palette": "ctrl+p",
    "command_palette": "ctrl+shift+p",
    "custom_command_palette": "ctrl+shift+o",
    "show_attention": "ctrl+shift+i",
    "dashboard": "cmd+shift+g",
    "attach_remote": "cmd+shift+r",
    "focus_left_pane": "cmd+opt+left",
    "focus_right_pane": "cmd+opt+right",
    "previous_session": "cmd+opt+up",
    "next_session": "cmd+opt+down",
    "previous_attention_session": "ctrl+opt+up",
    "next_attention_session": "ctrl+opt+down",
}


def _chords(ctl):
    listing = assert_ok(ctl.request("keymap.list"))["keymap"]
    return {action["action"]: action.get("chord") for action in listing["actions"]}


@pytest.mark.covers("keymap.list")
def test_the_keymap_lists_upstreams_command_chords(launch):
    ctl = launch()
    chords = _chords(ctl)
    assert {action: chords.get(action) for action in UPSTREAM_CHORDS} == UPSTREAM_CHORDS


def _cat_session(ctl):
    """A new selected session whose terminal runs `cat`, once it reads the keys."""
    sid = assert_ok(ctl.request("session.new", command="cat"))["id"]
    ctl.wait_mounted(sid, "left")
    assert_ok(ctl.request("session.type", sid, text="cat-reads\n"))
    ctl.wait_text(sid, r"cat-reads[\s\S]*cat-reads", all=True)
    return sid


@pytest.mark.covers("field:tree.zoomedSurface")
def test_command_shift_return_zooms_the_focused_terminal_and_back(launch, ui_for):
    ctl = launch()
    ui = ui_for(ctl)
    sid = _cat_session(ctl)
    ui.press("cmd+shift+return")
    wait_until(lambda: ctl.tree()["tree"].get("zoomedSurface") == f"surface:{sid}:left")
    ui.press("cmd+shift+return")

    def unzoomed():
        assert_absent(ctl.tree()["tree"], "zoomedSurface")
        return True

    wait_until(unzoomed)


@pytest.mark.covers("tree")
def test_command_n_opens_one_new_session(launch, ui_for):
    ctl = launch()
    ui = ui_for(ctl)
    _cat_session(ctl)
    before = session_ids(ctl.tree())
    ui.press("cmd+n")
    wait_until(lambda: len(session_ids(ctl.tree())) == len(before) + 1)
    # a key equivalent the menu and the key monitor both ran would open a second one
    assert len(session_ids(ctl.tree())) == len(before) + 1
