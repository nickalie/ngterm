"""Pages of sessions attached from another machine (`live attach HOST ID`), against an origin that is a second instance
of the build behind the harness's own sshd (`remote_origin`).

A page the origin opens at `http://127.0.0.1:PORT/` names a server on the origin, so the viewer forwards a port of its
own loopback to it over ssh and loads the page there (`ngterm-core`'s `remote/page_tunnel.rs`). Upstream refuses such a
page instead. The origin and the viewer share this machine's loopback, so the origin's server port is always busy here,
and every forward takes a port of its own.

A top-level page session (`session.page`) of the origin attaches as a top-level row with its group (#930)."""

from http.server import BaseHTTPRequestHandler
from urllib.parse import urlsplit

import pytest
from agterm_conformance.loopback import serve
from agterm_conformance.match import assert_absent, assert_ok, assert_subset, find_session, session_ids
from agterm_conformance.plugin import Ctl

# the loopback machine's sshd is POSIX only
POSIX_SSHD = pytest.mark.requires(
    "posix-command-line",
    reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
)

TITLE = "Origin Board"
BODY = f"<!doctype html><title>{TITLE}</title><p>served on the origin</p>".encode()


class Board(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200 if self.path == "/board" else 404)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *args):
        pass


def _origin_session(origin):
    sid = assert_ok(origin.request("session.new"))["id"]
    origin.wait_mounted(sid, "left")
    return sid


def _attach(viewer, host, sid):
    """`live attach HOST ID` on the viewer: the local row's id, once its pane reads back."""
    local = assert_ok(viewer.request("live.attach", sid, host=host))["id"]
    viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
    return local


def _follows(tree, sid):
    (left,) = [surface for surface in find_session(tree, sid)["surfaces"] if surface["kind"] == "left"]
    assert left.get("lead") == "follower", left
    return True


def _loaded(viewer, local):
    def loaded(tree):
        pages = find_session(tree, local).get("htmlOverlays", [])
        assert len(pages) == 1, pages
        (page,) = pages
        assert page["state"] == "loaded" and page.get("title") == TITLE, page
        return True

    loaded.__qualname__ = f"the forwarded page loaded in {local}"
    (page,) = find_session(viewer.wait_tree(loaded), local)["htmlOverlays"]
    return page


@POSIX_SSHD
# The forward has no capability of its own; the build claims live-any-shell wherever it runs Live, and the reference,
# which refuses a loopback page bound for another machine, does not.
@pytest.mark.requires(
    "live-any-shell",
    reason="the origin's session must be Live in any login shell, and the loopback page forward is the Rust "
    "build's extension: upstream refuses the page",
)
@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_two_loopback_pages_opened_at_once_on_a_port_busy_here_each_load_on_a_port_of_their_own(
    remote_origin, remote_viewer
):
    origin = Ctl(remote_origin.instance)
    with serve(Board) as server:
        port = server.server_port
        before = set(session_ids(origin.tree()))
        sids = [_origin_session(origin), _origin_session(origin)]
        # the origin hands a page over only while its pane follows the viewer's lead, and its own window leads the
        # session it shows: it shows another one
        assert_ok(origin.request("session.select", sorted(before)[0]))
        locals_ = [_attach(remote_viewer, remote_origin.host, sid) for sid in sids]
        origin.wait_tree(lambda tree: all(_follows(tree, sid) for sid in sids))
        address = f"http://127.0.0.1:{port}/board"
        # back to back: the second forward is placed while the first one's ssh may not have bound its port yet
        for sid in sids:
            assert_ok(origin.request("session.overlay.open", sid, url=address))

        pages = [_loaded(remote_viewer, local) for local in locals_]
        forwarded = [urlsplit(page["url"]) for page in pages]
        ports = [url.port for url in forwarded]
        assert all(url.hostname == "127.0.0.1" and url.path == "/board" for url in forwarded), pages
        assert port not in ports, f"the origin's port {port} is busy here: {pages}"
        assert ports[0] != ports[1], f"two forwards on one port: {pages}"
        assert all(page.get("replica") is True for page in pages), pages
        tree = origin.tree()
        for sid in sids:
            (shown,) = find_session(tree, sid)["htmlOverlays"]
            assert (shown["url"], shown.get("remote")) == (address, True), "the origin reads back its own address"

        for local in locals_:
            assert_ok(remote_viewer.request("session.close", local))
        for sid in sids:
            assert_ok(origin.request("session.close", sid))


def _page_file(ctl, name):
    """A static page titled `name` in a folder of its own under the instance's root."""
    folder = ctl.instance.root / f"remote-page-{name}"
    folder.mkdir()
    path = folder / f"{name}.html"
    path.write_text(f"<!doctype html><title>{name}</title><body><p>{name}</p></body>")
    return path


def _page_loaded(viewer, local, title):
    def loaded(tree):
        pages = find_session(tree, local).get("htmlOverlays") or []
        return any(page["state"] == "loaded" and page.get("title") == title for page in pages)

    loaded.__qualname__ = f"the page {title} loaded in {local}"
    return viewer.wait_tree(loaded)


