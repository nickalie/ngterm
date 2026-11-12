"""Shell profiles: a Rust-only extension (capability `shell-profiles`).

A profile is an absolute executable with its own arguments and variables, run in place of the login shell. Settings'
`defaultShellProfile` picks it for new sessions, `shellProfiles` adds entries the + menu offers (tests/ui), and each
pane keeps its own copy through splits, duplicates and relaunches while the default changes (docs/development.md >
Windows shell profiles). Upstream has none: the reference skips these tests as unsupported. The seeded profiles and
the read-back of a pane's are `agterm_conformance.shell_profiles`'."""

import itertools
import json
import re

import pytest
from agterm_conformance.impls.base import WINDOWS, posix_shell
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.shell_profiles import (
    ALPHA,
    BETA,
    NO_PROFILE,
    SHELL_PROFILES,
    profile as _profile,
    settings as _settings,
    shell_of,
)

_MARKERS = itertools.count(1)


def _new(ctl, **args):
    sid = assert_ok(ctl.request("session.new", **args))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _split(ctl, sid):
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")


def _rewrite_settings(ctl, **changes):
    """Changes the instance's settings file in place, for the relaunch that follows to read: the way
    docs/development.md tells a user to edit `shellProfiles` (the app writes the file only on a change of its own)."""
    path = ctl.instance.state_dir / "settings.json"
    settings = json.loads(path.read_text(encoding="utf-8"))
    for key, value in changes.items():
        if value is None:
            settings.pop(key, None)
        else:
            settings[key] = value
    path.write_text(json.dumps(settings), encoding="utf-8")


@SHELL_PROFILES
@pytest.mark.covers("session.new")
def test_the_default_profile_runs_in_the_first_and_every_new_session(launch):
    ctl = launch(settings=_settings(_profile(ALPHA)))
    (first,) = session_ids(ctl.tree())
    ctl.wait_mounted(first, "left")
    # the window the launch opened before the settings were read gets the default too
    assert shell_of(ctl, first) == ALPHA
    assert shell_of(ctl, _new(ctl)) == ALPHA


@SHELL_PROFILES
@pytest.mark.covers("session.split", "session.companion.duplicate", "behavior:restore.relaunch-keeps-layout")
def test_a_pane_keeps_its_profile_through_splits_duplicates_and_relaunches_while_the_default_changes(launch):
    ctl = launch(settings=_settings(_profile(ALPHA)))
    kept = _new(ctl)
    _split(ctl, kept)
    assert shell_of(ctl, kept, pane="right") == ALPHA

    # a changed default applies to new sessions only: the relaunch restores each pane with the profile it had
    _rewrite_settings(ctl, defaultShellProfile=_profile(BETA))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(kept, "left", "right")
    assert shell_of(ctl, kept, pane="left") == ALPHA
    assert shell_of(ctl, kept, pane="right") == ALPHA
    fresh = _new(ctl)
    assert shell_of(ctl, fresh) == BETA
    # a split copies its session's profile, not the default
    assert_ok(ctl.request("session.split", kept, mode="off"))
    _split(ctl, fresh)
    assert shell_of(ctl, fresh, pane="right") == BETA

    # a duplicate copies the source panes' profiles, not the default
    copy = assert_ok(ctl.request("session.companion.duplicate", kept))["id"]
    ctl.wait_mounted(copy, "left")
    assert shell_of(ctl, copy, pane="left") == ALPHA
    if find_session(ctl.tree(), copy).get("hasSplit"):
        ctl.wait_mounted(copy, "right")
        assert shell_of(ctl, copy, pane="right") == ALPHA


@SHELL_PROFILES
@pytest.mark.covers("session.companion.duplicate")
def test_a_session_from_before_any_default_keeps_the_configured_shell(launch):
    ctl = launch(settings=_settings())
    old = _new(ctl)
    assert shell_of(ctl, old) == NO_PROFILE

    _rewrite_settings(ctl, defaultShellProfile=_profile(ALPHA))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(old, "left")
    # neither the restored session nor its duplicate adopts the later default
    assert shell_of(ctl, old) == NO_PROFILE
    copy = assert_ok(ctl.request("session.companion.duplicate", old))["id"]
    ctl.wait_mounted(copy, "left")
    assert shell_of(ctl, copy) == NO_PROFILE
    assert shell_of(ctl, _new(ctl)) == ALPHA


@SHELL_PROFILES
@pytest.mark.covers("session.new")
@pytest.mark.parametrize(
    "malformed",
    [
        pytest.param({"executable": "sh"}, id="relative-executable"),
        pytest.param({"env": {"QA_SHELL": "malformed", "BAD=NAME": "x"}}, id="variable-name-with-equals"),
    ],
)
def test_a_malformed_default_starts_the_configured_shell(launch, malformed):
    ctl = launch(settings=_settings({**_profile("malformed"), **malformed}))
    (first,) = session_ids(ctl.tree())
    ctl.wait_mounted(first, "left")
    assert shell_of(ctl, first) == NO_PROFILE
    assert shell_of(ctl, _new(ctl)) == NO_PROFILE


