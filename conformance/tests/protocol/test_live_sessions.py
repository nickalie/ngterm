"""The `live` group, `session.lead`/`session.restart` and the Live-sessions read-back.

Every group command, result key, tree key and error text goes through the instance's vocabulary
(`agterm_conformance.vocabulary`): the Swift reference answers this group by upstream's names.

The module's shared instance runs without Live, so its daemons are only claimed, never observed: the inventory,
prune, the hostless `live.tree` and every refusal are checked there. The Live tests share one instance launched with
`restoreMode: live` seeded; a pane in it runs the account's zsh or fish inside a live session (a session of the Rust
build's server, a daemon of its own upstream), so a line typed there only prints a marker. The tests that type no such
line (`LIVE_IN_ANY_SHELL`) also run where the Rust build wraps a bash or PowerShell login shell (`live-any-shell`). Each test ends the sessions
it opens, which ends their daemons; the harness's stop then finds no process left in the instance's namespace (under the
vocabulary's directory variable, or the session server of the namespace).

`live.reset` is refused here without `--force` only: a confirmed reset quits the app and relaunches it outside the
harness (see the catalog exemption)."""

import base64
import contextlib
import itertools
import json
import os
import shlex

import pytest
from agterm_conformance.client import NoResponse
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.vocabulary import foundation_resolved
from agterm_conformance.wait import wait_frame, wait_until

UNKNOWN = "zzz"
LIVE = pytest.mark.requires(
    "live-sessions", reason="needs panes in live sessions, which upstream creates only for a zsh or fish login shell"
)
# A Live test that types no line of a particular shell: it holds in any login shell Live wraps, upstream's zsh and fish
# and the Rust build's bash and PowerShell too (`live-any-shell`).
LIVE_IN_ANY_SHELL = pytest.mark.requires(
    "live-sessions",
    "live-any-shell",
    any=True,
    reason="needs panes in live sessions, which upstream creates only for a zsh or fish login shell",
)
# The Rust-only extension that keeps a page handed to a presenter past that presenter's loss (rust-build.md >
# Rust-only extensions): the reference ends it, as upstream does.
PAGES_SURVIVE = pytest.mark.requires("pages-survive", reason="upstream ends a page with its presenter")
ATTRIBUTIONS = ("supervisor", "app", "orphaned", "unknown")
PRESENTATION_VERSION = 1
# These observed failures have no established cause (README > Reference observations). In particular, a command
# still running, or one released only after attach, cannot have ended in upstream's documented host-handoff gap.
RESTART_OUTPUT_MISSING = pytest.mark.upstream_bug(
    "live-restarted-command-output-missing",
    reason="a restarted Live command's first line can be absent for 10 s while its sleep 600 still runs; "
    "cause unknown, observed in reference-macos 42520472215 (conformance/README.md > Reference observations)",
)
WAIT_OUTPUT_LOST = pytest.mark.upstream_bug(
    "live-wait-command-loses-output",
    reason="a Live --wait pane can leave its output above an empty visible screen or lose it from the whole "
    "scrollback at exit even after attach; cause unknown, observed in reference-macos 42520472215 "
    "(conformance/README.md > Reference observations)",
)
# The line a `--wait` pane holds on once its command ended (Ghostty's `wait-after-command`).
HELD_PROMPT = r"Process exited\. Press any key to close the terminal\."
_markers = itertools.count(1)


def _rows(ctl, response, sid):
    """This session's `live.list` rows by pane."""
    rows = assert_ok(response)[ctl.vocabulary.result_key]["entries"]
    return {row["pane"]: row for row in rows if row.get("sessionID") == sid}


def _entries(ctl, sid):
    return _rows(ctl, ctl.request(ctl.vocabulary.command("live.list")), sid)


def _initial_session(ctl):
    (sid,) = session_ids(ctl.tree())
    ctl.wait_mounted(sid, "left")
    return sid


# -- without Live


@pytest.mark.covers("live.list", "field:result.live")
def test_list_joins_each_pane_claim(ctl):
    sid = _initial_session(ctl)
    inventory = assert_ok(ctl.request(ctl.vocabulary.command("live.list")))[ctl.vocabulary.result_key]
    assert_subset(
        inventory,
        {
            "inventoryComplete": True,
            "restore": {"configured": "none", "requestedAtLaunch": "none", "active": "none"},
            # The socket namespace is the state dir's hash, the one the harness checks for leftovers under.
            "endpoint": {"socketDirectory": ctl.instance.live_dir},
        },
    )
    (row,) = [row for row in inventory["entries"] if row.get("sessionID") == sid]
    # Outside Live a pane still claims its daemon name; no daemon runs, so none is observed.
    assert_subset(row, {"pane": "left", "state": "claimed", "observation": "absent", "windowState": "open"})
    assert row["daemon"].startswith(ctl.vocabulary.daemon_prefix)
    assert_absent(row, "clients", "leaderPID")


@pytest.mark.covers("live.prune", "field:result.text", "field:result.affected")
def test_prune_finds_no_orphans(ctl):
    assert assert_ok(ctl.request(ctl.vocabulary.command("live.prune"))) == {"text": "no orphan daemons", "affected": 0}


@pytest.mark.covers("live.tree", "field:result.remote")
def test_tree_without_a_host_lists_no_attachable_session_outside_live(ctl):
    remote = assert_ok(ctl.request(ctl.vocabulary.command("live.tree")))["remote"]
    assert remote["sessions"] == []
    assert remote["presentation"] >= PRESENTATION_VERSION
    assert remote["endpoint"]["socketDirectory"] == ctl.instance.live_dir


@pytest.mark.covers("live.kill")
@pytest.mark.parametrize(
    ("target", "args", "error"),
    [
        pytest.param(None, {"pane": "left", "force": True}, "live.kill requires an explicit --target", id="no-target"),
        pytest.param("SID", {"force": True}, "live.kill requires --pane left|right", id="no-pane"),
        pytest.param("SID", {"pane": "scratch", "force": True}, "live.kill requires --pane left|right", id="scratch"),
        pytest.param("SID", {"pane": "left"}, "live.kill requires --force", id="no-force"),
        pytest.param(
            "active",
            {"pane": "left", "force": True},
            "live.kill needs a session id; 'active' is not accepted",
            id="active",
        ),
        pytest.param(
            UNKNOWN, {"pane": "left", "force": True}, f"no left pane daemon for session {UNKNOWN}", id="unknown"
        ),
        pytest.param("SID", {"pane": "right", "force": True}, "no right pane daemon for session SID", id="no-split"),
    ],
)
def test_kill_rejects(ctl, target, args, error):
    sid = _initial_session(ctl)
    target = sid if target == "SID" else target
    error = ctl.vocabulary.text(error).replace("SID", sid)
    assert_error(ctl.request(ctl.vocabulary.command("live.kill"), target, **args), error)


@pytest.mark.covers("live.kill")
def test_kill_refuses_a_daemon_that_is_not_running(ctl):
    sid = _initial_session(ctl)
    daemon = _entries(ctl, sid)["left"]["daemon"]
    kill = ctl.vocabulary.command("live.kill")
    assert_error(ctl.request(kill, sid, pane="left", force=True), f"{daemon} is not running")
    assert find_session(ctl.tree(), sid)["realized"] is True


@pytest.mark.covers("live.screen")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "live.screen requires a daemon name", id="no-name"),
        pytest.param({"name": "two words"}, "live.screen requires a daemon name", id="not-plain"),
        pytest.param({"name": "agterm-x", "all": True, "lines": 3}, "use either --all or --lines, not both", id="both"),
        pytest.param({"name": "agterm-x", "lines": 0}, "--lines must be greater than 0", id="zero-lines"),
        pytest.param({"name": "agterm-x"}, "could not read the live session screen of {name}", id="no-daemon"),
    ],
)
def test_screen_rejects(ctl, args, error):
    assert_error(ctl.request(ctl.vocabulary.command("live.screen"), **args), ctl.vocabulary.text(error, **args))


@pytest.mark.covers("live.reset")
def test_reset_requires_force(ctl):
    assert_error(ctl.request(ctl.vocabulary.command("live.reset")), ctl.vocabulary.text("live.reset requires --force"))
    assert_absent(ctl.tree()["tree"], "liveReset")


@pytest.mark.covers("session.lead")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "pane has no lead to take", id="not-live"),
        pytest.param({"pane": "right"}, "session has no split pane", id="no-split"),
        pytest.param({"pane": "scratch"}, "the scratch terminal has no lead", id="scratch"),
    ],
)
def test_lead_rejects(ctl, args, error):
    assert_error(ctl.request("session.lead", _initial_session(ctl), **args), error)


