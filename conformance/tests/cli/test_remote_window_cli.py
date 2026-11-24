"""The subcommands of a remote window and its headless server (`catalog`, `window remote`, `session|workspace
detach|reattach`, `server install`), and the other v0.42 leaves and options whose refusals come before any
connection: `session convert`, `session overlay wrap` and `session overlay open --page`. A remote window needs ssh to a
headless server, which the suite's no-network constraints exclude, so the socket commands are run for what an app with
windows answers. `server install` is local-only and runs through `cli.local`; a destination ssh cannot be given is
refused before anything runs."""

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids

UNKNOWN = "zzzz"
OP_ID = "an operation id is 1 to 128 letters, digits, '.', '_', ':' or '-'"
NOT_ACTIVE = "a detached row is never 'active'; pass its id"
PAGE_FULL_SIZE = "--page is full size: it takes no --pane, --size-percent, --width or --height"

pytestmark = pytest.mark.since("0.42.0")


def _usage(result, message=None):
    """Refused before anything is sent: exit 64, `Error: ...` and the usage line on stderr."""
    assert (result.returncode, result.stdout) == (64, ""), (result.argv, result.returncode, result.stderr)
    assert result.stderr.startswith(f"Error: {message}\n" if message else "Error: "), result.stderr
    assert f"\nUsage: {result.program} " in result.stderr, result.stderr


def _refused(result, error):
    """A server refusal: exit 1, `error: <text>` on stderr, nothing on stdout."""
    assert (result.returncode, result.stdout, result.stderr) == (1, "", f"error: {error}\n"), result.argv


def _failed(result, message):
    """A local failure: exit 1, `Error: <message>` on stderr and no usage block under it."""
    assert (result.returncode, result.stdout, result.stderr) == (1, "", f"Error: {message}\n"), result.argv


def _server_only(cli, command):
    return cli.vocabulary.text(f"{command} is served only by ngterm-headless")


# -- catalog


@pytest.mark.covers("cli:catalog subscribe", "catalog.subscribe")
def test_catalog_subscribe_reports_the_refusal_on_stderr_and_writes_no_frame(cli):
    _failed(cli("catalog", "subscribe", input=""), _server_only(cli, "catalog.subscribe"))


@pytest.mark.covers("cli:catalog op")
@pytest.mark.parametrize(
    ("args", "message"),
    [
        pytest.param([], None, id="no-id"),
        pytest.param(["a b"], OP_ID, id="spaced-id"),
        pytest.param(["a/b"], OP_ID, id="slash"),
        pytest.param(["x" * 129], OP_ID, id="long-id"),
        pytest.param(["op-1", "--epoch", "E"], "--epoch and --rev go together", id="epoch-alone"),
        pytest.param(["op-1", "--rev", "3"], "--epoch and --rev go together", id="rev-alone"),
        pytest.param(["op-1", "--epoch", "E", "--rev", "x"], None, id="rev-not-a-number"),
    ],
)
def test_catalog_op_validation(cli, args, message):
    _usage(cli("catalog", "op", *args), message)


@pytest.mark.covers("cli:catalog op", "catalog.op")
def test_catalog_op_is_the_servers_alone(cli):
    _refused(cli("catalog", "op", "op-1"), _server_only(cli, "catalog.op"))
    _refused(cli("catalog", "op", "a.b_c:d-1", "--epoch", "E", "--rev", "3"), _server_only(cli, "catalog.op"))


RECEIPTS = "operation receipts are served only by ngterm-headless"
OPERATION = ["--op-id", "op-1", "--epoch", "E"]


@pytest.mark.covers("cli:session new", "cli:session rename", "cli:session swap", "cli:session split close",
                    "cli:workspace new", "cli:workspace delete")
@pytest.mark.parametrize(
    "words",
    [
        pytest.param(["session", "new"], id="session-new"),
        pytest.param(["session", "rename", "x"], id="session-rename"),
        pytest.param(["session", "swap"], id="session-swap"),
        pytest.param(["session", "split", "close"], id="session-split-close"),
        pytest.param(["workspace", "new"], id="workspace-new"),
        pytest.param(["workspace", "delete", "--target", UNKNOWN], id="workspace-delete"),
    ],
)
def test_a_structural_subcommand_sends_its_operation_id_which_an_app_refuses(cli, words):
    """Upstream's `OperationOptions` on every structural leaf: the id and its epoch go together, and an app with windows
    keeps no receipts, so it refuses the command before it runs."""
    _usage(cli(*words, "--op-id", "op-1"), "--op-id and --epoch go together")
    _refused(cli(*words, *OPERATION), cli.vocabulary.text(RECEIPTS))


@pytest.mark.covers("cli:session scratch")
def test_session_scratch_takes_the_scratch_it_expects_only_with_an_operation(cli):
    _usage(cli("session", "scratch", "--expect-scratch", "none"), "--expect-scratch needs --op-id")
    _usage(cli("session", "scratch", "--expect-scratch", "x", *OPERATION),
           "--expect-scratch takes a scratch identity or none")
    _refused(cli("session", "scratch", "--expect-scratch", "none", *OPERATION), cli.vocabulary.text(RECEIPTS))


# -- window remote


@pytest.mark.covers("cli:window remote open", "window.remote.open")
def test_window_remote_open_needs_a_host_ssh_can_be_given(cli):
    _usage(cli("window", "remote", "open"))
    _refused(cli("window", "remote", "open", "a b"), "window.remote.open: not a host ssh can be given")


@pytest.mark.covers("cli:window remote dismiss", "window.remote.dismiss")
def test_window_remote_dismiss_acts_only_on_a_remote_window(cli):
    _refused(cli("window", "remote", "dismiss"), "window.remote.dismiss acts only on a remote window")
    _refused(cli("window", "remote", "dismiss", UNKNOWN), f"no such window: {UNKNOWN}")


