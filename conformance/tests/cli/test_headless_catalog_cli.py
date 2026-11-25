"""What the CLI prints against a headless origin's catalog (upstream's agterm-server, `ngterm-headless` here): a
structural subcommand sent with `--op-id` and `--epoch` prints its receipt under the command's own output, a repeat
prints the same receipt alone and runs nothing, a refusal is the receipt on stderr, `catalog op` reads a receipt back
or fences an id it has not seen, and `version` names the server's catalog version. What an app with windows answers
is in test_remote_window_cli.py."""

import re
import time

import pytest
from agterm_conformance.match import assert_ok, session_ids
from agterm_conformance.plugin import Cli, Ctl

pytestmark = [
    pytest.mark.since("0.42.0"),
    pytest.mark.requires("posix-command-line", reason="the remote origin's sshd is POSIX only"),
    pytest.mark.requires("live-any-shell", reason="ngterm-headless runs every session Live, in any login shell"),
    pytest.mark.parametrize("remote_origin", ["headless"], indirect=True),
]


def _hello(origin):
    """The catalog's hello: its epoch and versions."""
    with origin.stream("catalog.subscribe") as stream:
        assert stream.reply.ok, stream.reply.raw
        stream.send({"kind": "hello", "hello": {"version": 1}})
        deadline = time.monotonic() + 30
        while (remaining := deadline - time.monotonic()) > 0:
            frame = stream.read(remaining)
            if frame.get("kind") == "hello":
                return frame["hello"]
        pytest.fail("no hello frame within 30s")


def _cli(adapter, remote_origin):
    if adapter.ctl_path is None:
        pytest.fail(f"{adapter.name} has no {adapter.ctl_name}: pass --ctl")
    return Cli(adapter.ctl_path, remote_origin.instance, adapter.ctl_name)


@pytest.mark.covers("cli:session new", "cli:catalog op", "field:result.receipt")
def test_an_operation_prints_its_receipt_and_catalog_op_reads_it_back(adapter, remote_origin):
    """Upstream's `SocketClient.formatReceipt`: the id, the outcome with the revision it reached, and the epoch."""
    cli, origin = _cli(adapter, remote_origin), Ctl(remote_origin.instance)
    epoch = _hello(origin)["epoch"]
    created = cli("session", "new", "--op-id", "cli-new", "--epoch", epoch)
    assert (created.returncode, created.stderr) == (0, ""), created
    sid, receipt = created.stdout.splitlines()
    assert re.fullmatch(rf"operation cli-new: applied at rev \d+ in epoch {re.escape(epoch)}", receipt), receipt
    before = session_ids(origin.tree())
    assert sid in before

    repeated = cli("session", "new", "--op-id", "cli-new", "--epoch", epoch)
    assert (repeated.returncode, repeated.stdout, repeated.stderr) == (0, f"{receipt}\n", ""), repeated
    assert session_ids(origin.tree()) == before
    looked = cli("catalog", "op", "cli-new")
    assert (looked.returncode, looked.stdout, looked.stderr) == (0, f"{receipt}\n", ""), looked
    assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("cli:session new", "cli:session split", "cli:catalog op")
def test_a_refused_or_fenced_operation_prints_its_receipt_as_the_error(adapter, remote_origin):
    cli, origin = _cli(adapter, remote_origin), Ctl(remote_origin.instance)
    epoch = _hello(origin)["epoch"]
    stale = cli("session", "new", "--op-id", "cli-stale", "--epoch", "an-old-epoch")
    refusal = f"the catalog is at epoch {epoch}; take its snapshot and send the command again"
    assert (stale.returncode, stale.stdout) == (1, ""), stale
    assert stale.stderr == f"error: operation cli-stale: refused in epoch {epoch}: {refusal}\n", stale.stderr

    sid = assert_ok(origin.request("session.new"))["id"]
    pinned = cli("session", "split", "visibility", "on", "--target", sid, "--op-id", "cli-pinned", "--epoch", epoch,
                 "--expect-panes", "00000000-0000-4000-8000-000000000000")
    assert (pinned.returncode, pinned.stdout) == (1, ""), pinned
    assert pinned.stderr == (f"error: operation cli-pinned: refused in epoch {epoch}: "
                             "the session's panes changed; take the catalog again\n"), pinned.stderr

    fenced = f"operation cli-late: fenced in epoch {epoch}, so it will not run\n"
    fence = cli("catalog", "op", "cli-late")
    assert (fence.returncode, fence.stdout, fence.stderr) == (0, fenced, ""), fence
    late = cli("session", "new", "--op-id", "cli-late", "--epoch", epoch)
    assert (late.returncode, late.stdout, late.stderr) == (1, "", f"error: {fenced}"), late
    assert_ok(origin.request("session.close", sid))


@pytest.mark.covers("cli:version", "version")
def test_version_names_the_servers_catalog_version(adapter, remote_origin):
    """Upstream's versions line, printed after the server's version when the server serves a catalog."""
    cli, origin = _cli(adapter, remote_origin), Ctl(remote_origin.instance)
    versions = _hello(origin)["versions"]
    result = cli("version")
    assert result.returncode == 0, result
    line = f"server: {versions['server']}, protocol: {versions['protocol']}, catalog: {versions['catalog']}"
    assert line in result.stdout.splitlines(), result.stdout