@pytest.mark.covers("session.restart")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "session.restart requires --pane-id or --pane", id="no-pane"),
        pytest.param(
            {"pane": "left"},
            "session.restart needs Live sessions mode; this pane has no live shell to replace",
            id="not-live",
        ),
        pytest.param({"pane": "right"}, "session has no split pane", id="no-split"),
        pytest.param({"pane": "scratch"}, "session.restart does not address the scratch pane", id="scratch"),
        pytest.param({"paneID": UNKNOWN}, f"unknown pane id: {UNKNOWN}", id="unknown-pane-id"),
        pytest.param(
            {"pane": "left", "command": "  "},
            "session.restart takes a non-empty command, or none to replay",
            id="blank-command",
        ),
    ],
)
def test_restart_rejects(ctl, args, error):
    assert_error(ctl.request("session.restart", _initial_session(ctl), **args), error)


@pytest.mark.covers("live.present")
def test_present_refuses_without_a_session(ctl):
    with ctl.stream(ctl.vocabulary.command("live.present")) as stream:
        assert_error(stream.reply, ctl.vocabulary.text("live.present requires a session"))


@pytest.mark.covers("live.present", "session.overlay.job.run", "session.overlay.page.serve")
@pytest.mark.parametrize(
    ("cmd", "target", "error"),
    [
        pytest.param(
            "live.present", "SID", "session is not live-backed, so nothing can be attached to it", id="present-not-live"
        ),
        pytest.param("session.overlay.job.run", None, "session.overlay.job.run requires a job id", id="job-no-id"),
        pytest.param("session.overlay.job.run", UNKNOWN, "invalid job id", id="job-invalid"),
        pytest.param(
            "session.overlay.job.run", "00000000-0000-0000-0000-000000000000", "job not claimable", id="job-unknown"
        ),
        pytest.param(
            "session.overlay.page.serve",
            None,
            "session.overlay.page.serve requires a page token",
            id="page-no-token",
        ),
        pytest.param("session.overlay.page.serve", UNKNOWN, "invalid page token", id="page-invalid"),
        pytest.param(
            "session.overlay.page.serve", "00000000-0000-0000-0000-000000000000", "page not claimable", id="page-unknown"
        ),
    ],
)
def test_stream_commands_refuse_before_streaming(ctl, cmd, target, error):
    target = _initial_session(ctl) if target == "SID" else target
    cmd = ctl.vocabulary.command(cmd) if cmd.startswith("live.") else cmd
    with ctl.stream(cmd, target) as stream:
        assert_error(stream.reply, error)


# -- Live


@pytest.fixture(scope="module")
def live(adapter):
    instance = adapter.launch(settings={"restoreMode": "live"})
    try:
        ctl = Ctl(instance)
        # An adapter that advertises Live must get it: a fallback to plain shells fails here rather than skipping.
        assert assert_ok(ctl.request("restore.mode"))["restore"]["active"] == "live"
        yield ctl
    finally:
        adapter.stop(instance)


def _live_session(live, split=False, **args):
    sid = assert_ok(live.request("session.new", **args))["id"]
    live.wait_mounted(sid, "left")
    if split:
        assert_ok(live.request("session.split", sid, mode="on"))
        live.wait_mounted(sid, "right")

    def backed(tree):
        key = live.vocabulary.backed_key
        node = find_session(tree, sid)
        assert node.get(key) is True, f"{key} {node.get(key)!r}"
        return True

    live.wait_tree(backed)
    panes = ("left", "right") if split else ("left",)

    def running(response):
        rows = _rows(live, response, sid)
        assert all(rows.get(pane, {}).get("observation") == "running" for pane in panes), rows
        return True

    live.wait_response(live.vocabulary.command("live.list"), None, running)
    return sid


def _gone(live, daemon):
    def gone(response):
        return all(row["daemon"] != daemon for row in assert_ok(response)[live.vocabulary.result_key]["entries"])

    live.wait_response(live.vocabulary.command("live.list"), None, gone)


@LIVE_IN_ANY_SHELL
@pytest.mark.covers(
    "live.list",
    "field:result.live",
    "field:tree.session.backedByLive",
    "field:tree.session.liveAttribution",
    "field:tree.session.splitLiveAttribution",
)
def test_live_panes_run_in_claimed_daemons(live):
    sid = _live_session(live, split=True)
    node = find_session(live.tree(), sid)
    assert [surface[live.vocabulary.backed_key] for surface in node["surfaces"]] == [True, True]
    assert node["liveAttribution"] in ATTRIBUTIONS and node["splitLiveAttribution"] in ATTRIBUTIONS
    # One reply with both clients attached: upstream's count drops to 0 for a moment while a pane's client attaches.
    attached = live.wait_response(
        live.vocabulary.command("live.list"),
        None,
        lambda r: all(_rows(live, r, sid).get(p, {}).get("clients", 0) >= 1 for p in ("left", "right")),
    )
    rows = _rows(live, attached, sid)
    for pane in ("left", "right"):
        assert_subset(rows[pane], {"state": "claimed", "observation": "running"})
        assert rows[pane]["leaderPID"] > 0
    assert rows["left"]["daemon"] != rows["right"]["daemon"]
    assert_ok(live.request("session.close", sid))
    for pane in ("left", "right"):
        _gone(live, rows[pane]["daemon"])


@LIVE
@pytest.mark.covers("live.screen", "field:result.text")
def test_screen_reads_the_daemon_grid(live):
    sid = _live_session(live)
    marker = f"ZMARK{next(_markers)}"
    # The command line carries the marker split in two, so only the printed line matches it.
    assert_ok(live.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:5]} {marker[5:]}\n"))
    live.wait_text(sid, rf"(?m)^{marker}$")
    daemon = _entries(live, sid)["left"]["daemon"]
    screen = assert_ok(live.request(live.vocabulary.command("live.screen"), name=daemon))
    assert f"\n{marker}\n" in f"\n{screen['text']}\n"
    assert_ok(live.request("session.close", sid))
    _gone(live, daemon)


@LIVE_IN_ANY_SHELL
@pytest.mark.covers("session.lead")
def test_lead_on_a_pane_that_already_leads(live):
    sid = _live_session(live)
    _wait_leads(live, sid)
    assert assert_ok(live.request("session.lead", sid)) == {"id": sid}
    assert_ok(live.request("session.close", sid))


def _pane_id(live, sid):
    return next(s for s in find_session(live.tree(), sid)["surfaces"] if s["kind"] == "left")["paneID"]


@LIVE
@pytest.mark.covers("session.restart", "field:result.restart", "field:result.pane")
def test_restart_replaces_the_pane_shell(live):
    sid = _live_session(live)
    before = _entries(live, sid)["left"]
    pane_id = _pane_id(live, sid)
    result = assert_ok(live.request("session.restart", sid, pane="left", command="cat"))
    assert_subset(result, {"id": sid, "pane": "left", "restart": {"paneID": pane_id, "oldPid": before["leaderPID"]}})
    new_pid = result["restart"]["newPid"]
    assert new_pid != before["leaderPID"]
    assert result["text"].startswith(f"restarted left pane: shell {before['leaderPID']} -> {new_pid}")
    # The daemon keeps its name: the pane identity it is derived from survives the restart.
    live.wait_response(
        live.vocabulary.command("live.list"),
        None,
        lambda response: _rows(live, response, sid)["left"]["leaderPID"] == new_pid,
    )
    # Closed only once the pane runs the new program: a close racing the restart has left upstream's daemon
    # orphaned rather than ended.
    live.wait_tree(lambda tree: find_session(tree, sid).get("foreground") == ["cat"])
    assert_ok(live.request("session.close", sid))
    _gone(live, before["daemon"])


@LIVE
@pytest.mark.covers("session.restart", "field:result.restart")
def test_restart_without_a_command_replays_the_foreground_program(live, tmp_path):
    sid = _live_session(live)
    pane_id = _pane_id(live, sid)
    daemon = _entries(live, sid)["left"]["daemon"]
    marker = tmp_path / "replay runs.txt"
    work = tmp_path / "work dir"
    work.mkdir()
    # A `sh -c` job of the pane's shell, not its root: the trailing `:` keeps sh from exec'ing the sleep.
    script = f"pwd -P >> '{marker}'; sleep 600; :"
    assert_ok(live.request("session.restart", sid, paneID=pane_id, command=f"cd '{work}' && sh -c \"{script}\""))
    ran = f"{os.path.realpath(work)}\n"
    wait_until(lambda: marker.exists() and marker.read_text() == ran, timeout=20)

    receipt = assert_ok(live.request("session.restart", sid, paneID=pane_id))["restart"]
    assert receipt["replayedArgv"] == ["sh", "-c", script]
    assert receipt["paneID"] == pane_id
    # The replay runs the program again where it was running, its quoting intact.
    wait_until(lambda: marker.read_text() == ran + ran, timeout=20)
    assert _pane_id(live, sid) == pane_id
    assert_ok(live.request("session.close", sid))
    _gone(live, daemon)


