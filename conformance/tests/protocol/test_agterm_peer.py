"""Attaching a Mac that runs upstream agterm, not ngterm (`agterm-peer`, a Rust-only extension): `ngtermctl live tree
HOST`, `live attach HOST ID` and what an attached session then shows, against loopback machines whose far side answers
as agterm v0.41.1 does (`agterm_conformance.fake_agterm`), reached by the instance's own ssh through a real `sshd`
(`agterm_conformance.loopback_ssh`).

The module's viewer runs with the machines' ssh client first on its PATH. `studio` runs agterm alone; `both` runs agterm
and an ngterm (another instance of the build, Live on), so the listing's choice between them shows. Each test adds the
sessions it needs to the app on `studio` and closes the rows it attached; the app keeps no state between tests that a
test reads."""

import re
import shlex

import pytest
from agterm_conformance.agterm_mac import LISTING_HEADER, AgtermMac, AgtermWorld
from agterm_conformance.match import assert_ok, find_session, sessions
from agterm_conformance.plugin import Cli, Ctl, launch_options
from agterm_conformance.wait import wait_text

pytestmark = [
    pytest.mark.requires(
        "agterm-peer",
        reason="attaching an upstream agterm Mac is the Rust build's extension; agterm attaches only agterm",
    ),
    # the loopback machine's sshd is POSIX only; Windows lacks this claim, so `--must-pass agterm-peer` excuses the skip
    # there and holds the Windows viewer to test_agterm_peer_windows.py instead
    pytest.mark.requires(
        "posix-command-line",
        reason="a Windows viewer runs a remote pane's attach through viewer-run, not as a /bin/sh line",
    ),
]

# `peer::missing_phrase(Agterm)`, the header of an agterm listing and the title bar's note
MISSING = (
    "the agent chat view, Kitty clipboard paste, renames made there and page companions opened there later are not "
    "available"
)
GONE = "ngterm: remote session is gone"


@pytest.fixture(scope="module")
def world(tmp_path_factory):
    world = AgtermWorld(tmp_path_factory.mktemp("agterm-peer"))
    try:
        yield world
    finally:
        world.stop()


@pytest.fixture(scope="module")
def studio(world) -> AgtermMac:
    return world.agterm_mac("studio")


@pytest.fixture(scope="module")
def viewer(adapter, world, request):
    instance = adapter.launch(path_prefix=[world.client.bin], **launch_options(request))
    try:
        yield Ctl(instance)
    finally:
        adapter.stop(instance)


@pytest.fixture
def vcli(adapter, viewer) -> Cli:
    return Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)


@pytest.fixture
def fresh(studio, viewer):
    """The app on `studio` starts empty and every row a test attached is closed after it (which only detaches)."""
    studio.remove_all()
    before = {s["id"] for s in _sessions(viewer)}
    yield studio
    for session in _sessions(viewer):
        if session["id"] not in before:
            viewer.request("session.close", session["id"])
    studio.remove_all()


def _sessions(ctl):
    return list(sessions(ctl.tree()))


def _attach(vcli, host, session_id):
    """`ngtermctl live attach HOST ID`: the local row's id, which a create command echoes."""
    result = vcli("live", "attach", host, session_id)
    assert result.returncode == 0, (result.stdout, result.stderr)
    local = result.stdout.strip()
    assert re.fullmatch(r"[0-9A-F-]{36}", local), result.stdout
    # its pane reads back only once the row is realized
    Ctl(vcli.instance).wait_tree(lambda tree: find_session(tree, local).get("realized"))
    return local


def _zmx_attaches(mac, daemon):
    return [call for call in mac.calls("zmx") if call["argv"][:2] == ["attach", daemon]]


# -- the listing


def test_live_tree_lists_an_agterm_macs_sessions_under_a_header_naming_what_it_lacks(vcli, fresh):
    build = fresh.add_session("build", cwd="/Users/me/repo", split=True, foreground=["claude"])
    notes = fresh.add_session("notes", cwd="/Users/me/notes", context="writing")
    result = vcli("live", "tree", "studio")
    assert (result.returncode, result.stderr) == (0, ""), result.stdout
    assert result.stdout == (
        f"agterm on studio: {MISSING}\n"
        f"  Main/work/build (split horizontal)  [{build['id']}]  /Users/me/repo  claude\n"
        f"  Main/work/notes  [{notes['id']}]  /Users/me/notes  writing\n"
    )
    # the listing reached agterm's CLI with its own live group
    assert fresh.calls("agtermctl")[-1]["argv"] == ["zmx", "tree", "--json"]


def test_live_tree_json_stamps_the_agterm_peer_on_the_endpoint(vcli, fresh):
    build = fresh.add_session("build")
    result = vcli("live", "tree", "studio", "--json")
    assert result.returncode == 0, result.stderr
    remote = result.json()["result"]["remote"]
    assert remote["host"] == "studio"
    assert remote["endpoint"] == {**fresh.endpoint, "peer": "agterm"}
    assert [s["id"] for s in remote["sessions"]] == [build["id"]]
    assert remote["sessions"][0]["panes"] == build["panes"]


