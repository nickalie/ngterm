"""Shell profiles in the ui: Settings' Default shell picker and its hints, and the sidebar's + menu, which starts one
session with a profile (capability `shell-profiles`, a Rust-only extension; its Settings ids are under `rust:` in
ui-ids.yaml). Windows detects its installed shells for both; elsewhere they list the settings file's `shellProfiles`.

Which profile started a pane's shell is read back by typing into it (`agterm_conformance.shell_profiles.shell_of`):
the profiles differ in the `QA_SHELL` they set."""

import json
import sys

import pytest
from agterm_conformance.drivers.base import ElementNotFound
from agterm_conformance.match import find_session, session_ids
from agterm_conformance.shell_profiles import (
    ALPHA,
    BETA,
    NO_PROFILE,
    SHELL_PROFILES,
    SHELL_START,
    profile as _profile,
    settings as _settings,
    shell_of,
)
from agterm_conformance.wait import DEFAULT_TIMEOUT, wait_until

WINDOWS = sys.platform == "win32"
CONFIG_DEFAULT = "Config / system default"
# What the + menu appends to a profile Live cannot run where sessions are restored live.
FRESH = " (fresh shell)"


def _open_general(ui):
    try:
        found = ui.tab("General", timeout=1)
    except ElementNotFound:
        # Linux and Windows have no Settings chord: the app menu's item opens it (tests/ui/test_settings.py).
        ui.menu("ngterm", "Settings…")
        found = ui.tab("General")
    ui.click(found)


def _close_settings(ui):
    ui.press("escape")
    ui.absent("settings-shell-profile")


def _choose(ui, item):
    """Pick `item` from the Default shell picker's menu, pressing the picker again while the open menu lacks it."""

    def opened():
        if not ui.all("settings-shell-profile"):
            _open_general(ui)
        ui.click(ui.find("settings-shell-profile", role="popup"))
        try:
            return ui.menu_item(item, timeout=DEFAULT_TIMEOUT)
        except ElementNotFound as error:
            ui.press("escape")
            raise AssertionError(f"the open menu has no {item!r}: {error}") from None

    ui.click(wait_until(opened, timeout=3 * DEFAULT_TIMEOUT))


def _stored_default(ctl):
    return json.loads((ctl.instance.state_dir / "settings.json").read_text(encoding="utf-8")).get(
        "defaultShellProfile"
    )


def _added(ctl, before):
    """The one session the tree gained over `before`, mounted."""
    tree = ctl.wait_tree(lambda tree: set(session_ids(tree)) - before)
    (created,) = set(session_ids(tree)) - before
    ctl.wait_mounted(created, "left")
    return created


def _from_menu(ctl, ui, item):
    """A session started from the + menu's `item`."""
    before = set(session_ids(ctl.tree()))
    ui.click(ui.find("add-session"))
    ui.click(ui.menu_item(item))
    return _added(ctl, before)


def _first(ctl):
    (sid,) = session_ids(ctl.tree())
    ctl.wait_mounted(sid, "left")
    return sid


@SHELL_PROFILES
@pytest.mark.covers("ui:settings-shell-profile", "ui:settings-shell-profile-hint")
def test_the_default_shell_picker_sets_the_shell_of_new_sessions_only(launch, ui_for):
    ctl = launch(settings=_settings())
    ui = ui_for(ctl)
    existing = _first(ctl)
    _open_general(ui)
    ui.find("settings-shell-profile", role="popup")
    assert "new sessions" in ui.text(ui.find("settings-shell-profile-hint", role="text"))
    # neither hint about the selected profile shows for the config/system default
    ui.absent("settings-shell-profile-missing")
    ui.absent("settings-shell-profile-live")

    _choose(ui, "QA Beta")
    # the complete profile is saved, so a later change to the catalog never changes what it runs
    wait_until(lambda: _stored_default(ctl) == _profile(BETA))
    _close_settings(ui)
    assert shell_of(ctl, _from_menu(ctl, ui, "New Session")) == BETA
    # the session that existed before keeps the shell it had
    assert shell_of(ctl, existing) == NO_PROFILE

    _open_general(ui)
    _choose(ui, CONFIG_DEFAULT)
    wait_until(lambda: _stored_default(ctl) is None)
    _close_settings(ui)
    assert shell_of(ctl, _from_menu(ctl, ui, "New Session")) == NO_PROFILE


@SHELL_PROFILES
@pytest.mark.covers("ui:settings-shell-profile")
def test_the_default_shell_picker_is_absent_without_profiles_outside_windows(launch, ui_for):
    ctl = launch()
    ui = ui_for(ctl)
    _first(ctl)
    _open_general(ui)
    ui.find("settings-new-session-placement", role="popup")
    if WINDOWS:
        # Windows offers the shells it detects even without a settings entry
        ui.find("settings-shell-profile", role="popup")
    else:
        ui.absent("settings-shell-profile")