@LIVE
@pytest.mark.covers("session.restart")
def test_restart_replay_refuses_a_shell_at_its_prompt(live):
    sid = _live_session(live)
    marker = f"ZMARK{next(_markers)}"
    # Once the marker prints, the pane's own shell is back at its prompt.
    assert_ok(live.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:5]} {marker[5:]}\n"))
    live.wait_text(sid, rf"(?m)^{marker}$")
    before = _entries(live, sid)["left"]
    response = live.request("session.restart", sid, paneID=_pane_id(live, sid))
    assert_error(response)
    assert response.error.endswith("holds the pane; pass --command to name the line. nothing was changed"), (
        response.error
    )
    assert _entries(live, sid)["left"]["leaderPID"] == before["leaderPID"]
    assert_ok(live.request("session.close", sid))
    _gone(live, before["daemon"])


def _ctrl_c_leaves_a_prompt(live, sid, daemon):
    """Ctrl+C on the pane's `sleep 600` ends it, and the pane's daemon goes on in its login shell: the session and its
    pane stay, `live.list` lists the same daemon, and a marker typed afterwards prints (#773)."""

    def sleeping(tree):
        foreground = find_session(tree, sid).get("foreground") or []
        assert foreground[:1] == ["sleep"], foreground
        return True

    live.wait_tree(sleeping)
    pane_id = _pane_id(live, sid)
    assert_ok(live.request("session.type", sid, text="\x03"))
    live.wait_tree(lambda tree: (find_session(tree, sid).get("foreground") or [])[:1] != ["sleep"])
    marker = f"ZMARK{next(_markers)}"
    assert_ok(live.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:5]} {marker[5:]}\n"))
    live.wait_text(sid, rf"(?m)^{marker}$")
    assert _pane_id(live, sid) == pane_id
    row = _entries(live, sid)["left"]
    assert (row["daemon"], row["observation"]) == (daemon, "running"), row
    assert_ok(live.request("session.close", sid))
    _gone(live, daemon)


@LIVE
@RESTART_OUTPUT_MISSING
@pytest.mark.covers("session.restart")
def test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt(live):
    sid = _live_session(live)
    daemon = _entries(live, sid)["left"]["daemon"]
    marker = f"ZMARK{next(_markers)}"
    line = f"printf '%s%s\\n' {marker[:5]} {marker[5:]}; sleep 600"
    assert_ok(live.request("session.restart", sid, paneID=_pane_id(live, sid), command=line))
    # The reply comes once the daemon runs the new shell, before the pane's own client has attached to it: until the
    # marker shows, the pane's terminal is not yet the client's raw one, and a Ctrl+C there ends the client and the
    # pane with it (upstream's session host hands a daemon it created over to the pane's client afterwards).
    # test_ctrl_c_right_after_a_restart_reaches_its_command holds the reply itself to the attach.
    live.wait_text(sid, rf"(?m)^{marker}$")
    _ctrl_c_leaves_a_prompt(live, sid, daemon)


@LIVE
@pytest.mark.upstream_bug(
    "restart-answers-before-the-pane-attaches",
    reason="session.restart answers, and the tree reads the command as the pane's foreground, once the daemon runs "
    "it (ControlServer+Restart.swift, shell(daemon:otherThan:)), before the pane's own client has attached to it "
    "(SessionHostRuntime/Client.swift run): a Ctrl+C typed then ends the client and the pane with it",
)
@pytest.mark.covers("session.restart")
def test_ctrl_c_right_after_a_restart_reaches_its_command(live):
    """Once the restart has answered and the tree reads its command as the pane's foreground, the pane runs it: a
    Ctrl+C typed then interrupts the command and leaves a prompt, as one typed later does (above)."""
    sid = _live_session(live)
    daemon = _entries(live, sid)["left"]["daemon"]
    assert_ok(live.request("session.restart", sid, paneID=_pane_id(live, sid), command="sleep 600"))
    _ctrl_c_leaves_a_prompt(live, sid, daemon)


@LIVE
@pytest.mark.since("0.39.1")
@pytest.mark.covers("session.new")
@WAIT_OUTPUT_LOST
def test_a_live_command_session_closes_when_its_command_exits_and_holds_with_wait(live):
    """A Live `--command` session closes when its command exits, as outside Live, and `--wait` holds its final output
    after the command and its daemon have ended (#840). This replaced #773's Ctrl+C-leaves-a-prompt case for
    `session new --command`, which v0.39.0 reversed; `session restart --command` keeps it (above)."""
    closing = assert_ok(live.request("session.new", command="sleep 1"))["id"]
    live.wait_tree(lambda tree: closing not in session_ids(tree))

    holding = assert_ok(live.request("session.new", command="echo held-output; sleep 1", wait=True))["id"]
    live.wait_mounted(holding, "left")
    live.wait_text(holding, r"(?m)^held-output$")

    def ended(response):
        row = _rows(live, response, holding).get("left", {})
        assert row.get("observation") != "running", row
        return True

    live.wait_response(live.vocabulary.command("live.list"), None, ended)
    # only the exit prompt proves the hold: the pane closes a moment after its daemon reads as ended
    live.wait_text(holding, HELD_PROMPT)
    assert holding in session_ids(live.tree())
    live.wait_text(holding, r"(?m)^held-output$")
    assert_ok(live.request("session.close", holding))


@LIVE
@pytest.mark.since("0.40.0")
@pytest.mark.covers("session.new", "session.text", "field:result.text")
@WAIT_OUTPUT_LOST
def test_a_wait_command_keeps_scrollback_after_attached_exit(live, tmp_path):
    """An attached Live client keeps both scrollback and the final screen when its command exits (#859)."""
    release = tmp_path / "release-scrollback"
    script = (
        "printf 'pane-attached-marker\\n'; "
        f"while [ ! -e {shlex.quote(str(release))} ]; do sleep 0.05; done; "
        "printf 'first-held-line\\n'; seq 1 300; "
        "printf '\\033[?1000h\\033[?1006h\\033[?2004h'; printf 'last-held-line\\n'"
    )
    sid = assert_ok(live.request("session.new", command=f"sh -c {shlex.quote(script)}", wait=True))["id"]
    try:
        live.wait_mounted(sid, "left")
        # Seeing the marker on the visible screen proves the pane's client has attached before the exit payload runs.
        live.wait_text(sid, r"(?m)^pane-attached-marker$")
        release.touch()
        live.wait_text(sid, HELD_PROMPT)
        text = assert_ok(live.request("session.text", sid, all=True))["text"]
        expected = "\n".join(["first-held-line", *map(str, range(1, 301)), "last-held-line"])
        assert expected in text, text
        assert "pane-attached-marker" in text, text
        assert sid in session_ids(live.tree())
    finally:
        assert_ok(live.request("session.close", sid))


@LIVE_IN_ANY_SHELL
@pytest.mark.covers("live.kill", "field:result.pane")
def test_kill_ends_the_daemon_and_closes_its_pane(live):
    sid = _live_session(live, split=True)
    daemon = _entries(live, sid)["right"]["daemon"]
    result = assert_ok(live.request(live.vocabulary.command("live.kill"), sid, pane="right", force=True))
    assert result == {"id": sid, "text": f"killed {daemon}", "pane": "right"}
    live.wait_tree(lambda tree: "hasSplit" not in find_session(tree, sid))
    _gone(live, daemon)
    assert_ok(live.request("session.close", sid))


# -- presentation streams


# The kinds a viewer from before status sounds lists, and one that plays them.
OLDER_KINDS = ["status", "hud", "notify", "context", "layout"]
SOUND_KINDS = [*OLDER_KINDS, "sound"]


def _hello(mode, kinds=OLDER_KINDS):
    return {
        "kind": "hello",
        "gen": 0,
        "rev": 0,
        "hello": {"version": PRESENTATION_VERSION, "kinds": kinds, "mode": mode},
    }


def _subscribed(live, sid, mode, kinds=OLDER_KINDS):
    stream = live.stream(live.vocabulary.command("live.present"), sid)
    assert stream.reply.ok, stream.reply.raw
    assert stream.reply.result == {"id": sid}
    stream.send(_hello(mode, kinds))
    hello = wait_frame(stream.read, "hello", stream.send)
    assert hello["hello"]["mode"] == mode and hello["hello"]["version"] == PRESENTATION_VERSION
    wait_frame(stream.read, "snapshot", stream.send)
    return stream, hello["gen"]