def test_an_empty_agterm_listing_names_the_app(vcli, fresh):
    result = vcli("live", "tree", "studio")
    assert (result.returncode, result.stdout, result.stderr) == (0, "no attachable sessions on studio (agterm)\n", "")


def test_a_session_whose_daemons_are_gone_is_not_listed(vcli, fresh):
    gone = fresh.add_session("gone")
    kept = fresh.add_session("kept")
    fresh.end_daemons(gone)
    remote = vcli("live", "tree", "studio", "--json").json()["result"]["remote"]
    assert [s["id"] for s in remote["sessions"]] == [kept["id"]]


def test_agterm_pin_reaches_agterm_and_ngterm_pin_refuses_a_mac_without_ngterm(vcli, fresh):
    build = fresh.add_session("build")
    pinned = vcli("live", "tree", "agterm:studio", "--json")
    assert pinned.returncode == 0, pinned.stderr
    remote = pinned.json()["result"]["remote"]
    assert remote["endpoint"]["peer"] == "agterm"
    assert [s["id"] for s in remote["sessions"]] == [build["id"]]
    # the pin reaches the app ssh is given the destination without: ssh connected to studio
    asked = len(fresh.calls("agtermctl"))
    ngterm_only = vcli("live", "tree", "ngterm:studio")
    assert ngterm_only.returncode != 0, ngterm_only.stdout
    assert "agterm" not in ngterm_only.stdout + ngterm_only.stderr, ngterm_only
    assert len(fresh.calls("agtermctl")) == asked, "ngterm:studio must not fall back to agterm's CLI"


def test_the_listing_runs_ssh_with_the_destination_alone(vcli, fresh, world):
    fresh.add_session("build")
    assert vcli("live", "tree", "agterm:studio").returncode == 0
    argv = world.client.invocations()[-1]
    assert "studio" in argv and "agterm:studio" not in argv, argv
    assert argv[:2] == ["-T", "-o"], argv
    assert "BatchMode=yes" in argv, argv
    command = shlex.split(argv[-1])
    assert command[:2] == ["/bin/sh", "-c"], command
    assert LISTING_HEADER in command[2] and "agtermctl zmx tree --json" in command[2], command


# -- one host that runs both apps


@pytest.fixture(scope="module")
def both(world, adapter, request):
    """`both` runs agterm and an ngterm: another instance of the build under test, Live on, with one live session."""
    mac = world.agterm_mac("both")
    origin = adapter.launch(settings={"restoreMode": "live"}, **launch_options(request))
    try:
        mac.install_ngtermctl(adapter.ctl_path, origin.state_dir)
        yield mac, Ctl(origin)
    finally:
        adapter.stop(origin)


def test_a_host_running_both_apps_answers_with_ngterm_unless_pinned_to_agterm(vcli, both):
    mac, _origin = both
    mac.remove_all()
    agterm_session = mac.add_session("from-agterm")
    plain = vcli("live", "tree", "both", "--json")
    assert plain.returncode == 0, plain.stderr
    remote = plain.json()["result"]["remote"]
    assert "peer" not in remote["endpoint"], remote["endpoint"]
    assert agterm_session["id"] not in {s["id"] for s in remote["sessions"]}
    assert not [c for c in mac.calls("agtermctl") if c["argv"][:2] == ["zmx", "tree"]], "ngterm answered first"

    text = vcli("live", "tree", "both")
    assert "agterm" not in text.stdout, text.stdout

    pinned = vcli("live", "tree", "agterm:both", "--json")
    assert pinned.returncode == 0, pinned.stderr
    remote = pinned.json()["result"]["remote"]
    assert remote["endpoint"]["peer"] == "agterm"
    assert [s["id"] for s in remote["sessions"]] == [agterm_session["id"]]


# -- attaching


def test_live_attach_runs_each_pane_through_the_macs_own_zmx(viewer, vcli, fresh, world):
    build = fresh.add_session("build")
    daemon = build["panes"][0]["daemon"]
    local = _attach(vcli, "studio", build["id"])
    session = find_session(viewer.tree(), local)
    assert session["name"] == "build"
    assert session["remoteHost"] == "studio"
    wait_text(viewer, local, re.escape(f"attached {daemon}"), timeout=30)

    call = _zmx_attaches(fresh, daemon)[0]
    # zmx's attach contract (upstream's `attachCommand`), its create-only guard after the daemon's name
    assert call["argv"][2:4] == ["/bin/sh", "-c"], call
    assert GONE in call["argv"][4], call
    env = call["env"]
    assert (env.get("ZMX_SESSION"), env.get("ZMX_SESSION_PREFIX"), env.get("ZMX_NO_DETACH_KEY")) == ("", "", "1"), env
    assert env.get("ZMX_DIR") == fresh.endpoint["socketDirectory"], env
    assert env.get("ZMX_MANAGED"), "the pane opts into explicit leadership"
    assert call["ngterm"] == [], "nothing of this build's own attach contract reaches agterm's zmx"

    # the zmx client's role report is read as this pane's lead
    viewer.wait_tree(lambda tree: find_session(tree, local)["surfaces"][0].get("lead") == "leader")

    # the pane is the Mac's shell: a typed line runs there
    viewer.request("session.type", local, text="echo $((6*7))-on-agterm\r")
    wait_text(viewer, local, r"42-on-agterm", timeout=30)

    # the attach ran ssh with a pty, quiet, to the destination
    attach = next(
        argv
        for argv in world.client.invocations()
        if argv[0] != "-G" and any("attach" in word and daemon in word for word in argv)
    )
    assert attach[0] == "-tt" and "LogLevel=ERROR" in attach and "studio" in attach, attach


