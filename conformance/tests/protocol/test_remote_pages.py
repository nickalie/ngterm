"""Loopback pages of a session attached from another machine (`live attach HOST ID`), against an origin that is a second
instance of the build behind the harness's own sshd (`remote_origin`): a page the origin opens at
`http://127.0.0.1:PORT/` names a server on the origin, so the viewer forwards a port of its own loopback to it over ssh
and loads the page there (`ngterm-core`'s `remote/page_tunnel.rs`). Upstream refuses such a page instead.

The origin and the viewer share this machine's loopback, so the origin's server port is always busy here, and every
forward takes a port of its own."""

from http.server import BaseHTTPRequestHandler
from urllib.parse import urlsplit

import pytest
from agterm_conformance.loopback import serve
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl

pytestmark = [
    # the loopback machine's sshd is POSIX only
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    # The forward has no capability of its own; the build claims live-any-shell wherever it runs Live, and the
    # reference, which refuses a loopback page bound for another machine, does not.
    pytest.mark.requires(
        "live-any-shell",
        reason="the origin's session must be Live in any login shell, and the loopback page forward is the Rust "
        "build's extension: upstream refuses the page",
    ),
]

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