def _wait_leads(live, sid):
    """Until the left pane's attach client has reported its role: before that `session.lead` has no lead to take."""

    def leads(tree):
        left = next(s for s in find_session(tree, sid)["surfaces"] if s["kind"] == "left")
        assert left.get("lead") == "leader", f"lead {left.get('lead')!r}"
        return True

    live.wait_tree(leads)


def _follows(tree, sid, viewer):
    left = next(s for s in find_session(tree, sid)["surfaces"] if s["kind"] == "left")
    assert left.get("lead") == "follower", f"lead {left.get('lead')!r}; viewer's client: {viewer.output()[-500:]!r}"
    return True


def _presenters(live, sid, expected):
    def counted(tree):
        assert find_session(tree, sid).get("presenters") == expected
        return True

    counted.__qualname__ = f"presenters of {sid} reading {expected}"
    live.wait_tree(counted)


@LIVE_IN_ANY_SHELL
@pytest.mark.covers("live.present", "field:tree.session.presenters", streams=("live.present",))
def test_present_mirrors_a_live_session(live):
    sid = _live_session(live)
    stream, _ = _subscribed(live, sid, "mirror")
    with stream:
        _presenters(live, sid, {"mirrors": 1})
        assert_ok(live.request("session.status", sid, status="active"))
        assert wait_frame(stream.read, "status", stream.send)["status"]["status"] == "active"
    _presenters(live, sid, None)
    assert_ok(live.request("session.close", sid))


@LIVE
@pytest.mark.covers(
    "session.overlay.job.run",
    "field:tree.session.presenters",
    "field:tree.session.remoteOverlays",
    streams=("session.overlay.job.run",),
)
def test_job_run_serves_an_overlay_a_presenter_shows(live):
    sid = _live_session(live)
    endpoint = assert_ok(live.request(live.vocabulary.command("live.tree")))["remote"]["endpoint"]
    # Upstream hands an overlay to the presenter only while the pane's own client follows, and open it
    # locally otherwise; a viewer's client claiming the lead makes it follow.
    daemon = _entries(live, sid)["left"]["daemon"]
    with live.vocabulary.leading_client(live.instance, endpoint, daemon) as viewer:
        live.wait_tree(lambda tree: _follows(tree, sid, viewer))
        presenter, gen = _subscribed(live, sid, "presenter")
        with presenter:
            presenter.send({"kind": "presenter.acquire", "gen": gen, "rev": 1})
            wait_frame(presenter.read, "presenter.granted", presenter.send)
            _presenters(live, sid, {"mirrors": 0, "presenter": True})
            # Handed to the presenter to show, and run by a helper that claims the job.
            assert assert_ok(live.request("session.overlay.open", sid, command="true"))["id"] == sid
            job = wait_frame(presenter.read, "overlay.request", presenter.send)["overlay"]["job"]
            live.wait_tree(lambda tree: find_session(tree, sid).get("remoteOverlays") == [{}])
            assert find_session(live.tree(), sid)["overlay"] is False
            with live.stream("session.overlay.job.run", job) as helper:
                assert helper.reply.ok and helper.reply.result == {"id": job}
                context = helper.read()
                assert context["kind"] == "context" and context["context"]["command"] == "true"
                helper.send({"kind": "started"})
                helper.send({"kind": "exited", "code": 0})
                live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
            # A job is claimable once.
            with live.stream("session.overlay.job.run", job) as again:
                assert_error(again.reply, "job not claimable")
    _presenters(live, sid, None)
    assert_ok(live.request("session.close", sid))


@LIVE
@pytest.mark.covers("session.overlay.open", "session.overlay.resize", "field:tree.session.remoteOverlays")
def test_points_are_refused_for_an_overlay_a_presenter_shows(live):
    """Historical ratchet ID: an older presenter now gets percent-only origin requests, including point resizes.

    A local replica's refusal is covered by core presenter tests: creating one here would require successful SSH.
    """
    with _page_presenter(live) as (sid, presenter, _):
        assert_ok(live.request("session.overlay.open", sid, command="true", sizePercent=60, height=240))
        opened = wait_frame(presenter.read, "overlay.request", presenter.send)["overlay"]
        assert opened["sizePercent"] == 60
        assert_absent(opened, "width", "height")
        resized = live.request("session.overlay.resize", sid, sizePercent=40, height=360)
        assert_ok(resized)
        change = wait_frame(presenter.read, "overlay.resize", presenter.send)["overlay"]
        assert change["sizePercent"] == 40
        assert_absent(change, "width", "height")
        assert find_session(live.tree(), sid)["remoteOverlays"] == [{"sizePercent": 40}]
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "overlay.close", presenter.send)


@LIVE
@pytest.mark.since("0.40.0")
@pytest.mark.covers("session.overlay.open", "session.overlay.resize", "field:tree.session.remoteOverlays")
def test_remote_overlay_points_reach_capable_presenters(live):
    with _page_presenter(live, [*PAGE_PRESENTER_KINDS, "overlay.points"]) as (sid, presenter, _):
        assert_ok(live.request("session.overlay.open", sid, command="true", sizePercent=60, width=320, height=240))
        opened = wait_frame(presenter.read, "overlay.request", presenter.send)["overlay"]
        assert_subset(opened, {"sizePercent": 60, "width": 320, "height": 240})
        # Points merge axes; percent plus points replaces them; percent alone and full clear them.
        for args, expected, omitted in [
            ({"height": 360}, {"sizePercent": 60, "width": 320, "height": 360}, ()),
            ({"sizePercent": 40, "width": 420}, {"sizePercent": 40, "width": 420}, ("height",)),
            ({"sizePercent": 30}, {"sizePercent": 30}, ("width", "height")),
            ({"full": True}, {}, ("sizePercent", "width", "height")),
        ]:
            assert_ok(live.request("session.overlay.resize", sid, **args))
            change = wait_frame(presenter.read, "overlay.resize", presenter.send)["overlay"]
            assert_subset(change, expected)
            assert_absent(change, *omitted)
            percent = expected.get("sizePercent")
            assert find_session(live.tree(), sid)["remoteOverlays"] == ([{}] if percent is None else [{"sizePercent": percent}])
        assert_error(
            live.request("session.overlay.resize", sid, height=240),
            "session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too",
        )
        assert find_session(live.tree(), sid)["remoteOverlays"] == [{}]
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "overlay.close", presenter.send)


@LIVE
@pytest.mark.since("0.40.0")
@pytest.mark.covers("live.present", "session.overlay.open", "session.overlay.resize", streams=("live.present",))
@pytest.mark.parametrize("source", [{"command": "true"}, {"url": "https://example.com/points"}], ids=["program", "page"])
def test_older_presenter_gets_percent_without_points(live, source):
    with _page_presenter(live) as (sid, presenter, _):
        assert_ok(live.request("session.overlay.open", sid, **source, sizePercent=60, width=320, height=240))
        page = "url" in source
        body = "page" if page else "overlay"
        opened = wait_frame(presenter.read, "page.request" if page else "overlay.request", presenter.send)[body]
        assert opened["sizePercent"] == 60
        assert_absent(opened, "width", "height")
        assert_ok(live.request("session.overlay.resize", sid, height=360))
        resized = wait_frame(presenter.read, "page.command" if page else "overlay.resize", presenter.send)[body]
        assert resized["sizePercent"] == 60
        assert_absent(resized, "width", "height")
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "page.close" if page else "overlay.close", presenter.send)


@LIVE
@pytest.mark.since("0.40.0")
@pytest.mark.covers("live.present", "session.overlay.open", "session.overlay.resize", streams=("live.present",))
def test_remote_page_points_and_resizes_reach_presenter(live):
    with _page_presenter(live, [*PAGE_PRESENTER_KINDS, "overlay.points"]) as (sid, presenter, _):
        page_id, opened = _handed_over(
            live, sid, presenter, url="https://example.com/points", sizePercent=70, width=320, height=240
        )
        assert_subset(opened, {"width": 320, "height": 240, "sizePercent": 70})
        for seq, (args, expected, omitted) in enumerate([
            ({"height": 360}, {"sizePercent": 70, "width": 320, "height": 360}, ()),
            ({"sizePercent": 40, "width": 420}, {"sizePercent": 40, "width": 420}, ("height",)),
            ({"sizePercent": 30}, {"sizePercent": 30}, ("width", "height")),
            ({"full": True}, {}, ("sizePercent", "width", "height")),
        ], start=1):
            assert_ok(live.request("session.overlay.resize", sid, **args))
            changed = wait_frame(presenter.read, "page.command", presenter.send)["page"]
            assert_subset(changed, {"page": page_id, "seq": seq, "action": "resize", **expected})
            assert_absent(changed, *omitted)
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "page.close", presenter.send)