# -- detach and reattach


@pytest.mark.covers("cli:session detach", "cli:workspace detach", "session.detach", "workspace.detach")
@pytest.mark.parametrize("group", ["session", "workspace"])
def test_detach_acts_only_on_a_remote_windows_rows(cli, ctl, group):
    before = session_ids(ctl.tree())
    _refused(cli(group, "detach"), f"{group}.detach acts only on a remote window")
    _refused(cli(group, "detach", "--target", UNKNOWN), f"no such {group}: {UNKNOWN}")
    assert session_ids(ctl.tree()) == before


@pytest.mark.covers("cli:session reattach", "cli:workspace reattach", "session.reattach", "workspace.reattach")
@pytest.mark.parametrize("group", ["session", "workspace"])
def test_reattach_names_a_detached_row(cli, group):
    # a hidden row has no default and is never `active`
    _usage(cli(group, "reattach"))
    _usage(cli(group, "reattach", "--target", "active"), NOT_ACTIVE)
    _usage(cli(group, "reattach", "--target", "ACTIVE"), NOT_ACTIVE)
    _refused(cli(group, "reattach", "--target", UNKNOWN), f"no such detached {group}: {UNKNOWN}")


# -- server install


@pytest.mark.covers("cli:server install")
def test_server_install_refuses_a_destination_ssh_cannot_be_given(cli):
    """Local-only: no `--socket`, and a bad destination is refused by name before ssh or a download runs."""
    for destination in ("a b", ""):
        _failed(
            cli.local("server", "install", destination),
            f"destination must be a host, user@host or an ssh alias, not '{destination}'",
        )
    _usage(cli.local("server", "install"))
    _usage(cli.local("server", "install", "--socket", "/tmp/nope.sock", "buildbox"))


# -- session convert


@pytest.mark.covers("cli:session convert")
def test_session_convert_needs_companion(cli):
    """The flag is required, so the verb names what the session becomes."""
    _usage(cli("session", "convert"), "session convert needs --companion")


@pytest.mark.covers("cli:session convert", "session.companion.convert")
def test_session_convert_prints_the_fresh_session(cli, ctl):
    wid = assert_ok(ctl.request("workspace.new"))["id"]
    sid = assert_ok(ctl.request("session.new", workspace=wid, cwd="/tmp"))["id"]
    result = cli("session", "convert", "--companion", "--target", sid)
    assert (result.returncode, result.stderr) == (0, ""), result
    fresh = result.stdout.strip()
    assert result.stdout == f"{fresh}\n" and fresh != sid
    tree = ctl.tree()
    assert session_ids(tree, wid) == [fresh, sid]
    assert find_session(tree, sid)["parent"] == fresh
    _refused(cli("session", "convert", "--companion", "--target", UNKNOWN), f"no such session: {UNKNOWN}")
    assert_ok(ctl.request("workspace.delete", wid))


# -- session overlay wrap, and open's --page, --op-id and --navigation


@pytest.mark.covers("cli:session overlay wrap")
@pytest.mark.parametrize(
    ("args", "message"),
    [
        pytest.param(["maybe"], "mode must be on, off or toggle", id="mode"),
        pytest.param([], None, id="needs-a-mode"),
        pytest.param(["on", "--pane", "scratch"], "--pane must be left or right", id="pane"),
    ],
)
def test_overlay_wrap_validation(cli, args, message):
    _usage(cli("session", "overlay", "wrap", *args), message)


@pytest.mark.covers("cli:session overlay open")
@pytest.mark.parametrize(
    ("args", "message"),
    [
        pytest.param(["--html", "/tmp/p.html", "--page", "--companion"], "--page cannot be combined with --companion",
                     id="page-and-companion"),
        pytest.param(["top", "--page"], "--page requires --html or --url", id="page-of-a-program"),
        pytest.param(["--html", "/tmp/p.html", "--page", "--pane", "right"], PAGE_FULL_SIZE, id="page-pane"),
        pytest.param(["--html", "/tmp/p.html", "--page", "--size-percent", "50"], PAGE_FULL_SIZE, id="page-size"),
        pytest.param(["--html", "/tmp/p.html", "--page", "--width", "300"], PAGE_FULL_SIZE, id="page-width"),
        pytest.param(["--html", "/tmp/p.html", "--op-id", "op-1", "--epoch", "E"],
                     "--op-id requires --companion or --page", id="op-id-on-an-overlay"),
        pytest.param(["--html", "/tmp/p.html", "--page", "--op-id", "op-1"], "--op-id and --epoch go together",
                     id="op-id-without-epoch"),
        pytest.param(["top", "--navigation"], "--navigation requires --html, --url or --markdown",
                     id="navigation-of-a-program"),
        pytest.param(["--markdown", "/tmp/r.md", "--page"], "--page requires --html or --url", id="markdown-page"),
        pytest.param(
            ["--markdown", "/tmp/r.md", "--js"],
            "--markdown takes no --companion, --page, --block, --wait, --js, --chromeless, --persistent, --browse or --cwd",
            id="markdown-js",
        ),
        pytest.param(
            ["--image", "/tmp/a.png", "--navigation"],
            "--image takes no --companion, --page, --block, --wait, --navigation, --js, --chromeless, --persistent, "
            "--browse or --cwd",
            id="image-navigation",
        ),
    ],
)
def test_overlay_open_page_and_operation_validation(cli, args, message):
    """A page session is a page, full size, and never also a companion; only a row keeps an operation receipt."""
    _usage(cli("session", "overlay", "open", *args), message)