def test_an_attached_agterm_split_arrives_split_with_both_panes_attached(viewer, vcli, fresh):
    build = fresh.add_session("pair", split=True)
    local = _attach(vcli, "studio", build["id"])
    for pane in build["panes"]:
        fresh.wait_call(lambda call, d=pane["daemon"]: call["argv"][:2] == ["attach", d], program="zmx")
    session = find_session(viewer.tree(), local)
    assert session.get("hasSplit") is True, session


def test_the_presentation_stream_opens_with_agterms_cli_and_mirrors_its_frames(viewer, vcli, fresh):
    build = fresh.add_session(
        "watched",
        snapshot={
            "status": {"status": "active", "blink": False},
            "context": "compiling",
        },
    )
    local = _attach(vcli, "studio", build["id"])
    fresh.wait_call(lambda call: call["argv"] == ["zmx", "present", build["id"]], program="agtermctl")
    hello = fresh.wait_received(build["id"], "hello")["hello"]
    assert hello["version"] == 1, hello

    viewer.wait_tree(lambda tree: find_session(tree, local).get("status") == "active")
    viewer.wait_tree(lambda tree: find_session(tree, local).get("context") == "compiling")

    fresh.push(build["id"], "context", context="linking")
    viewer.wait_tree(lambda tree: find_session(tree, local).get("context") == "linking")
    fresh.push(build["id"], "status", status={"status": "blocked", "blink": True})
    viewer.wait_tree(lambda tree: find_session(tree, local).get("status") == "blocked")


def test_a_notification_from_the_agterm_session_reaches_the_viewer(viewer, vcli, fresh):
    build = fresh.add_session("notifier")
    local = _attach(vcli, "studio", build["id"])
    fresh.wait_received(build["id"], "hello")
    baseline = viewer.events_baseline()
    fresh.push(build["id"], "notify", notify={"title": "Build", "body": "done on agterm", "source": "control"})
    viewer.wait_event(baseline, "notify", lambda event: "done on agterm" in str(event), timeout=30)


def test_an_agterm_hud_shows_on_the_attached_row(viewer, vcli, fresh):
    build = fresh.add_session("hud")
    local = _attach(vcli, "studio", build["id"])
    fresh.wait_received(build["id"], "hello")
    fresh.push(build["id"], "hud", hud={"spec": {"message": "deploying from agterm"}, "generation": 1})
    viewer.wait_tree(
        lambda tree: (find_session(tree, local).get("hud") or {}).get("message") == "deploying from agterm"
    )
    fresh.push(build["id"], "hud")
    viewer.wait_tree(lambda tree: not find_session(tree, local).get("hud"))


def test_an_overlay_agterm_hands_over_runs_its_job_through_agtermctl(viewer, vcli, fresh, world):
    build = fresh.add_session("overlaid")
    local = _attach(vcli, "studio", build["id"])
    fresh.wait_received(build["id"], "presenter.acquire")
    fresh.push(build["id"], "overlay.request", overlay={"job": "job-7", "follow": False, "wait": False})
    fresh.wait_call(lambda call: call["argv"] == ["session", "overlay", "run-job", "job-7"], program="agtermctl")
    viewer.wait_tree(lambda tree: find_session(tree, local).get("overlay") is True)
    kinds = [surface["kind"] for surface in find_session(viewer.tree(), local)["surfaces"]]
    assert "overlay" in kinds, kinds
    run = next(argv for argv in world.client.invocations() if any("run-job" in word for word in argv))
    assert run[0] == "-tt" and "studio" in run, run


def test_a_daemon_gone_before_the_attach_prints_the_gone_line_and_hands_back_no_shell(viewer, vcli, fresh):
    build = fresh.add_session("vanishing")
    fresh.vanish_on_attach(build)
    local = _attach(vcli, "studio", build["id"])
    wait_text(viewer, local, re.escape(GONE), timeout=30)
    wait_text(viewer, local, re.escape("ngterm: vanishing (left) on studio disconnected, exit 1"), timeout=30)
    text = assert_ok(viewer.request("session.text", local))["text"]
    assert "agterm$" not in text and "attached " not in text, text


def test_attach_refuses_a_session_the_mac_no_longer_lists(vcli, fresh):
    refused = vcli("live", "attach", "studio", "AAAAAAAA-0000-0000-0000-000000000000")
    assert refused.returncode != 0
    assert "no attachable session AAAAAAAA-0000-0000-0000-000000000000 on studio" in refused.stderr, refused.stderr