def _acquired(live, sid, kinds):
    """A presenter of `sid` whose hello lists `kinds`, granted the role."""
    presenter, gen = _subscribed(live, sid, "presenter", kinds)
    presenter.send({"kind": "presenter.acquire", "gen": gen, "rev": 1})
    wait_frame(presenter.read, "presenter.granted", presenter.send)
    _presenters(live, sid, {"mirrors": 0, "presenter": True})
    return presenter


def _sounds_before_fence(live, sid, stream, fence):
    """The `sound` payloads `stream` got before the context frame a `session.context set fence` publishes: the origin
    sends a sound while it answers `session.status`, so one sent by an earlier request precedes the fence."""
    assert_ok(live.request("session.context", sid, mode="set", text=fence))
    frames = []

    def read(timeout):
        frame = stream.read(timeout)
        frames.append(frame)
        return frame

    while wait_frame(read, "context", stream.send).get("context") != fence:
        pass
    return [frame.get("sound") for frame in frames if frame.get("kind") == "sound"]


@LIVE
@pytest.mark.covers("session.status", "live.present", "field:tree.session.presenters", streams=("live.present",))
def test_status_sound_goes_to_a_following_presenter(live):
    sid = _live_session(live)
    endpoint = assert_ok(live.request(live.vocabulary.command("live.tree")))["remote"]["endpoint"]
    # A status sound goes to the presenter only while the status pane's own client follows.
    daemon = _entries(live, sid)["left"]["daemon"]
    with live.vocabulary.leading_client(live.instance, endpoint, daemon) as viewer:
        live.wait_tree(lambda tree: _follows(tree, sid, viewer))
        with _acquired(live, sid, SOUND_KINDS) as presenter:
            assert_ok(live.request("session.status", sid, status="blocked", sound="Glass"))
            assert _sounds_before_fence(live, sid, presenter, "fence 1") == [{"name": "Glass"}]
            # entering blocked without a sound sends the marker the presenter plays as its own blocked sound
            assert_ok(live.request("session.status", sid, status="idle"))
            assert_ok(live.request("session.status", sid, status="blocked"))
            assert _sounds_before_fence(live, sid, presenter, "fence 2") == [{}]
            # staying blocked plays nothing
            assert_ok(live.request("session.status", sid, status="blocked"))
            assert _sounds_before_fence(live, sid, presenter, "fence 3") == []
        _presenters(live, sid, None)
        # a sound is never part of a snapshot, so a reconnect while blocked replays none
        with _acquired(live, sid, SOUND_KINDS) as presenter:
            assert _sounds_before_fence(live, sid, presenter, "fence 4") == []
        _presenters(live, sid, None)
        # a presenter whose hello omits the kind never gets one
        with _acquired(live, sid, OLDER_KINDS) as presenter:
            assert_ok(live.request("session.status", sid, status="idle"))
            assert_ok(live.request("session.status", sid, status="blocked", sound="Glass"))
            assert _sounds_before_fence(live, sid, presenter, "fence 5") == []
        _presenters(live, sid, None)
    assert_ok(live.request("session.close", sid))


@LIVE_IN_ANY_SHELL
@pytest.mark.covers("live.tree", "field:result.remote", "session.status")
def test_tree_rows_carry_the_session_status(live):
    sid = _live_session(live)
    tree_cmd = live.vocabulary.command("live.tree")

    def row(response):
        return next((session for session in assert_ok(response)["remote"]["sessions"] if session["id"] == sid), None)

    assert_ok(live.request("session.status", sid, status="blocked", color="#ff8800", shape="star"))
    blocked = row(live.wait_response(tree_cmd, None, lambda r: (row(r) or {}).get("status") == "blocked"))
    assert_subset(blocked, {"status": "blocked", "statusColor": "#ff8800", "statusShape": "star"})
    assert_ok(live.request("session.status", sid, status="idle"))
    idle = row(live.wait_response(tree_cmd, None, lambda r: row(r) is not None and "status" not in row(r)))
    assert_absent(idle, "status", "statusColor", "statusShape")
    assert_ok(live.request("session.close", sid))


# A viewer that draws the shapes added after upstream's first six (#898).
SHAPES_KINDS = [*OLDER_KINDS, "status.shapes"]


def _subscribed_snapshot(live, sid, kinds):
    """A mirror of `sid` whose hello lists `kinds`, and the snapshot it was sent."""
    stream = live.stream(live.vocabulary.command("live.present"), sid)
    assert stream.reply.ok, stream.reply.raw
    stream.send(_hello("mirror", kinds))
    wait_frame(stream.read, "hello", stream.send)
    return stream, wait_frame(stream.read, "snapshot", stream.send)


@LIVE_IN_ANY_SHELL
@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.status", "live.present", streams=("live.present",))
def test_a_viewer_without_status_shapes_gets_only_shapes_it_decodes(live):
    """An older viewer decodes only upstream's first six shapes, so the origin clears any other from its status and
    snapshot; a viewer listing `status.shapes` gets every shape as set (#898)."""
    sid = _live_session(live)
    older, _ = _subscribed(live, sid, "mirror")
    capable, _ = _subscribed(live, sid, "mirror", SHAPES_KINDS)
    with older, capable:
        for status, shape in [("blocked", "bolt"), ("active", "star"), ("completed", "triangle-down")]:
            assert_ok(live.request("session.status", sid, status=status, color="#ff8800", shape=shape))
            seen = wait_frame(older.read, "status", older.send)["status"]
            assert_subset(seen, {"status": status, "color": "#ff8800"})
            if shape == "star":
                assert seen["shape"] == "star"
            else:
                assert_absent(seen, "shape")
            assert_subset(wait_frame(capable.read, "status", capable.send)["status"], {"status": status, "shape": shape})
        # the shape set when a viewer subscribes reaches it in the snapshot, cleared for an older one alike
        late_older, snapshot = _subscribed_snapshot(live, sid, OLDER_KINDS)
        with late_older:
            assert_subset(snapshot["snapshot"]["status"], {"status": "completed", "color": "#ff8800"})
            assert_absent(snapshot["snapshot"]["status"], "shape")
        late_capable, snapshot = _subscribed_snapshot(live, sid, SHAPES_KINDS)
        with late_capable:
            assert_subset(snapshot["snapshot"]["status"], {"status": "completed", "shape": "triangle-down"})
    assert_ok(live.request("session.status", sid, status="idle"))
    assert_ok(live.request("session.close", sid))


@LIVE_IN_ANY_SHELL
@pytest.mark.since("0.39.1")
@pytest.mark.covers("live.tree", "field:result.remote", "session.companion.page", "session.companion.new")
def test_tree_lists_a_page_companion_without_panes_under_its_listed_parent(live, tmp_path):
    """A page companion owns no daemon, so the hostless tree lists it with no panes, naming its parent, beside that
    listed parent; a terminal companion is listed as any session, with its parent (#840)."""
    sid = _live_session(live)
    page_path = tmp_path / "report.html"
    page_path.write_text("<!doctype html><title>report</title><p>companion page</p>")
    page = assert_ok(live.request("session.companion.page", sid, html=str(page_path)))["id"]
    companion = assert_ok(live.request("session.companion.new", sid, noSelect=True))["id"]
    tree_cmd = live.vocabulary.command("live.tree")

    def rows(response):
        return {session["id"]: session for session in assert_ok(response)["remote"]["sessions"]}

    listed = rows(live.wait_response(tree_cmd, None, lambda r: {sid, page, companion} <= rows(r).keys()))
    assert_subset(listed[page], {"panes": [], "parent": sid, "content": "page", "cwd": listed[sid]["cwd"]})
    assert_absent(listed[page], "splitAxis", "status")
    assert_subset(listed[companion], {"parent": sid})
    assert_absent(listed[companion], "content")
    assert_absent(listed[sid], "parent", "content")
    assert [entry["pane"] for entry in listed[companion]["panes"]] == ["left"]
    assert_ok(live.request("session.close", sid))
    live.wait_tree(lambda tree: not {sid, page, companion} & set(session_ids(tree)))


