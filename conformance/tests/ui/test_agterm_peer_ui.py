"""The desktop's side of attaching a Mac that runs upstream agterm (`agterm-peer`, a Rust-only extension), driven as a
user does: File ▸ Attach Remote with the Mac in `remotes.conf`, the title bar of the attached row, and the remote row's
menu, whose New Session, Duplicate Session and End Session go to the Mac's `agtermctl`. The Mac is a loopback machine
answering as agterm v0.41.1 does (`agterm_conformance.fake_agterm`), reached by the instance's own ssh through a real
`sshd`."""

import re

import pytest
from agterm_conformance.agterm_mac import AgtermWorld
from agterm_conformance.match import sessions
from agterm_conformance.plugin import Cli
from agterm_conformance.wait import wait_text, wait_until

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

# `peer::limits_note(Agterm)`, the title bar badge's hover text
NOTE = (
    "Attached from agterm: the agent chat view, Kitty clipboard paste, renames made there and page companions opened "
    "there later are not available"
)

@pytest.fixture
def world(tmp_path):
    world = AgtermWorld(tmp_path)
    try:
        yield world
    finally:
        world.stop()

@pytest.fixture
def studio(world):
    return world.agterm_mac("studio")

@pytest.fixture
def viewer(launch, world):
    ctl = launch(path_prefix=[world.client.bin])
    (ctl.instance.state_dir / "config").mkdir(exist_ok=True)
    (ctl.instance.state_dir / "config" / "remotes.conf").write_text("agterm:studio Studio\n")
    return ctl

def _remote_rows(ctl):
    return [s for s in sessions(ctl.tree()) if s.get("remoteHost") is not None]

def _attach(cli, session_id):
    """`ngtermctl live attach agterm:studio ID`: the local row's id."""
    result = cli("live", "attach", "agterm:studio", session_id)
    assert result.returncode == 0, (result.stdout, result.stderr)
    return result.stdout.strip()


def _row(ui, name):
    return wait_until(
        lambda: next((row for row in ui.all("session-row") if ui.text(row) == name), None), timeout=ui.timeout
    )

def _attach_from_the_menu(ui, ctl, name):
    ui.menu("File", "Attach Remote…")
    ui.find("pick-palette")
    wait_until(lambda: name in ui.tree_dump(), timeout=ui.timeout)
    ui.press("return")
    ui.absent("pick-palette")
    return wait_until(lambda: next((s for s in _remote_rows(ctl) if s["name"] == name), None))

def test_attach_remote_lists_the_agterm_macs_sessions_and_attaches_the_chosen_one(viewer, ui_for, studio):
    build = studio.add_session("build")
    ui = ui_for(viewer)
    row = _attach_from_the_menu(ui, viewer, "build")
    assert row["remoteHost"] == "agterm:studio"
    wait_text(viewer, row["id"], re.escape(f"attached {build['panes'][0]['daemon']}"), timeout=30)
    # the pinned destination reached agterm's CLI, ssh given the host alone
    studio.wait_call(lambda call: call["argv"] == ["zmx", "present", build["id"]], program="agtermctl")

def test_the_title_bar_names_the_host_and_badges_agterm_with_what_it_lacks(adapter, viewer, ui_for, studio):
    """The title bar of a row attached from `agterm:studio` reads the host without its pin and an `agterm` badge whose
    hover text says what that Mac lacks; both must reach the accessibility tree, as every other title-bar label does."""
    build = studio.add_session("build")
    ui = ui_for(viewer)
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    local = _attach(cli, build["id"])
    viewer.wait_mounted(local, "left")
    wait_text(viewer, local, re.escape(f"attached {build['panes'][0]['daemon']}"), timeout=30)

    def labelled():
        dump = ui.tree_dump()
        assert re.search(r"name='[^']*\bagterm\b(?!:)", dump), dump
        return dump

    dump = wait_until(labelled, timeout=ui.timeout)
    assert NOTE in dump, dump