@SHELL_PROFILES
@pytest.mark.covers("ui:settings-shell-profile-missing")
def test_a_default_whose_shell_is_gone_is_kept_and_flagged(launch, ui_for):
    gone = _profile(
        "gone", executable=r"C:\ngterm-qa-missing\sh.exe" if WINDOWS else "/ngterm-qa-missing/sh"
    )
    ctl = launch(settings=_settings(gone))
    ui = ui_for(ctl)
    # its pane cannot start the program, so it never mounts: the window's one session is there all the same
    ctl.wait_tree(lambda tree: len(session_ids(tree)) == 1)
    _open_general(ui)
    # never silently replaced by another shell: the picker keeps it, and the hint asks for another
    assert "no longer installed" in ui.text(ui.find("settings-shell-profile-missing", role="text"))
    assert _stored_default(ctl) == gone


@SHELL_PROFILES
@pytest.mark.covers("ui:add-session")
def test_the_add_session_menu_starts_one_session_with_a_profile_in_its_directory(launch, ui_for):
    ctl = launch(settings=_settings(_profile(ALPHA)))
    start = ctl.instance.root / "beta start"
    start.mkdir()
    ctl = launch.relaunch(_seed(ctl, shellProfiles=[_profile(ALPHA), _profile(BETA, startingDirectory=str(start))]))
    ui = ui_for(ctl)
    _first(ctl)

    beta = _from_menu(ctl, ui, "QA Beta")
    assert shell_of(ctl, beta) == BETA
    # the profile's starting directory is where its session starts
    ctl.wait_tree(lambda tree: find_session(tree, beta).get("cwd") == str(start))
    # the choice was for that session alone: New Session still runs the default
    assert shell_of(ctl, _from_menu(ctl, ui, "New Session")) == ALPHA
    assert _stored_default(ctl) == _profile(ALPHA)


def _seed(ctl, **changes):
    path = ctl.instance.state_dir / "settings.json"
    settings = json.loads(path.read_text(encoding="utf-8"))
    settings.update(changes)
    path.write_text(json.dumps(settings), encoding="utf-8")
    return ctl


@SHELL_PROFILES
@pytest.mark.requires(
    "live-sessions", "live-any-shell", any=True, reason="the hint and the label speak of sessions restored live"
)
@pytest.mark.covers("ui:settings-shell-profile-live", "ui:add-session", "field:tree.session.backedByLive")
def test_under_live_a_profile_live_cannot_run_is_marked_a_fresh_shell(launch, ui_for):
    # the panes' `sh` is no shell Live wraps
    ctl = launch(settings=_settings(_profile(ALPHA), restoreMode="live"))
    ui = ui_for(ctl)
    _first(ctl)
    _open_general(ui)
    assert "Live sessions" in ui.text(ui.find("settings-shell-profile-live", role="text"))
    _close_settings(ui)

    fresh = _from_menu(ctl, ui, f"QA Beta{FRESH}")
    assert shell_of(ctl, fresh) == BETA
    assert find_session(ctl.tree(), fresh).get(ctl.vocabulary.backed_key) is not True


@SHELL_PROFILES
@pytest.mark.requires("windows-shells", reason="Windows detects its installed shells; other OSes detect none")
@pytest.mark.covers("ui:add-session", "ui:settings-shell-profile")
def test_windows_offers_its_detected_shells_and_runs_command_prompt(launch, ui_for):
    ctl = launch()
    ui = ui_for(ctl)
    _first(ctl)
    # Windows PowerShell and Command Prompt come with every Windows, Git Bash with the Git the suite runs on
    detected = ("Windows PowerShell", "Command Prompt", "Git Bash")
    _open_general(ui)
    for name in detected:
        _choose_and_dismiss(ui, name)
    _close_settings(ui)
    ui.click(ui.find("add-session"))
    for name in detected:
        ui.menu_item(name)
    ui.press("escape")

    cmd = _from_menu(ctl, ui, "Command Prompt")
    # only cmd.exe expands %OS%; the caret keeps the typed line from matching its own echo
    marker = "QCMD^X"

    def expanded():
        if not ctl.request("session.type", cmd, text=f"echo {marker}%OS%\n").ok:
            return None
        try:
            return ctl.wait_text(cmd, r"(?m)^QCMDXWindows_NT\s*$", timeout=2)
        except AssertionError:
            return None

    wait_until(expanded, timeout=SHELL_START)


def _choose_and_dismiss(ui, item):
    """The picker's menu lists `item`; the menu closes again without a choice."""
    ui.click(ui.find("settings-shell-profile", role="popup"))
    ui.menu_item(item)
    ui.press("escape")
    if not ui.all("settings-shell-profile"):
        _open_general(ui)