@LIVE_IN_ANY_SHELL
@pytest.mark.since("0.39.1")
@pytest.mark.covers("live.present", "session.companion.new", streams=("live.present",))
def test_a_respawned_live_parent_publishes_its_respawn_to_a_presenter(live):
    """A parent whose shell exits while it has companions restarts it under the same daemon name; once a new leader
    runs under that name the origin publishes `pane.respawn` with the pane's count, under the epoch every snapshot
    carries, so a viewer whose attach exited cleanly can attach again (#840)."""
    sid = _live_session(live)
    companion = assert_ok(live.request("session.companion.new", sid, noSelect=True))["id"]
    daemon = _entries(live, sid)["left"]["daemon"]
    stream = live.stream(live.vocabulary.command("live.present"), sid)
    assert stream.reply.ok, stream.reply.raw
    with stream:
        stream.send(_hello("mirror"))
        wait_frame(stream.read, "hello", stream.send)
        respawns = wait_frame(stream.read, "snapshot", stream.send)["snapshot"]["respawns"]
        assert respawns["generations"] == {} and respawns["epoch"]
        assert_ok(live.request("session.type", sid, text="exit\n"))
        frame = wait_frame(stream.read, "pane.respawn", stream.send, timeout=30)
        assert frame["respawn"] == {"epoch": respawns["epoch"], "generations": {daemon: 1}}
    assert sid in session_ids(live.tree())

    def running(response):
        row = _rows(live, response, sid).get("left", {})
        assert row.get("observation") == "running", row
        return True

    live.wait_response(live.vocabulary.command("live.list"), None, running)
    # a later viewer's snapshot carries every count under the same epoch
    later = live.stream(live.vocabulary.command("live.present"), sid)
    with later:
        later.send(_hello("mirror"))
        wait_frame(later.read, "hello", later.send)
        snapshot = wait_frame(later.read, "snapshot", later.send)["snapshot"]
        assert snapshot["respawns"] == {"epoch": respawns["epoch"], "generations": {daemon: 1}}
    assert_ok(live.request("session.close", sid))
    live.wait_tree(lambda tree: not {sid, companion} & set(session_ids(tree)))


# -- pages handed to a presenter

PAGE_KINDS = ["page.request", "page.rejected", "page.close", "page.closed", "page.command", "page.state"]
# The kinds a presenter that shows pages lists, in upstream's order.
PAGE_PRESENTER_KINDS = [*SOUND_KINDS, "ask.form", "ask.other", *PAGE_KINDS]
NO_PAGES = "a viewer presents this session: an html overlay would open where nobody sees it"
PAGE_NO_RESULT = "no overlay result: the slot holds an html page"
PAGE_NO_READ = "no overlay to read: the slot holds an html page"


@contextlib.contextmanager
def _page_presenter(live, kinds=PAGE_PRESENTER_KINDS):
    """A Live session whose left pane follows a viewer's client, and a presenter of it whose hello lists `kinds`:
    `(sid, presenter stream, its generation)`. Upstream hands a page over only while the pane's own client follows."""
    sid = _live_session(live)
    endpoint = assert_ok(live.request(live.vocabulary.command("live.tree")))["remote"]["endpoint"]
    daemon = _entries(live, sid)["left"]["daemon"]
    with live.vocabulary.leading_client(live.instance, endpoint, daemon) as viewer:
        live.wait_tree(lambda tree: _follows(tree, sid, viewer))
        presenter, gen = _subscribed(live, sid, "presenter", kinds)
        with presenter:
            presenter.send({"kind": "presenter.acquire", "gen": gen, "rev": 1})
            wait_frame(presenter.read, "presenter.granted", presenter.send)
            _presenters(live, sid, {"mirrors": 0, "presenter": True})
            yield sid, presenter, gen
        _presenters(live, sid, None)
    assert_ok(live.request("session.close", sid))


def _page_file(tmp_path):
    path = tmp_path / "docs" / "index.html"
    path.parent.mkdir()
    path.write_text("<!doctype html><title>page</title><p>shown on another machine</p>")
    return path


def _handed_over(live, sid, presenter, **args):
    """Opens a page that goes to the presenter: its page id and the `page.request` the presenter got."""
    result = assert_ok(live.request("session.overlay.open", sid, **args))
    assert result["id"] == sid
    request = wait_frame(presenter.read, "page.request", presenter.send)["page"]
    assert request["page"] == result["pageID"]
    return result["pageID"], request


def _outcome(live, page_id):
    return assert_ok(live.request("session.overlay.result", page=page_id))["pageOutcome"]


@LIVE
@pytest.mark.covers("session.overlay.open")
def test_a_presenter_without_page_kinds_keeps_the_page_refusal(live, tmp_path):
    page = _page_file(tmp_path)
    with _page_presenter(live, SOUND_KINDS) as (sid, _, _):
        assert_error(live.request("session.overlay.open", sid, html=str(page)), NO_PAGES)
        node = find_session(live.tree(), sid)
        assert "remoteOverlays" not in node and node["overlay"] is False


@LIVE
@pytest.mark.covers("session.overlay.open")
def test_a_loopback_url_bound_for_the_presenter_is_refused(live):
    with _page_presenter(live) as (sid, _, _):
        assert_error(
            live.request("session.overlay.open", sid, url="http://127.0.0.1:8080/"),
            "a loopback url names this Mac, so the Mac presenting the session cannot load it",
        )
        assert "remoteOverlays" not in find_session(live.tree(), sid)


@LIVE
@pytest.mark.covers("session.overlay.open", "field:tree.session.remoteOverlays", "field:tree.session.htmlOverlays")
def test_a_page_goes_to_the_presenter_and_its_reports_reach_the_tree(live, tmp_path):
    page = _page_file(tmp_path)
    with _page_presenter(live) as (sid, presenter, gen):
        result = assert_ok(
            live.request("session.overlay.open", sid, html=str(page), cwd=str(page.parent), sizePercent=70)
        )
        page_id = result["pageID"]
        request = wait_frame(presenter.read, "page.request", presenter.send)["page"]
        assert_subset(
            request,
            {
                "page": page_id,
                "source": {"file": {"path": str(page), "grantRoot": str(page.parent)}},
                "identity": "index.html",
                "sizePercent": 70,
                "follow": False,
            },
        )
        live.wait_tree(
            lambda tree: find_session(tree, sid).get("remoteOverlays") == [{"sizePercent": 70, "pageID": page_id}]
        )
        node = find_session(live.tree(), sid)
        assert node["overlay"] is False, "nothing covers the session here"
        [shown] = node["htmlOverlays"]
        assert_subset(shown, {"id": page_id, "file": str(page), "cwd": str(page.parent), "remote": True})
        assert_absent(shown, "zoom", "hidden", "replica")
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "pending"}
        # what the presenter reports is what the tree shows
        state = {
            "page": page_id,
            "title": "Remote",
            "address": str(page.parent / "b.html"),
            "load": "loaded",
            "canGoBack": True,
            "canGoForward": False,
            "hidden": True,
            "appliedSeq": 0,
        }
        presenter.send({"kind": "page.state", "gen": gen, "rev": 2, "page": state})
        live.wait_tree(lambda tree: find_session(tree, sid).get("hiddenOverlays") == ["session"])
        assert_subset(
            find_session(live.tree(), sid)["htmlOverlays"][0],
            {"title": "Remote", "page": str(page.parent / "b.html"), "state": "loaded", "hidden": True, "remote": True},
        )
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "page.close", presenter.send)


@LIVE
@pytest.mark.covers(
    "session.overlay.reload",
    "session.overlay.navigate",
    "session.overlay.resize",
    "session.overlay.result",
    "session.overlay.text",
)
def test_origin_commands_on_a_remote_page_reach_the_presenter(live):
    with _page_presenter(live) as (sid, presenter, _):
        page_id, request = _handed_over(live, sid, presenter, url="https://example.com/a")
        # Swift's synthesized coding of an unlabeled associated value
        assert request["source"] == {"url": {"_0": "https://example.com/a"}}
        sent = [
            ("session.overlay.reload", {}, "reloadOriginal"),
            ("session.overlay.reload", {"current": True}, "reloadCurrent"),
            ("session.overlay.navigate", {"to": "back"}, "back"),
            ("session.overlay.navigate", {"to": "forward"}, "forward"),
            ("session.overlay.navigate", {"to": "browser"}, "browser"),
        ]
        for seq, (cmd, args, action) in enumerate(sent, start=1):
            assert assert_ok(live.request(cmd, sid, **args))["id"] == sid
            command = wait_frame(presenter.read, "page.command", presenter.send)["page"]
            assert command == {"page": page_id, "seq": seq, "action": action}, cmd
        assert assert_ok(live.request("session.overlay.resize", sid, sizePercent=40))["id"] == sid
        command = wait_frame(presenter.read, "page.command", presenter.send)["page"]
        assert command == {"page": page_id, "seq": len(sent) + 1, "action": "resize", "sizePercent": 40}
        resized = [{"sizePercent": 40, "pageID": page_id}]
        live.wait_tree(lambda tree: find_session(tree, sid).get("remoteOverlays") == resized)
        # the file manager is a file page's, a page runs no program, and it has no text here
        assert_error(live.request("session.overlay.navigate", sid, to="finder"), "show in Finder requires a file page")
        assert_error(live.request("session.overlay.result", sid), PAGE_NO_RESULT)
        assert_error(live.request("session.overlay.text", sid), PAGE_NO_READ)
        assert_ok(live.request("session.overlay.close", sid))