@pytest.mark.since("0.42.0")
@POSIX_SSHD
@pytest.mark.requires(
    "live-sessions", reason="the origin lists its sessions only in Live mode, and its terminal companion must be Live"
)
@pytest.mark.covers("live.tree", "live.attach.group", "session.page", "session.companion.new", "field:result.remote")
def test_a_top_level_page_session_attaches_from_another_machine_as_a_top_level_row_with_its_group(
    remote_origin, remote_viewer
):
    origin = Ctl(remote_origin.instance)
    host = remote_origin.host
    target = _origin_session(origin)
    root = assert_ok(origin.request("session.page", target, html=str(_page_file(origin, "dash"))))["id"]
    terminal = assert_ok(origin.request("session.companion.new", root, noSelect=True))["id"]
    origin.wait_mounted(terminal, "left")
    held = assert_ok(origin.request("session.companion.page", root, html=str(_page_file(origin, "report"))))["id"]
    local = None
    try:
        # the origin lists its own top-level page with its whole group and no daemon, and a page companion under it
        def listed(response):
            rows = {row["id"] for row in assert_ok(response)["remote"]["sessions"]}
            return {root, terminal, held} <= rows

        listing = origin.wait_response(origin.vocabulary.command("live.tree"), None, listed)
        rows = {row["id"]: row for row in assert_ok(listing)["remote"]["sessions"]}
        assert_subset(rows[root], {"content": "page", "panes": []})
        assert_absent(rows[root], "parent")
        assert [member["id"] for member in rows[root]["companions"]] == [terminal, held]
        assert_subset(rows[held], {"content": "page", "parent": root, "panes": []})

        reply = assert_ok(
            remote_viewer.request(remote_viewer.vocabulary.command("live.attach.group"), root, host=host)
        )
        local = reply["id"]
        tree = _page_loaded(remote_viewer, local, "dash")
        node = find_session(tree, local)
        assert_subset(node, {"content": "page", "remoteHost": host})
        assert_absent(node, "parent")
        members = [member["id"] for member in reply["remoteGroup"]["companions"]]
        assert len(members) == 2 and all(members), reply
        tree = _page_loaded(remote_viewer, members[1], "report")
        assert [find_session(tree, member).get("parent") for member in members] == [local, local]
        assert [find_session(tree, member).get("content") for member in members] == [None, "page"]

        # New Companion on the page's row has the origin make one under its page
        created = assert_ok(remote_viewer.request("session.companion.new", local, noSelect=True))
        assert created["parent"] == local
        origin_tree = origin.tree()
        under_root = [row for row in session_ids(origin_tree) if find_session(origin_tree, row).get("parent") == root]
        assert len(under_root) == 3, under_root
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        for sid in (root, target):
            assert_ok(origin.request("session.close", sid))


def _rows_under(tree, parent):
    return [sid for sid in session_ids(tree) if find_session(tree, sid).get("parent") == parent]


@pytest.mark.since("0.42.0")
@POSIX_SSHD
@pytest.mark.requires(
    "live-sessions", reason="the origin lists its sessions only in Live mode, and its terminal companion must be Live"
)
@pytest.mark.covers("live.attach", "live.tree", "session.page", "field:tree.session.parent")
def test_a_top_level_page_session_attaches_alone_and_its_companions_attach_under_its_row_later(remote_origin, launch):
    # a viewer in Live mode lists its own sessions, so its listing shows what it leaves out
    remote_viewer = launch(path_prefix=remote_origin.viewer_path, settings={"restoreMode": "live"})
    origin = Ctl(remote_origin.instance)
    host = remote_origin.host
    target = _origin_session(origin)
    root = assert_ok(origin.request("session.page", target, html=str(_page_file(origin, "alone"))))["id"]
    terminal = assert_ok(origin.request("session.companion.new", root, noSelect=True))["id"]
    origin.wait_mounted(terminal, "left")
    held = assert_ok(origin.request("session.companion.page", root, html=str(_page_file(origin, "later"))))["id"]
    local = None
    try:
        local = assert_ok(remote_viewer.request("live.attach", root, host=host))["id"]
        tree = _page_loaded(remote_viewer, local, "alone")
        assert_subset(find_session(tree, local), {"content": "page", "remoteHost": host})
        assert_absent(find_session(tree, local), "parent")
        assert _rows_under(tree, local) == []

        attached = [assert_ok(remote_viewer.request("live.attach", sid, host=host))["id"] for sid in (terminal, held)]
        tree = _page_loaded(remote_viewer, attached[1], "later")
        tree = remote_viewer.wait_tree(lambda tree: find_session(tree, attached[0]).get("realized"))
        assert _rows_under(tree, local) == attached
        assert [find_session(tree, row).get("content") for row in attached] == [None, "page"]

        # this machine's own listing leaves out every row it attached from another machine
        own = [sid for sid in session_ids(tree) if find_session(tree, sid).get("remoteHost") is None]
        listing = remote_viewer.wait_response(
            remote_viewer.vocabulary.command("live.tree"),
            None,
            lambda response: set(own) <= {row["id"] for row in assert_ok(response)["remote"]["sessions"]},
        )
        listed = {row["id"] for row in assert_ok(listing)["remote"]["sessions"]}
        assert own and not listed & {local, *attached, root, terminal, held}, listed
    finally:
        if local is not None:
            assert_ok(remote_viewer.request("session.close", local))
        for sid in (root, target):
            assert_ok(origin.request("session.close", sid))
