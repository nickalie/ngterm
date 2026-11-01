"""The `zmx` group, `session.lead`/`session.restart` and the Live-sessions read-back.

Every group command, result key, tree key and error text goes through the instance's vocabulary
(`agterm_conformance.vocabulary`): the Rust build answers this group as `live`.

The module's shared instance runs without Live, so its daemons are only claimed, never observed: the inventory,
prune, the hostless `zmx.tree` and every refusal are checked there. The Live tests share one instance launched with
`restoreMode: live` seeded; a pane in it runs the account's zsh or fish inside a live session (a zmx daemon upstream,
a session of the Rust build's server), so a line typed there only prints a marker. Each test ends the sessions it
opens, which ends their daemons; the harness's stop then finds no process left in the instance's namespace (under the
vocabulary's directory variable, or the session server of the namespace).

`zmx.reset` is refused here without `--force` only: a confirmed reset quits the app and relaunches it outside the
harness (see the catalog exemption)."""

import base64
import contextlib
import itertools
import json
import os

import pytest
from agterm_conformance.client import NoResponse
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_frame, wait_until

UNKNOWN = "zzz"
LIVE = pytest.mark.requires(
    "live-sessions", reason="needs panes in zmx daemons, which upstream creates only for a zsh or fish login shell"
)
ATTRIBUTIONS = ("supervisor", "app", "orphaned", "unknown")
PRESENTATION_VERSION = 1
# The line a `--wait` pane holds on once its command ended (Ghostty's `wait-after-command`).
HELD_PROMPT = r"Process exited\. Press any key to close the terminal\."
_markers = itertools.count(1)


def _rows(ctl, response, sid):
    """This session's `zmx.list` rows by pane."""
    rows = assert_ok(response)[ctl.vocabulary.result_key]["entries"]
    return {row["pane"]: row for row in rows if row.get("sessionID") == sid}


def _entries(ctl, sid):
    return _rows(ctl, ctl.request(ctl.vocabulary.command("zmx.list")), sid)


def _initial_session(ctl):
    (sid,) = session_ids(ctl.tree())
    ctl.wait_mounted(sid, "left")
    return sid


# -- without Live


@pytest.mark.covers("zmx.list", "field:result.zmx")
def test_list_joins_each_pane_claim(ctl):
    sid = _initial_session(ctl)
    inventory = assert_ok(ctl.request(ctl.vocabulary.command("zmx.list")))[ctl.vocabulary.result_key]
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


@pytest.mark.covers("zmx.prune", "field:result.text", "field:result.affected")
def test_prune_finds_no_orphans(ctl):
    assert assert_ok(ctl.request(ctl.vocabulary.command("zmx.prune"))) == {"text": "no orphan daemons", "affected": 0}


@pytest.mark.covers("zmx.tree", "field:result.remote")
def test_tree_without_a_host_lists_no_attachable_session_outside_live(ctl):
    remote = assert_ok(ctl.request(ctl.vocabulary.command("zmx.tree")))["remote"]
    assert remote["sessions"] == []
    assert remote["presentation"] >= PRESENTATION_VERSION
    assert remote["endpoint"]["socketDirectory"] == ctl.instance.live_dir


@pytest.mark.covers("zmx.kill")
@pytest.mark.parametrize(
    ("target", "args", "error"),
    [
        pytest.param(None, {"pane": "left", "force": True}, "zmx.kill requires an explicit --target", id="no-target"),
        pytest.param("SID", {"force": True}, "zmx.kill requires --pane left|right", id="no-pane"),
        pytest.param("SID", {"pane": "scratch", "force": True}, "zmx.kill requires --pane left|right", id="scratch"),
        pytest.param("SID", {"pane": "left"}, "zmx.kill requires --force", id="no-force"),
        pytest.param(
            "active",
            {"pane": "left", "force": True},
            "zmx.kill needs a session id; 'active' is not accepted",
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
    assert_error(ctl.request(ctl.vocabulary.command("zmx.kill"), target, **args), error)


@pytest.mark.covers("zmx.kill")
def test_kill_refuses_a_daemon_that_is_not_running(ctl):
    sid = _initial_session(ctl)
    daemon = _entries(ctl, sid)["left"]["daemon"]
    kill = ctl.vocabulary.command("zmx.kill")
    assert_error(ctl.request(kill, sid, pane="left", force=True), f"{daemon} is not running")
    assert find_session(ctl.tree(), sid)["realized"] is True


@pytest.mark.covers("zmx.screen")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "zmx.screen requires a daemon name", id="no-name"),
        pytest.param({"name": "two words"}, "zmx.screen requires a daemon name", id="not-plain"),
        pytest.param({"name": "agterm-x", "all": True, "lines": 3}, "use either --all or --lines, not both", id="both"),
        pytest.param({"name": "agterm-x", "lines": 0}, "--lines must be greater than 0", id="zero-lines"),
        pytest.param({"name": "agterm-x"}, "could not read the zmx screen of {name}", id="no-daemon"),
    ],
)
def test_screen_rejects(ctl, args, error):
    assert_error(ctl.request(ctl.vocabulary.command("zmx.screen"), **args), ctl.vocabulary.text(error, **args))