@LIVE
@pytest.mark.covers("session.overlay.close", "session.overlay.result")
def test_closing_or_rejecting_a_remote_page_ends_it_dismissed(live, tmp_path):
    page = _page_file(tmp_path)
    with _page_presenter(live) as (sid, presenter, gen):
        closed, _ = _handed_over(live, sid, presenter, html=str(page))
        assert assert_ok(live.request("session.overlay.close", sid))["id"] == sid
        assert wait_frame(presenter.read, "page.close", presenter.send)["page"] == {"page": closed}
        live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, closed) == {"pageID": closed, "outcome": "dismissed"}
        # the presenter refuses the next one: it ends at once, dismissed
        rejected, _ = _handed_over(live, sid, presenter, html=str(page))
        presenter.send({"kind": "page.rejected", "gen": gen, "rev": 2, "page": {"page": rejected, "reason": "busy"}})
        live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, rejected) == {"pageID": rejected, "outcome": "dismissed"}
        assert_error(live.request("session.overlay.close", sid), "no overlay")


def _claimed_page(live, sid, presenter, tmp_path):
    """A file page granted its folder, handed to the presenter and its channel claimed with the request's token:
    `(page id, page file, token, helper stream)`. The folder also holds a file larger than one chunk."""
    page = _page_file(tmp_path)
    (page.parent / "big.bin").write_bytes(bytes(range(256)) * 600)
    page_id, request = _handed_over(live, sid, presenter, html=str(page), cwd=str(page.parent))
    helper = live.stream("session.overlay.page.serve", request["token"])
    assert helper.reply.ok, helper.reply.raw
    assert helper.reply.result == {"id": page_id}
    return page_id, page, request["token"], helper


def _transfer(helper, request_id):
    """The frames answering resource `request_id`: its MIME type, its body and its failure."""
    mime, body = None, b""
    while True:
        frame = helper.read()
        assert frame["id"] == request_id, frame
        if frame["kind"] == "head":
            mime = frame["mimeType"]
        elif frame["kind"] == "chunk":
            data = base64.b64decode(frame["data"])
            assert len(data) <= 64 * 1024
            body += data
        elif frame["kind"] == "end":
            return mime, body, None
        elif frame["kind"] == "failure":
            return mime, body, frame["error"]
        else:
            raise AssertionError(frame)


def _until_closed(helper):
    """The `reply` frames a helper reads until the origin closes its channel."""
    replies = []
    while True:
        try:
            frame = helper.read()
        except NoResponse:
            return replies
        if frame.get("kind") == "reply":
            replies.append(frame)


@LIVE
@pytest.mark.covers(
    "session.overlay.page.serve",
    "session.overlay.submit",
    "session.overlay.result",
    streams=("session.overlay.page.serve",),
)
def test_page_serve_serves_a_page_a_presenter_shows(live, tmp_path):
    with _page_presenter(live) as (sid, presenter, _):
        page_id, page, token, helper = _claimed_page(live, sid, presenter, tmp_path)
        with helper:
            # a token claims once
            with live.stream("session.overlay.page.serve", token) as again:
                assert_error(again.reply, "page not claimable")
            helper.send({"kind": "resource", "id": 1, "path": str(page)})
            assert _transfer(helper, 1) == ("text/html", page.read_bytes(), None)
            big = page.parent / "big.bin"
            helper.send({"kind": "resource", "id": 2, "path": str(big)})
            assert _transfer(helper, 2)[1:] == (big.read_bytes(), None)
            helper.send({"kind": "resource", "id": 3, "path": str(page.parent.parent / "elsewhere.txt")})
            assert _transfer(helper, 3) == (None, b"", "file is outside the page's folder")
            # a bridge call runs here, against the page's own session: the submit closes the page and its channel
            submit = {"cmd": "session.overlay.submit", "args": {"value": "done"}}
            helper.send({"kind": "bridge", "id": 4, "request": json.dumps(submit)})
            assert wait_frame(presenter.read, "page.close", presenter.send)["page"] == {"page": page_id}
            replies = _until_closed(helper)
            assert all(json.loads(frame["response"])["ok"] is True for frame in replies), replies
        live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "submitted", "value": "done"}


# -- image, Markdown and text pages handed to a presenter (#891)

# A presenter that builds image, Markdown and text pages from the bytes it fetches lists `page.content`.
CONTENT_PRESENTER_KINDS = [*PAGE_PRESENTER_KINDS, "page.content"]
# A 1x1 picture.
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
TEXT_MIME = "text/plain; charset=utf-8"
NOT_A_PICTURE = "not a png, jpg, gif or webp image"


def _content_files(tmp_path):
    """A folder with a picture, a Markdown file showing it and a source file: `(folder, picture, readme, source)`."""
    folder = tmp_path / "notes"
    folder.mkdir()
    picture, readme, source = folder / "shot.png", folder / "README.md", folder / "main.go"
    picture.write_bytes(PNG)
    readme.write_text("# notes\n\n![s](shot.png)\n")
    source.write_text("package main\n")
    return folder, picture, readme, source


@LIVE
@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open", "field:tree.session.remoteOverlays", "field:tree.session.htmlOverlays")
def test_content_pages_go_only_to_a_presenter_listing_page_content(live, tmp_path):
    _, picture, readme, source = _content_files(tmp_path)
    # the origin sends each path standardized as upstream's Foundation does: macOS's /private/var/... as /var/...
    sent_picture, sent_readme, sent_source = (foundation_resolved(str(path)) for path in (picture, readme, source))
    opens = [
        ({"image": str(picture)}, {"image": {"path": sent_picture, "width": 1, "height": 1}}),
        ({"markdownFile": str(readme)}, {"markdown": {"path": sent_readme}}),
        ({"textFile": str(source), "line": 1}, {"text": {"path": sent_source, "line": 1}}),
        ({"textFile": str(source)}, {"text": {"path": sent_source}}),
    ]
    # an older presenter keeps the refusal, and nothing is reserved for it
    with _page_presenter(live) as (sid, _, _):
        for args, _ in opens:
            assert_error(live.request("session.overlay.open", sid, **args), NO_PAGES)
        node = find_session(live.tree(), sid)
        assert "remoteOverlays" not in node and node["overlay"] is False
    with _page_presenter(live, CONTENT_PRESENTER_KINDS) as (sid, presenter, _):
        for args, sent in opens:
            page_id, request = _handed_over(live, sid, presenter, **args)
            assert request["source"] == sent, args
            # the presenter sizes a picture for its own screen: the origin sends no size
            assert_absent(request, "sizePercent", "width", "height")
            reserved = {"pageID": page_id, **({"sizePercent": 95} if "image" in args else {})}
            live.wait_tree(lambda tree: find_session(tree, sid).get("remoteOverlays") == [reserved])
            node = find_session(live.tree(), sid)
            assert node["overlay"] is False, "nothing covers the session here"
            [shown] = node["htmlOverlays"]
            assert_subset(shown, {"id": page_id, "remote": True})
            assert_ok(live.request("session.overlay.close", sid))
            wait_frame(presenter.read, "page.close", presenter.send)
        # the file is read before anything is reserved
        missing = str(tmp_path / "notes" / "missing.png")
        assert_error(
            live.request("session.overlay.open", sid, image=missing),
            f"session.overlay.open: unreadable image: {missing}",
        )
        assert "remoteOverlays" not in find_session(live.tree(), sid)