def _menu_on(ui, name, item):
    ui.mouse_click(_row(ui, name), button="right")
    ui.click(ui.menu_item(item))

def test_the_remote_rows_menu_ends_the_session_on_the_mac_after_confirming(adapter, viewer, ui_for, studio):
    build = studio.add_session("doomed")
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    local = _attach(cli, build["id"])
    ui = ui_for(viewer)
    _menu_on(ui, "doomed", "End Session on agterm:studio…")
    # the confirmation's buttons carry no ui id; Return takes its highlighted End Session once it is up
    wait_until(lambda: "button name='End Session'" in ui.tree_dump(), timeout=ui.timeout)
    ui.press("return")
    studio.wait_call(lambda call: call["argv"] == ["session", "close", "--target", build["id"], "--json"], "agtermctl")
    viewer.wait_tree(lambda tree: local not in {s["id"] for s in sessions(tree)})
    assert studio.session(build["id"]) is None

def test_new_session_on_the_mac_creates_it_with_agtermctl_and_attaches_it(adapter, viewer, ui_for, studio):
    build = studio.add_session("first")
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    _attach(cli, build["id"])
    ui = ui_for(viewer)
    _menu_on(ui, "first", "New Session on agterm:studio")
    studio.wait_call(lambda call: call["argv"] == ["restore", "mode", "--json"], "agtermctl")
    created = studio.wait_call(lambda call: call["argv"] == ["session", "new", "--no-select", "--json"], "agtermctl")
    assert created
    made = wait_until(lambda: next((s for s in studio.state()["sessions"] if s["id"] != build["id"]), None))
    row = wait_until(lambda: next((s for s in _remote_rows(viewer) if s["name"] == made["name"]), None))
    assert row["remoteHost"] == "agterm:studio"
    wait_text(viewer, row["id"], re.escape(f"attached {made['panes'][0]['daemon']}"), timeout=30)

def test_duplicate_session_on_a_remote_row_duplicates_it_on_the_mac(adapter, viewer, ui_for, studio):
    build = studio.add_session("source")
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    _attach(cli, build["id"])
    ui = ui_for(viewer)
    _menu_on(ui, "source", "Duplicate Session")
    studio.wait_call(
        lambda call: call["argv"] == ["session", "duplicate", "--target", build["id"], "--json"], "agtermctl"
    )
    copy = wait_until(
        lambda: next((s for s in studio.state()["sessions"] if s.get("duplicateOf") == build["id"]), None)
    )
    row = wait_until(lambda: next((s for s in _remote_rows(viewer) if s["name"] == copy["name"]), None))
    assert row["remoteHost"] == "agterm:studio"

def test_an_ask_from_the_agterm_session_is_drawn_here_and_its_answer_goes_back(adapter, viewer, ui_for, studio):
    """The presenter draws the ask agterm hands it and sends the chosen button back as `ask.resolve`."""
    build = studio.add_session("asking")
    cli = Cli(adapter.ctl_path, viewer.instance, adapter.ctl_name)
    local = _attach(cli, build["id"])
    viewer.wait_mounted(local, "left")
    hello = studio.wait_received(build["id"], "hello")["hello"]
    assert hello["mode"] == "presenter", hello
    studio.wait_received(build["id"], "presenter.acquire")
    owner = studio.wait_received(build["id"], "hello")["gen"]
    studio.push(
        build["id"],
        "ask.request",
        ask={
            "id": "deploy-1",
            "title": "Deploy from agterm?",
            "buttons": [{"id": "yes", "label": "Yes"}, {"id": "no", "label": "No"}],
            "style": "gui",
            "align": "center",
            "owner": owner,
        },
    )
    ui = ui_for(viewer)
    ui.find("ask-dialog")
    ui.click(ui.find(ui.ui_ids.expand("ask-button-{choice}", choice="yes"), name="Yes"))
    resolved = studio.wait_received(build["id"], "ask.resolve")["ask"]
    assert (resolved["id"], resolved.get("button")) == ("deploy-1", "yes"), resolved
    ui.absent("ask-dialog")