@SHELL_PROFILES
@pytest.mark.covers("session.new")
def test_a_default_whose_program_is_gone_shows_why_in_the_pane(launch):
    # an uninstalled shell: the profile is well formed, its program cannot start, and the pane says so (libghostty's
    # failure report under the start's own complaint) instead of staying blank
    missing = r"C:\ngterm-qa-missing\sh.exe" if WINDOWS else "/ngterm-qa-missing/sh"
    ctl = launch(settings=_settings(_profile("gone", executable=missing)))
    (first,) = session_ids(ctl.tree())
    ctl.wait_mounted(first, "left")
    path = re.escape(missing)
    report = rf"(?s)cannot start the program:.*{path}.*failed to launch the requested command:\s+{path}"
    ctl.wait_text(first, report)
    ctl.wait_text(_new(ctl), report)


@SHELL_PROFILES
@pytest.mark.requires("posix-command-line", reason="the creation command is a POSIX line run through `/bin/sh -c`")
@pytest.mark.covers("session.new")
def test_a_creation_command_runs_in_the_command_shell_without_the_profiles_variables(launch):
    ctl = launch(settings=_settings(_profile(ALPHA)))
    marker = f"QC{next(_MARKERS):04d}X"
    command = f"printf '%s%s|%s|\\n' {marker[:3]} {marker[3:]} \"${{QA_SHELL-{NO_PROFILE}}}\"; sleep 600"
    sid = _new(ctl, command=command)
    ctl.wait_text(sid, rf"(?m)^{marker}\|{NO_PROFILE}\|\s*$")


@SHELL_PROFILES
@pytest.mark.covers("session.new")
def test_a_profiles_arguments_reach_its_program_one_by_one(launch):
    # the profile's own startup command, then its shell: each argument arrives whole, spaces and all, never a line
    # a shell splits
    scripted = _profile(
        "scripted",
        args=["-c", 'printf "%s|%s|%s\\n" QAR "$0" "$1"; exec "$QA_SH"', "one word", "two  words"],
        env={"QA_SHELL": "scripted", "QA_SH": posix_shell()},
    )
    ctl = launch(settings=_settings(scripted))
    (first,) = session_ids(ctl.tree())
    ctl.wait_mounted(first, "left")
    ctl.wait_text(first, r"(?m)^QAR\|one word\|two  words\s*$")
    assert shell_of(ctl, first) == "scripted"


def _live_row(ctl, sid):
    vocabulary = ctl.vocabulary
    rows = assert_ok(ctl.request(vocabulary.command("live.list")))[vocabulary.result_key]["entries"]
    (row,) = [row for row in rows if row.get("sessionID") == sid and row["pane"] == "left"]
    return row


@SHELL_PROFILES
@pytest.mark.requires("live-any-shell", reason="Live sessions in a bash login shell are the Rust build's own")
@pytest.mark.requires("posix-command-line", reason="the profile runs the host's /bin/bash")
@pytest.mark.covers("behavior:restore.live-session-survives-relaunch", "field:tree.session.backedByLive")
def test_a_live_pane_runs_its_profiles_shell_and_a_profile_live_cannot_run_starts_fresh(launch):
    bash = _profile("bash", executable="/bin/bash", args=["-l"])
    ctl = launch(settings=_settings(bash, restoreMode="live"))
    backed_key = ctl.vocabulary.backed_key
    live = _new(ctl)
    ctl.wait_tree(lambda tree: find_session(tree, live).get(backed_key) is True)
    assert shell_of(ctl, live) == "bash"
    before = _live_row(ctl, live)
    assert_ok(ctl.request("session.type", live, text="QA_KEPT=still-here\n"))

    # `sh` is no shell Live wraps: a session with that profile runs it as an ordinary pane, never the login shell
    _rewrite_settings(ctl, defaultShellProfile=_profile(ALPHA))
    ctl = launch.relaunch(ctl)
    ctl.wait_mounted(live, "left")
    ctl.wait_tree(lambda tree: find_session(tree, live).get(backed_key) is True)
    # the same shell came through the quit: its leader, its own variable and the profile's
    assert _live_row(ctl, live)["leaderPID"] == before["leaderPID"]
    marker = f"QK{next(_MARKERS):04d}X"
    assert_ok(ctl.request("session.type", live, text=f"printf '%s%s|%s|\\n' {marker[:3]} {marker[3:]} \"$QA_KEPT\"\n"))
    ctl.wait_text(live, rf"(?m)^{marker}\|still-here\|\s*$")
    assert shell_of(ctl, live) == "bash"
    fresh = _new(ctl)
    assert shell_of(ctl, fresh) == ALPHA
    assert find_session(ctl.tree(), fresh).get(backed_key) is not True
