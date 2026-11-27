"""What a build below 0.42.0 answers for the v0.42 commands and subcommands it registers ahead of their features: the
CLI lists each, a run with valid arguments reaches the app and prints its refusal, and nothing changes; never a
success, a hang or a crash. These are the build's own interim answers, so the module skips on a build at that level
(the reference too), which the `since("0.42.0")` tests hold instead (test_remote_window_cli.py and, in
tests/protocol, test_remote_window.py, test_companions.py and test_text_pages.py). The task that builds a command
removes its case here with its id in the ratchet; the task that raises the build's level removes the module."""

import re

import pytest
from agterm_conformance.match import assert_error, assert_ok, find_session, session_ids, window_ids, workspace_ids

LEVEL = "0.42.0"
RECEIPTS = "operation receipts are served only by ngterm-headless"
PANE = "6f1f3f0e-4a3b-4c58-9d0e-0d6f3c1f2a10"

LEAVES = [
    "session convert",
    "session detach",
    "session reattach",
    "session overlay wrap",
    "workspace detach",
    "workspace reattach",
    "window remote open",
    "window remote dismiss",
    "catalog subscribe",
    "catalog op",
    "server install",
]


@pytest.fixture(autouse=True, scope="module")
def _below_the_level(adapter):
    if adapter.profile.at_level(LEVEL):
        level = adapter.profile.parity_level
        pytest.skip(f"{adapter.name} is at {level}: the since({LEVEL}) tests hold these commands")


@pytest.fixture
def row(ctl):
    """A workspace with one session and a page file in the CLI's directory, by the words the cases name them with."""
    wid = assert_ok(ctl.request("workspace.new", name="registered ahead"))["id"]
    sid = assert_ok(ctl.request("session.new", workspace=wid))["id"]
    ctl.wait_mounted(sid, "left")
    page = ctl.instance.home / "registered-ahead.html"
    page.write_text("<html><body>page</body></html>")
    yield {"SESSION": sid, "WORKSPACE": wid, "PAGE": page.name}
    page.unlink(missing_ok=True)
    assert_ok(ctl.request("workspace.delete", wid))


def _holds(ctl):
    tree = ctl.tree()
    rows = [
        (node["id"], node["name"], bool(node.get("htmlOverlays")))
        for node in (find_session(tree, sid) for sid in session_ids(tree))
    ]
    return workspace_ids(tree), rows, window_ids(ctl.request("window.list"))


def _refused(result, error):
    """A server refusal: exit 1, `error: <text>` on stderr, nothing on stdout."""
    assert (result.returncode, result.stdout, result.stderr) == (1, "", f"error: {error}\n"), result.argv


def _failed(result, message):
    """A local failure: exit 1, `Error: <message>` on stderr and no usage block under it."""
    assert (result.returncode, result.stdout, result.stderr) == (1, "", f"Error: {message}\n"), result.argv


def test_help_lists_the_v042_subcommands_and_options(cli):
    for leaf in LEAVES:
        *groups, name = leaf.split()
        listing = cli.local(*groups, "--help")
        assert (listing.returncode, listing.stderr) == (0, ""), leaf
        assert re.search(rf"^  {name}\s", listing.stdout, re.MULTILINE), listing.stdout
        own = cli.local(*groups, name, "--help")
        assert (own.returncode, own.stderr) == (0, ""), leaf
        assert f"USAGE: {own.program} {leaf}" in own.stdout, own.stdout
    root = cli.local("--help").stdout
    for group in ("catalog", "server"):
        assert re.search(rf"^  {group}\s", root, re.MULTILINE), root
    options = cli.local("session", "overlay", "open", "--help").stdout
    for option in ("--page", "--op-id <op-id>", "--epoch <epoch>", "--expect-panes <expect-panes>"):
        assert re.search(rf"^  {option}\s", options, re.MULTILINE), options


@pytest.mark.parametrize(
    ("argv", "command"),
    [
        pytest.param(["session", "detach"], "session.detach", id="session-detach"),
        pytest.param(["session", "reattach", "--target", "SESSION"], "session.reattach", id="session-reattach"),
        pytest.param(["workspace", "detach"], "workspace.detach", id="workspace-detach"),
        pytest.param(["workspace", "reattach", "--target", "WORKSPACE"], "workspace.reattach",
                     id="workspace-reattach"),
    ],
)
def test_a_command_registered_ahead_of_its_feature_is_refused_by_name(cli, ctl, row, argv, command):
    argv = [row.get(word, word) for word in argv]
    before = _holds(ctl)
    refusal = f"{command} is not supported by this host"
    _refused(cli(*argv), refusal)
    raw = cli(*argv, "--json")
    assert (raw.returncode, raw.stderr) == (1, ""), raw
    assert raw.json() == {"ok": False, "error": refusal}
    assert _holds(ctl) == before


def test_the_catalog_commands_are_the_headless_servers_alone(cli, ctl, row):
    before = _holds(ctl)
    _refused(cli("catalog", "op", "op-1"), "catalog.op is served only by ngterm-headless")
    _refused(cli("catalog", "op", "a.b_c:d-1", "--epoch", "E", "--rev", "3"),
             "catalog.op is served only by ngterm-headless")
    # the stream ends with the refusal and writes no frame
    _failed(cli("catalog", "subscribe", input=""), "catalog.subscribe is served only by ngterm-headless")
    assert _holds(ctl) == before


def test_an_operation_id_is_refused_and_nothing_runs(cli, ctl, row):
    """Over the socket on any command, and from the one subcommand that sends one today."""
    sid = row["SESSION"]
    before = _holds(ctl)
    for cmd, target, args in [
        ("session.rename", sid, {"name": "renamed"}),
        ("workspace.new", None, {"name": "made"}),
        ("tree", None, {}),
    ]:
        response = ctl.request(cmd, target, opID="op-1", epoch="E", **args)
        assert_error(response, RECEIPTS)
        assert response.result is None, response.raw
    operation = ["--op-id", "op-1", "--epoch", "E"]
    for flags in (["--companion"], ["--page"], ["--page", "--expect-panes", PANE]):
        _refused(cli("session", "overlay", "open", "--html", row["PAGE"], *flags, *operation, "--target", sid),
                 RECEIPTS)
    assert _holds(ctl) == before