@LIVE
@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.page.serve", streams=("session.overlay.page.serve",))
def test_page_serve_serves_a_content_pages_files_under_its_rules(live, tmp_path):
    folder, picture, readme, _ = _content_files(tmp_path)
    (folder / "plot").write_bytes(PNG)
    (folder / "notes.txt").write_text("secret")
    (folder / "fake.png").write_text("<svg/>")
    (tmp_path / "outside.png").write_bytes(PNG)
    with _page_presenter(live, CONTENT_PRESENTER_KINDS) as (sid, presenter, _):
        page_id, request = _handed_over(live, sid, presenter, markdownFile=str(readme))
        with live.stream("session.overlay.page.serve", request["token"]) as helper:
            assert helper.reply.ok, helper.reply.raw
            assert helper.reply.result == {"id": page_id}
            # its own file as text, and a file of its folder only as a picture, whatever its name
            for request_id, (path, expected) in enumerate([
                (readme, (TEXT_MIME, readme.read_bytes(), None)),
                (picture, ("image/png", PNG, None)),
                (folder / "plot", ("image/png", PNG, None)),
                (folder / "notes.txt", (None, b"", NOT_A_PICTURE)),
                (folder / "fake.png", (None, b"", NOT_A_PICTURE)),
                (tmp_path / "outside.png", (None, b"", "file is outside the page's folder")),
            ], start=1):
                helper.send({"kind": "resource", "id": request_id, "path": str(path)})
                assert _transfer(helper, request_id) == expected, path
            # a content page runs no control request
            helper.send({"kind": "bridge", "id": 9, "request": json.dumps({"cmd": "tree"})})
            assert helper.read() == {"kind": "failure", "id": 9, "error": "this page has no bridge"}
            # closed while its helper is still connected: upstream ends a page whose helper goes (see
            # test_losing_the_page_helper_takes_only_the_page_off_its_presenter), and the close ends the helper's channel
            assert_ok(live.request("session.overlay.close", sid))
            wait_frame(presenter.read, "page.close", presenter.send)
            _until_closed(helper)


@LIVE
@pytest.mark.since("0.41.0")
@pytest.mark.covers("live.present", "session.overlay.open", "session.overlay.resize", streams=("live.present",))
def test_a_point_resize_of_an_auto_sized_image_merges_on_the_presenter(live, tmp_path):
    _, picture, _, _ = _content_files(tmp_path)
    with _page_presenter(live, [*CONTENT_PRESENTER_KINDS, "overlay.points"]) as (sid, presenter, _):
        page_id, request = _handed_over(live, sid, presenter, image=str(picture))
        assert_absent(request, "sizePercent", "width", "height")
        # a point resize of the picture the presenter fitted sends only its axes, to merge into that fit; a resize
        # that replaces the size ends it
        for seq, (args, expected, omitted) in enumerate([
            ({"width": 600}, {"sizePercent": 95, "width": 600, "merge": True}, ("height",)),
            ({"height": 300}, {"sizePercent": 95, "height": 300, "merge": True}, ("width",)),
            ({"sizePercent": 60, "width": 500}, {"sizePercent": 60, "width": 500}, ("height", "merge")),
            ({"height": 200}, {"sizePercent": 60, "width": 500, "height": 200}, ("merge",)),
        ], start=1):
            assert_ok(live.request("session.overlay.resize", sid, **args))
            changed = wait_frame(presenter.read, "page.command", presenter.send)["page"]
            assert_subset(changed, {"page": page_id, "seq": seq, "action": "resize", **expected})
            assert_absent(changed, *omitted)
        assert_ok(live.request("session.overlay.close", sid))
        wait_frame(presenter.read, "page.close", presenter.send)


@LIVE
@pytest.mark.covers("session.overlay.page.serve", streams=("session.overlay.page.serve",))
def test_losing_the_page_helper_takes_only_the_page_off_its_presenter(live, adapter, tmp_path):
    with _page_presenter(live) as (sid, presenter, _):
        page_id, _, _, helper = _claimed_page(live, sid, presenter, tmp_path)
        helper.close()
        assert wait_frame(presenter.read, "page.close", presenter.send)["page"] == {"page": page_id}
        if adapter.has("pages-survive"):
            # the page waits, parked, for the next presenter, and still closes for good here
            parked = [{"pageID": page_id, "parked": True}]
            live.wait_tree(lambda tree: find_session(tree, sid).get("remoteOverlays") == parked)
            assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "pending"}
            assert_ok(live.request("session.overlay.close", sid))
        else:
            # upstream ends it
            live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "dismissed"}
        _presenters(live, sid, {"mirrors": 0, "presenter": True})


def _granted(live, sid, kinds=PAGE_PRESENTER_KINDS):
    """Another presenter of `sid` whose hello lists `kinds`, granted the role: `(stream, its generation)`."""
    presenter, gen = _subscribed(live, sid, "presenter", kinds)
    presenter.send({"kind": "presenter.acquire", "gen": gen, "rev": 1})
    wait_frame(presenter.read, "presenter.granted", presenter.send)
    return presenter, gen


def _parked(live, sid, entry):
    def parked(tree):
        assert find_session(tree, sid).get("remoteOverlays") == [entry]
        return True

    parked.__qualname__ = f"remoteOverlays of {sid} reading {entry}"
    live.wait_tree(parked)


@LIVE
@PAGES_SURVIVE
@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.close",
    "session.overlay.result",
    "field:tree.session.remoteOverlays",
    "field:tree.session.htmlOverlays",
)
def test_a_page_whose_presenter_goes_waits_parked_for_the_next_one(live, tmp_path):
    page = _page_file(tmp_path)
    with _page_presenter(live) as (sid, presenter, _):
        page_id, first = _handed_over(live, sid, presenter, html=str(page), cwd=str(page.parent), sizePercent=70)
        presenter.close()
        _parked(live, sid, {"sizePercent": 70, "pageID": page_id, "parked": True})
        assert_subset(find_session(live.tree(), sid)["htmlOverlays"][0], {"id": page_id, "remote": True, "parked": True})
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "pending"}
        # the next presenter is handed the same page again, under a fresh claim and without raising the session
        later, _ = _granted(live, sid)
        with later:
            again = wait_frame(later.read, "page.request", later.send)["page"]
            assert_subset(again, {"page": page_id, "source": first["source"], "sizePercent": 70, "follow": False})
            assert again["token"] != first["token"]
            _parked(live, sid, {"sizePercent": 70, "pageID": page_id})
            assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "pending"}
            assert_ok(live.request("session.overlay.close", sid))
            assert wait_frame(later.read, "page.close", later.send)["page"] == {"page": page_id}
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "dismissed"}


@LIVE
@PAGES_SURVIVE
@pytest.mark.covers("session.overlay.open", "session.overlay.close", "session.overlay.result")
def test_a_page_closed_while_parked_is_not_handed_on(live, tmp_path):
    page = _page_file(tmp_path)
    with _page_presenter(live) as (sid, presenter, _):
        closed, _ = _handed_over(live, sid, presenter, html=str(page))
        presenter.close()
        _parked(live, sid, {"pageID": closed, "parked": True})
        assert assert_ok(live.request("session.overlay.close", sid))["id"] == sid
        live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, closed) == {"pageID": closed, "outcome": "dismissed"}
        later, _ = _granted(live, sid)
        with later:
            # the first page the next presenter is handed is one opened now, never the closed one
            opened, _ = _handed_over(live, sid, later, url="https://example.com/a")
            assert opened != closed
            assert_ok(live.request("session.overlay.close", sid))
            assert wait_frame(later.read, "page.close", later.send)["page"] == {"page": opened}



@pytest.mark.since("0.40.0")
@pytest.mark.covers("live.attach.group")
def test_group_attach_validates_before_remote_discovery(ctl):
    command = ctl.vocabulary.command("live.attach.group")
    assert_error(ctl.request(command), f"{command} requires a host")
    assert_error(ctl.request(command, host="  "), f"{command} requires a host")
    assert_error(ctl.request(command, host="unused.invalid"), f"{command} requires a remote session")
    assert_error(ctl.request(command, "bad\nname", host="unused.invalid"), "invalid remote session")


@LIVE_IN_ANY_SHELL
@pytest.mark.since("0.40.0")
@pytest.mark.covers("live.tree", "field:result.remote", "session.companion.page", "session.companion.new")
def test_tree_parent_membership_lists_companions_in_sidebar_order(live, tmp_path):
    sid = _live_session(live)
    page_path = tmp_path / "membership.html"
    page_path.write_text("<!doctype html><title>membership</title>")
    page = assert_ok(live.request("session.companion.page", sid, html=str(page_path)))["id"]
    companion = assert_ok(live.request("session.companion.new", sid, noSelect=True))["id"]
    tree_cmd = live.vocabulary.command("live.tree")

    def rows(response):
        return {row["id"]: row for row in assert_ok(response)["remote"]["sessions"]}

    listed = rows(live.wait_response(tree_cmd, None, lambda r: {sid, page, companion} <= rows(r).keys()))
    members = listed[sid]["companions"]
    assert {member["id"] for member in members} == {page, companion}
    assert all(member["name"] == listed[member["id"]]["name"] for member in members)
    assert [member["id"] for member in members] == [
        row for row in session_ids(live.tree()) if row in {page, companion}
    ]
    assert_absent(listed[page], "companions")
    assert_absent(listed[companion], "companions")
    assert_ok(live.request("session.close", sid))