@pytest.mark.covers("zmx.reset")
def test_reset_requires_force(ctl):
    assert_error(ctl.request(ctl.vocabulary.command("zmx.reset")), ctl.vocabulary.text("zmx.reset requires --force"))
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


@pytest.mark.covers("zmx.present")
def test_present_refuses_without_a_session(ctl):
    with ctl.stream(ctl.vocabulary.command("zmx.present")) as stream:
        assert_error(stream.reply, ctl.vocabulary.text("zmx.present requires a session"))


@pytest.mark.covers("zmx.present", "session.overlay.job.run", "session.overlay.page.serve")
@pytest.mark.parametrize(
    ("cmd", "target", "error"),
    [
        pytest.param(
            "zmx.present", "SID", "session is not live-backed, so nothing can be attached to it", id="present-not-live"
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
    cmd = ctl.vocabulary.command(cmd) if cmd.startswith("zmx.") else cmd
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

    live.wait_response(live.vocabulary.command("zmx.list"), None, running)
    return sid


def _gone(live, daemon):
    def gone(response):
        return all(row["daemon"] != daemon for row in assert_ok(response)[live.vocabulary.result_key]["entries"])

    live.wait_response(live.vocabulary.command("zmx.list"), None, gone)


@LIVE
@pytest.mark.covers(
    "zmx.list",
    "field:result.zmx",
    "field:tree.session.backedByZmx",
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
        live.vocabulary.command("zmx.list"),
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
@pytest.mark.covers("zmx.screen", "field:result.text")
def test_screen_reads_the_daemon_grid(live):
    sid = _live_session(live)
    marker = f"ZMARK{next(_markers)}"
    # The command line carries the marker split in two, so only the printed line matches it.
    assert_ok(live.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:5]} {marker[5:]}\n"))
    live.wait_text(sid, rf"(?m)^{marker}$")
    daemon = _entries(live, sid)["left"]["daemon"]
    screen = assert_ok(live.request(live.vocabulary.command("zmx.screen"), name=daemon))
    assert f"\n{marker}\n" in f"\n{screen['text']}\n"
    assert_ok(live.request("session.close", sid))
    _gone(live, daemon)


@LIVE
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
        live.vocabulary.command("zmx.list"),
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
    pane stay, `zmx.list` lists the same daemon, and a marker typed afterwards prints (#773)."""

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
@pytest.mark.covers("session.restart")
def test_restart_command_interrupted_by_ctrl_c_leaves_a_prompt(live):
    sid = _live_session(live)
    daemon = _entries(live, sid)["left"]["daemon"]
    assert_ok(live.request("session.restart", sid, paneID=_pane_id(live, sid), command="sleep 600"))
    _ctrl_c_leaves_a_prompt(live, sid, daemon)


@LIVE
@pytest.mark.since("0.39.1")
@pytest.mark.covers("session.new")
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

    live.wait_response(live.vocabulary.command("zmx.list"), None, ended)
    # only the exit prompt proves the hold: the pane closes a moment after its daemon reads as ended
    live.wait_text(holding, HELD_PROMPT)
    assert holding in session_ids(live.tree())
    live.wait_text(holding, r"(?m)^held-output$")
    assert_ok(live.request("session.close", holding))


@LIVE
@pytest.mark.covers("zmx.kill", "field:result.pane")
def test_kill_ends_the_daemon_and_closes_its_pane(live):
    sid = _live_session(live, split=True)
    daemon = _entries(live, sid)["right"]["daemon"]
    result = assert_ok(live.request(live.vocabulary.command("zmx.kill"), sid, pane="right", force=True))
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
    stream = live.stream(live.vocabulary.command("zmx.present"), sid)
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


@LIVE
@pytest.mark.covers("zmx.present", "field:tree.session.presenters", streams=("zmx.present",))
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
    endpoint = assert_ok(live.request(live.vocabulary.command("zmx.tree")))["remote"]["endpoint"]
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


POINTS_REMOTE = "--width/--height are not supported for an overlay shown on another Mac"


@LIVE
@pytest.mark.covers("session.overlay.open", "session.overlay.resize", "field:tree.session.remoteOverlays")
def test_points_are_refused_for_an_overlay_a_presenter_shows(live):
    """The presentation wire carries only a percent, so point sizes are refused before a reservation or a send."""
    sid = _live_session(live)
    endpoint = assert_ok(live.request(live.vocabulary.command("zmx.tree")))["remote"]["endpoint"]
    daemon = _entries(live, sid)["left"]["daemon"]
    with live.vocabulary.leading_client(live.instance, endpoint, daemon) as viewer:
        live.wait_tree(lambda tree: _follows(tree, sid, viewer))
        presenter, gen = _subscribed(live, sid, "presenter")
        with presenter:
            presenter.send({"kind": "presenter.acquire", "gen": gen, "rev": 1})
            wait_frame(presenter.read, "presenter.granted", presenter.send)
            _presenters(live, sid, {"mirrors": 0, "presenter": True})
            refused = live.request("session.overlay.open", sid, command="true", sizePercent=60, height=240)
            assert_error(refused, POINTS_REMOTE)
            node = find_session(live.tree(), sid)
            assert "remoteOverlays" not in node and node["overlay"] is False
            assert assert_ok(live.request("session.overlay.open", sid, command="true", sizePercent=60))["id"] == sid
            wait_frame(presenter.read, "overlay.request", presenter.send)
            live.wait_tree(lambda tree: find_session(tree, sid).get("remoteOverlays") == [{"sizePercent": 60}])
            resized = live.request("session.overlay.resize", sid, sizePercent=40, height=240)
            assert_error(resized, POINTS_REMOTE)
            assert find_session(live.tree(), sid)["remoteOverlays"] == [{"sizePercent": 60}]
            assert_ok(live.request("session.overlay.close", sid))
            wait_frame(presenter.read, "overlay.close", presenter.send)
    _presenters(live, sid, None)
    assert_ok(live.request("session.close", sid))


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
@pytest.mark.covers("session.status", "zmx.present", "field:tree.session.presenters", streams=("zmx.present",))
def test_status_sound_goes_to_a_following_presenter(live):
    sid = _live_session(live)
    endpoint = assert_ok(live.request(live.vocabulary.command("zmx.tree")))["remote"]["endpoint"]
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


@LIVE
@pytest.mark.covers("zmx.tree", "field:result.remote", "session.status")
def test_tree_rows_carry_the_session_status(live):
    sid = _live_session(live)
    tree_cmd = live.vocabulary.command("zmx.tree")

    def row(response):
        return next((session for session in assert_ok(response)["remote"]["sessions"] if session["id"] == sid), None)

    assert_ok(live.request("session.status", sid, status="blocked", color="#ff8800", shape="star"))
    blocked = row(live.wait_response(tree_cmd, None, lambda r: (row(r) or {}).get("status") == "blocked"))
    assert_subset(blocked, {"status": "blocked", "statusColor": "#ff8800", "statusShape": "star"})
    assert_ok(live.request("session.status", sid, status="idle"))
    idle = row(live.wait_response(tree_cmd, None, lambda r: row(r) is not None and "status" not in row(r)))
    assert_absent(idle, "status", "statusColor", "statusShape")
    assert_ok(live.request("session.close", sid))


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
    endpoint = assert_ok(live.request(live.vocabulary.command("zmx.tree")))["remote"]["endpoint"]
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


@LIVE
@pytest.mark.covers("session.overlay.page.serve", streams=("session.overlay.page.serve",))
def test_losing_the_page_helper_ends_only_the_page(live, tmp_path):
    with _page_presenter(live) as (sid, presenter, _):
        page_id, _, _, helper = _claimed_page(live, sid, presenter, tmp_path)
        helper.close()
        assert wait_frame(presenter.read, "page.close", presenter.send)["page"] == {"page": page_id}
        live.wait_tree(lambda tree: "remoteOverlays" not in find_session(tree, sid))
        assert _outcome(live, page_id) == {"pageID": page_id, "outcome": "dismissed"}
        _presenters(live, sid, {"mirrors": 0, "presenter": True})

