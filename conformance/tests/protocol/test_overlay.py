"""The session overlay slot holding a caller's program or an HTML page, and the pane-scoped overlays.

Every program shown here only prints a marker and then waits in `cat` until its overlay is closed, so nothing runs
outside the instance. Every page is a static file written into the instance's own root, or served from this
process on 127.0.0.1: no page loads anything from the network."""

import itertools
import os
import threading
import uuid
from http.server import BaseHTTPRequestHandler

import pytest
from agterm_conformance.loopback import serve
from agterm_conformance.match import (
    ANY,
    assert_absent,
    assert_error,
    assert_ok,
    assert_subset,
    find_session,
    window_ids,
)

UNKNOWN = "zzz"
HOLD = "exec cat"
_pages = itertools.count(1)


def _new(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _split(ctl):
    sid = _new(ctl)
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")
    return sid


def _node(ctl, sid):
    return find_session(ctl.tree(), sid)


def _kinds(node):
    return {surface["kind"] for surface in node["surfaces"]}


def _open(ctl, sid, command=HOLD, **args):
    assert assert_ok(ctl.request("session.overlay.open", sid, command=command, **args))["id"] == sid


def _closed(ctl, sid, **args):
    assert assert_ok(ctl.request("session.overlay.close", sid, **args))["id"] == sid


def _result(ctl, sid, code, **args):
    """The status lands at the surface's teardown, which upstream runs after the slot reads closed."""

    def accept(response):
        return response.ok and response.result == {"id": sid, "exitCode": code}

    accept.__qualname__ = f"exit status {code}"
    ctl.wait_response("session.overlay.result", sid, accept, **args)


def _showing(needle):
    def accept(response):
        # `overlay not realized` is the moment between the open and the overlay's terminal coming up.
        assert response.ok, response.error
        return needle in response.result["text"]

    accept.__qualname__ = f"text containing {needle!r}"
    return accept


def _overlay_text(ctl, sid, needle, **args):
    return ctl.wait_response("session.overlay.text", sid, _showing(needle), **args).result["text"]


def _page(ctl, body="<p>static</p>", directory=None):
    """A static page in the instance's own root; `directory` is a subdirectory to put it in."""
    number = next(_pages)
    folder = ctl.instance.root / directory if directory else ctl.instance.root
    folder.mkdir(exist_ok=True)
    path = folder / f"page-{number}.html"
    path.write_text(f"<!doctype html><title>Page {number}</title><body>{body}</body>")
    return path, f"Page {number}"


def _loaded(ctl, sid, title, index=0):
    def loaded(tree):
        page = find_session(tree, sid)["htmlOverlays"][index]
        assert page["state"] == "loaded" and page.get("title") == title, page
        return True

    return find_session(ctl.wait_tree(loaded), sid)


def _open_page(ctl, sid, **args):
    path, title = _page(ctl)
    result = assert_ok(ctl.request("session.overlay.open", sid, html=str(path), **args))
    assert result["id"] == sid
    return path, title, result["pageID"]


def _outcome(ctl, page_id):
    return assert_ok(ctl.request("session.overlay.result", page=page_id))["pageOutcome"]


# -- a program


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.close",
    "session.overlay.result",
    "field:tree.session.overlay",
    "field:tree.session.overlaySizePercent",
    "field:tree.session.surfaces",
)
def test_program_overlay_covers_the_session(ctl):
    sid = _new(ctl)
    _open(ctl, sid)
    node = _node(ctl, sid)
    assert node["overlay"] is True
    assert_absent(node, "overlaySizePercent", "hud", "htmlOverlays", "paneOverlays")
    assert "overlay" in _kinds(node)
    assert_error(ctl.request("session.overlay.result", sid), "overlay still running")

    _closed(ctl, sid)
    node = _node(ctl, sid)
    assert node["overlay"] is False and "overlay" not in _kinds(node)
    assert_error(ctl.request("session.overlay.close", sid), "no overlay")


@pytest.mark.covers("session.overlay.result", "field:result.exitCode", "field:result.id")
def test_overlay_result_reports_the_exit_status(ctl):
    sid = _new(ctl)
    assert_error(ctl.request("session.overlay.result", sid), "no overlay result")
    _open(ctl, sid, "exit 7")
    ctl.wait_tree(lambda tree: find_session(tree, sid)["overlay"] is False)
    _result(ctl, sid, 7)

    # A new overlay replaces the previous status.
    _open(ctl, sid, "exit 3")
    ctl.wait_tree(lambda tree: find_session(tree, sid)["overlay"] is False)
    _result(ctl, sid, 3)


@pytest.mark.covers("session.overlay.text")
def test_overlay_text_reads_the_covering_program(ctl):
    sid = _new(ctl)
    assert_error(ctl.request("session.overlay.text", sid), "no overlay")
    _open(ctl, sid, "echo OVL-TEXT-1; echo OVL-TEXT-2; exec cat")
    text = _overlay_text(ctl, sid, "OVL-TEXT-2")
    assert "OVL-TEXT-1" in text
    assert _overlay_text(ctl, sid, "OVL-TEXT-1", all=True)
    last = assert_ok(ctl.request("session.overlay.text", sid, lines=1))["text"]
    assert "OVL-TEXT-2" in last and "OVL-TEXT-1" not in last
    # `session.text` reads the pane underneath, never the program drawn over it.
    assert "OVL-TEXT" not in assert_ok(ctl.request("session.text", sid))["text"]
    assert_error(ctl.request("session.overlay.text", sid, all=True, lines=2), "use either --all or --lines, not both")
    assert_error(ctl.request("session.overlay.text", sid, lines=0), "--lines must be greater than 0")
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open", "session.overlay.resize", "field:tree.session.overlaySizePercent")
def test_overlay_size_and_resize(ctl):
    sid = _new(ctl)
    assert_error(ctl.request("session.overlay.resize", sid, sizePercent=50), "no overlay")
    _open(ctl, sid, sizePercent=40)
    assert _node(ctl, sid)["overlaySizePercent"] == 40
    for args, size in [({"sizePercent": 75}, 75), ({"full": True}, None), ({"sizePercent": 1}, 1)]:
        assert assert_ok(ctl.request("session.overlay.resize", sid, **args))["id"] == sid
        node = _node(ctl, sid)
        assert node["overlay"] is True
        if size is None:
            assert_absent(node, "overlaySizePercent")
        else:
            assert node["overlaySizePercent"] == size
    for args, error in [
        ({"full": True, "sizePercent": 50}, "session.overlay.resize: --full is mutually exclusive with --size-percent"),
        ({}, "session.overlay.resize requires --size-percent, --width/--height or --full"),
        ({"sizePercent": 0}, "session.overlay.resize: --size-percent must be 1...100"),
        ({"sizePercent": 101}, "session.overlay.resize: --size-percent must be 1...100"),
        (
            {"sizePercent": 50, "pane": "left"},
            "session.overlay.resize: --pane is not supported (pane overlays are always full)",
        ),
    ]:
        assert_error(ctl.request("session.overlay.resize", sid, **args), error)
    assert _node(ctl, sid)["overlaySizePercent"] == 1
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize("requested", [0, 101])
def test_overlay_open_size_outside_its_range(ctl, requested):
    sid = _new(ctl)
    response = ctl.request("session.overlay.open", sid, command=HOLD, sizePercent=requested)
    assert_error(response, "session.overlay.open: --size-percent must be 1...100")
    assert _node(ctl, sid)["overlay"] is False


@pytest.mark.covers("session.overlay.open", "field:tree.session.active")
def test_overlay_opens_in_the_background_unless_it_follows(ctl):
    first, second = _new(ctl), _new(ctl)
    assert_ok(ctl.request("session.select", first))
    _open(ctl, second)
    assert _node(ctl, first)["active"] is True and _node(ctl, second)["overlay"] is True
    _closed(ctl, second)
    _open(ctl, second, follow=True)
    assert _node(ctl, second)["active"] is True
    _closed(ctl, second)


@pytest.mark.covers("session.overlay.open")
def test_overlay_runs_in_its_cwd(ctl):
    sid = _new(ctl)
    root = ctl.instance.root
    _open(ctl, sid, "pwd; exec cat", cwd=str(root), color="#203040")
    text = _overlay_text(ctl, sid, root.name)
    assert str(root) in text or str(root.resolve()) in text
    _closed(ctl, sid)


# -- sizes in points

POINTS_NEED_PERCENT = "session.overlay.open: --width/--height need --size-percent"
POINTS_NOT_POSITIVE = "session.overlay: --width/--height must be positive"
# The page strip and the rule under it, which a point height leaves out of the content and adds to the panel.
CHROME = 29


def _laid_out(ctl, sid):
    """The node once the session's layout gives its floating overlay a content size."""

    def sized(tree):
        node = find_session(tree, sid)
        return "overlayWidth" in node and "overlayHeight" in node

    return find_session(ctl.wait_tree(sized), sid)


def _detail(ctl, sid, chrome=0):
    """The session's detail area (width, height), read back through a 50% panel's content size; `chrome` is what a
    page's strip takes off the panel's height."""
    node = _laid_out(ctl, sid)
    assert node["overlaySizePercent"] == 50
    return node["overlayWidth"] * 2, (node["overlayHeight"] + chrome) * 2


def _content(ctl, sid, **args):
    """Resizes the overlay with `args`: the content size it reads back, or None without one."""
    assert assert_ok(ctl.request("session.overlay.resize", sid, **args))["id"] == sid
    node = _node(ctl, sid)
    if "overlayWidth" not in node:
        assert_absent(node, "overlayHeight")
        return None
    return node["overlayWidth"], node["overlayHeight"]


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.resize",
    "field:tree.session.overlayWidth",
    "field:tree.session.overlayHeight",
    "field:tree.session.overlaySizePercent",
)
def test_overlay_points_size_and_resize(ctl):
    sid = _new(ctl)
    assert_ok(ctl.request("session.select", sid))
    _open(ctl, sid, sizePercent=50)
    width, height = _detail(ctl, sid)
    # a point axis replaces the percent on its own; alone, points change only their axes
    assert _content(ctl, sid, height=100) == (pytest.approx(width * 0.5), pytest.approx(100))
    assert _content(ctl, sid, width=200) == (pytest.approx(200), pytest.approx(100))
    assert _node(ctl, sid)["overlaySizePercent"] == 50
    # with a percent, exactly the axes given; a percent alone drops them
    assert _content(ctl, sid, sizePercent=60, height=100) == (pytest.approx(width * 0.6), pytest.approx(100))
    assert _content(ctl, sid, sizePercent=60) == (pytest.approx(width * 0.6), pytest.approx(height * 0.6))
    # a point axis stops at 80% of the session
    assert _content(ctl, sid, width=100000, height=100000) == (pytest.approx(width * 0.8), pytest.approx(height * 0.8))
    # full drops them, and points alone then have no floating panel to change
    assert _content(ctl, sid, full=True) is None
    assert_absent(_node(ctl, sid), "overlaySizePercent")
    assert_error(
        ctl.request("session.overlay.resize", sid, height=100),
        "session.overlay.resize: --width/--height need a floating overlay; pass --size-percent too",
    )
    _closed(ctl, sid)
    # an open takes them beside its percent
    _open(ctl, sid, sizePercent=40, width=200)
    node = _laid_out(ctl, sid)
    assert (node["overlayWidth"], node["overlayHeight"]) == (pytest.approx(200), pytest.approx(height * 0.4))
    _closed(ctl, sid)
    assert_absent(_node(ctl, sid), "overlayWidth", "overlayHeight")


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.resize",
    "field:tree.session.overlayWidth",
    "field:tree.session.overlayHeight",
)
def test_page_points_leave_out_the_strip(ctl):
    sid = _new(ctl)
    assert_ok(ctl.request("session.select", sid))
    path, title = _page(ctl)
    assert_ok(ctl.request("session.overlay.open", sid, html=str(path), sizePercent=50))
    _loaded(ctl, sid, title)
    width, height = _detail(ctl, sid, CHROME)
    # a page's content is what it gets below its strip, whose height the panel adds on top of the points
    assert _content(ctl, sid, height=100) == (pytest.approx(width * 0.5), pytest.approx(100))
    assert _content(ctl, sid, height=100000) == (pytest.approx(width * 0.5), pytest.approx(height * 0.8 - CHROME))
    _closed(ctl, sid)
    # a chromeless page has no strip to leave out
    path, title = _page(ctl)
    assert_ok(ctl.request("session.overlay.open", sid, html=str(path), chromeless=True, sizePercent=50, height=100000))
    _loaded(ctl, sid, title)
    node = _laid_out(ctl, sid)
    assert (node["overlayWidth"], node["overlayHeight"]) == (pytest.approx(width * 0.5), pytest.approx(height * 0.8))
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open", "session.overlay.resize")
@pytest.mark.parametrize(
    ("cmd", "slot", "args", "error"),
    [
        pytest.param("session.overlay.open", None, {"height": 240}, POINTS_NEED_PERCENT, id="open-without-percent"),
        pytest.param(
            "session.overlay.open", None, {"sizePercent": 60, "height": 0}, POINTS_NOT_POSITIVE, id="open-zero"
        ),
        pytest.param(
            "session.overlay.open", None, {"sizePercent": 60, "width": -5}, POINTS_NOT_POSITIVE, id="open-negative"
        ),
        pytest.param(
            "session.overlay.open",
            None,
            {"pane": "left", "width": 300},
            "session.overlay.open: --pane is mutually exclusive with --width/--height",
            id="open-pane",
        ),
        pytest.param(
            "session.overlay.resize",
            50,
            {"full": True, "height": 240},
            "session.overlay.resize: --full is mutually exclusive with --width/--height",
            id="resize-full",
        ),
        pytest.param("session.overlay.resize", 50, {"height": 0}, POINTS_NOT_POSITIVE, id="resize-zero"),
        pytest.param(
            "session.overlay.resize", 50, {"sizePercent": 60, "width": -5}, POINTS_NOT_POSITIVE, id="resize-negative"
        ),
        pytest.param(
            "session.overlay.resize",
            50,
            {"pane": "left", "height": 240},
            "session.overlay.resize: --pane is not supported (pane overlays are always full)",
            id="resize-pane",
        ),
        pytest.param("session.overlay.resize", None, {"height": 240}, "no overlay", id="resize-nothing-open"),
    ],
)
def test_point_size_refusals(ctl, cmd, slot, args, error):
    sid = _new(ctl)
    if slot is not None:
        _open(ctl, sid, sizePercent=slot)
    request = {"command": HOLD, **args} if cmd == "session.overlay.open" else args
    assert_error(ctl.request(cmd, sid, **request), error)
    node = _node(ctl, sid)
    if slot is None:
        assert node["overlay"] is False
        assert_absent(node, "paneOverlays")
    else:
        assert node["overlaySizePercent"] == slot
        _closed(ctl, sid)


# -- pane overlays


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.close",
    "session.overlay.result",
    "session.overlay.text",
    "field:tree.session.paneOverlays",
    "field:tree.session.surfaces",
)
def test_pane_overlays_cover_one_pane_each(ctl):
    sid = _split(ctl)
    _open(ctl, sid, "echo PANE-RIGHT; exec cat", pane="right")
    node = _node(ctl, sid)
    assert node["paneOverlays"] == ["right"] and node["overlay"] is False
    assert "overlay-right" in _kinds(node) and "overlay-left" not in _kinds(node)
    assert_error(ctl.request("session.overlay.open", sid, command=HOLD, pane="right"), "pane overlay already open")
    _open(ctl, sid, pane="left")
    assert _node(ctl, sid)["paneOverlays"] == ["left", "right"]
    assert _overlay_text(ctl, sid, "PANE-RIGHT", pane="right")
    assert_error(ctl.request("session.overlay.result", sid, pane="right"), "overlay still running")
    # The session-wide slot stays free beside them.
    assert_error(ctl.request("session.overlay.text", sid), "no overlay")

    _closed(ctl, sid, pane="right")
    assert _node(ctl, sid)["paneOverlays"] == ["left"]
    _closed(ctl, sid, pane="left")
    assert_absent(_node(ctl, sid), "paneOverlays")
    assert_error(ctl.request("session.overlay.close", sid, pane="left"), "no overlay")

    # A fresh session: upstream records a closed overlay's status at its surface's teardown, which can land after
    # the next overlay in the same pane has exited and overwrite that one's.
    sid = _split(ctl)
    _open(ctl, sid, "exit 5", pane="right")
    ctl.wait_tree(lambda tree: "paneOverlays" not in find_session(tree, sid))
    _result(ctl, sid, 5, pane="right")


@pytest.mark.covers("session.overlay.open", "field:tree.session.paneOverlays")
def test_unsplit_session_takes_a_left_pane_overlay(ctl):
    sid = _new(ctl)
    _open(ctl, sid, pane="left")
    assert _node(ctl, sid)["paneOverlays"] == ["left"]
    _closed(ctl, sid, pane="left")


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize("hidden", [False, True], ids=["unsplit", "hidden-split"])
def test_pane_overlay_needs_a_rendered_pane(ctl, hidden):
    sid = _split(ctl) if hidden else _new(ctl)
    if hidden:
        # A hidden split still lays out its focused pane, so the right one is hidden only behind a focused left.
        assert_ok(ctl.request("session.focus", sid, pane="left"))
        assert_ok(ctl.request("session.split", sid, mode="off"))
    assert_error(ctl.request("session.overlay.open", sid, command=HOLD, pane="right"), "pane not visible")
    assert_absent(_node(ctl, sid), "paneOverlays")


# -- an html page


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.result",
    "session.overlay.close",
    "field:result.pageID",
    "field:result.pageOutcome",
    "field:tree.session.htmlOverlays",
    "field:tree.session.overlay",
)
def test_html_page_occupies_the_slot(ctl):
    sid = _new(ctl)
    path, title, page_id = _open_page(ctl, sid)
    node = _loaded(ctl, sid, title)
    assert node["overlay"] is True and "overlay" not in _kinds(node)
    assert_absent(node, "overlaySizePercent", "hud")
    (page,) = node["htmlOverlays"]
    assert_subset(
        page,
        {
            "id": page_id,
            "file": str(path),
            "state": "loaded",
            "title": title,
            "javascript": False,
            "chromeless": False,
            "persistent": False,
            "browse": False,
            "canGoBack": False,
            "canGoForward": False,
        },
    )
    assert_absent(page, "pane", "url", "cwd", "navigation", "error")
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "pending"}
    assert_error(ctl.request("session.overlay.result", sid), "no overlay result: the slot holds an html page")
    assert_error(ctl.request("session.overlay.text", sid), "no overlay to read: the slot holds an html page")
    assert_error(ctl.request("session.overlay.copy", sid), "no overlay to read: the slot holds an html page")

    _closed(ctl, sid)
    node = _node(ctl, sid)
    assert node["overlay"] is False
    assert_absent(node, "htmlOverlays")
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "dismissed"}


@pytest.mark.covers("session.overlay.submit", "field:result.pageOutcome")
@pytest.mark.parametrize("value", ["chosen", ""], ids=["value", "empty"])
def test_submit_answers_the_page_and_closes_it(ctl, value):
    sid = _new(ctl)
    _, title, page_id = _open_page(ctl, sid)
    _loaded(ctl, sid, title)
    assert assert_ok(ctl.request("session.overlay.submit", sid, value=value))["id"] == sid
    node = _node(ctl, sid)
    assert node["overlay"] is False
    assert_absent(node, "htmlOverlays")
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "submitted", "value": value}


@pytest.mark.covers("session.overlay.submit")
def test_submit_refusals(ctl):
    sid = _new(ctl)
    assert_error(ctl.request("session.overlay.submit", sid, value="x"), "no overlay")
    _open(ctl, sid)
    assert_error(ctl.request("session.overlay.submit", sid, value="x"), "the overlay is not an html page")
    _closed(ctl, sid)
    *_, page_id = _open_page(ctl, sid)
    assert_error(ctl.request("session.overlay.submit", sid), "session.overlay.submit requires a value")
    assert _outcome(ctl, page_id)["outcome"] == "pending"
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.reload", "session.overlay.navigate")
def test_reload_and_navigate_a_page(ctl):
    sid = _new(ctl)
    _, title, page_id = _open_page(ctl, sid)
    _loaded(ctl, sid, title)
    for args in ({}, {"current": True}):
        assert assert_ok(ctl.request("session.overlay.reload", sid, **args))["id"] == sid
        node = _loaded(ctl, sid, title)
        assert node["htmlOverlays"][0]["id"] == page_id
    assert_error(ctl.request("session.overlay.navigate", sid, to="back"), "no page to go back to")
    assert_error(ctl.request("session.overlay.navigate", sid, to="forward"), "no page to go forward to")
    for args in ({}, {"to": "sideways"}):
        assert_error(
            ctl.request("session.overlay.navigate", sid, **args),
            "session.overlay.navigate requires back, forward, browser or finder",
        )
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.reload", "session.overlay.navigate")
def test_page_commands_need_a_page(ctl):
    sid = _new(ctl)
    for cmd, args in [("session.overlay.reload", {}), ("session.overlay.navigate", {"to": "back"})]:
        assert_error(ctl.request(cmd, sid, **args), "no overlay")
        _open(ctl, sid)
        assert_error(ctl.request(cmd, sid, **args), "the overlay is not an html page")
        _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays", "field:tree.session.overlaySizePercent")
def test_page_flags_read_back(ctl):
    sid = _new(ctl)
    path, title = _page(ctl, directory="site")
    grant = path.parent
    result = assert_ok(
        ctl.request(
            "session.overlay.open",
            sid,
            html=str(path),
            cwd=str(grant),
            navigation=True,
            javascript=True,
            sizePercent=50,
            color="#102030",
        )
    )
    node = _loaded(ctl, sid, title)
    assert node["overlaySizePercent"] == 50
    assert_subset(
        node["htmlOverlays"],
        [{"id": result["pageID"], "file": str(path), "cwd": str(grant), "navigation": True, "javascript": True}],
    )
    _closed(ctl, sid)

    path, title = _page(ctl)
    assert_ok(ctl.request("session.overlay.open", sid, html=str(path), chromeless=True))
    page = _loaded(ctl, sid, title)["htmlOverlays"][0]
    assert page["chromeless"] is True
    assert_absent(page, "navigation")
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays", "field:tree.session.paneOverlays")
def test_page_in_a_pane_slot(ctl):
    sid = _split(ctl)
    path, title, page_id = _open_page(ctl, sid, pane="right")
    node = _loaded(ctl, sid, title)
    assert node["paneOverlays"] == ["right"] and node["overlay"] is False
    assert_subset(node["htmlOverlays"], [{"pane": "right", "id": page_id, "file": str(path)}])
    assert "overlay-right" not in _kinds(node)
    assert_error(
        ctl.request("session.overlay.result", sid, pane="right"), "no overlay result: the slot holds an html page"
    )
    assert assert_ok(ctl.request("session.overlay.submit", sid, pane="right", value="pane"))["id"] == sid
    assert_absent(_node(ctl, sid), "paneOverlays", "htmlOverlays")
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "submitted", "value": "pane"}


def _routes(routes):
    """A handler serving `routes`, path -> (status, headers, body), for one site on 127.0.0.1."""

    class Site(BaseHTTPRequestHandler):
        def do_GET(self):
            status, headers, body = routes.get(self.path, (404, {}, b""))
            self.send_response(status)
            for name, value in {"Content-Type": "text/html; charset=utf-8", **headers}.items():
                self.send_header(name, value)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    return Site


@pytest.fixture
def served_page():
    """A page on 127.0.0.1, served by this process."""
    body = b"<!doctype html><title>Served Page</title><body><p>served</p></body>"
    with serve(_routes({"/page.html": (200, {}, body)})) as server:
        yield f"http://127.0.0.1:{server.server_port}/page.html"


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize("browse", [False, True], ids=["pinned", "browse"])
def test_url_page_reads_back_its_address(ctl, served_page, browse):
    sid = _new(ctl)
    result = assert_ok(
        ctl.request("session.overlay.open", sid, url=served_page, **({"browse": True} if browse else {}))
    )
    node = _loaded(ctl, sid, "Served Page")
    assert node["overlay"] is True
    (page,) = node["htmlOverlays"]
    assert_subset(
        page, {"id": result["pageID"], "url": served_page, "browse": browse, "persistent": False, "page": ANY}
    )
    assert_absent(page, "file", "cwd")
    _closed(ctl, sid)


@pytest.fixture
def closing_site():
    """A page on 127.0.0.1 whose script closes its window as it loads."""
    body = b"<!doctype html><title>Closing Page</title><body><script>window.close()</script></body>"
    with serve(_routes({"/": (200, {}, body)})) as server:
        yield f"http://127.0.0.1:{server.server_port}/"


def _gone(sid):
    def gone(tree):
        node = find_session(tree, sid)
        assert node["overlay"] is False, node.get("htmlOverlays")
        return True

    gone.__qualname__ = "the page's overlay closed"
    return gone


@pytest.mark.covers("session.overlay.open", "session.overlay.result", "field:result.pageOutcome")
def test_a_script_page_closing_its_window_is_dismissed(ctl, closing_site):
    sid = _new(ctl)
    page_id = assert_ok(ctl.request("session.overlay.open", sid, url=closing_site, javascript=True))["pageID"]
    node = find_session(ctl.wait_tree(_gone(sid)), sid)
    assert_absent(node, "htmlOverlays")
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "dismissed"}


@pytest.mark.covers("session.overlay.open", "session.overlay.result", "field:result.pageOutcome")
def test_a_page_without_script_cannot_close_its_window(ctl, closing_site):
    sid = _new(ctl)
    page_id = assert_ok(ctl.request("session.overlay.open", sid, url=closing_site))["pageID"]
    _loaded(ctl, sid, "Closing Page")
    # a reload is a second whole load: a close the first one asked for has landed by the time it ends
    assert_ok(ctl.request("session.overlay.reload", sid))
    node = _loaded(ctl, sid, "Closing Page")
    assert node["overlay"] is True
    assert node["htmlOverlays"][0]["id"] == page_id
    assert _outcome(ctl, page_id) == {"pageID": page_id, "outcome": "pending"}
    _closed(ctl, sid)


@pytest.fixture
def redirecting_sites():
    """Two sites on 127.0.0.1: the first one's `/away` redirects to the second one's root."""
    second_body = b"<!doctype html><title>Second Site</title><body><p>second</p></body>"
    with serve(_routes({"/": (200, {}, second_body)})) as second:
        target = f"http://127.0.0.1:{second.server_port}/"
        with serve(_routes({"/away": (302, {"Location": target}, b"")})) as first:
            yield f"http://127.0.0.1:{first.server_port}/away", target


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize("browse", [False, True], ids=["pinned", "browse"])
def test_url_page_follows_a_cross_origin_redirect_only_when_browsing(ctl, redirecting_sites, browse):
    away, target = redirecting_sites
    sid = _new(ctl)
    result = assert_ok(
        ctl.request("session.overlay.open", sid, url=away, **({"browse": True} if browse else {}))
    )
    if browse:
        (page,) = _loaded(ctl, sid, "Second Site")["htmlOverlays"]
        assert_subset(page, {"id": result["pageID"], "url": away, "browse": True, "page": target})
        assert_absent(page, "error")
    else:

        def failed(tree):
            (page,) = find_session(tree, sid)["htmlOverlays"]
            assert page["state"] == "failed", page
            return True

        failed.__qualname__ = "the redirect refused"
        (page,) = find_session(ctl.wait_tree(failed), sid)["htmlOverlays"]
        assert_subset(
            page, {"id": result["pageID"], "url": away, "browse": False, "error": f"navigation blocked: {target}"}
        )
    _closed(ctl, sid)


@pytest.fixture(params=["127.0.0.1", "[::1]", "localhost"])
def loopback_page(request):
    """A page served by this process, addressed by one of the exact loopback hosts (localhost served on 127.0.0.1)."""
    host = request.param
    body = b"<!doctype html><title>Loopback Page</title><body><p>loopback</p></body>"
    with serve(_routes({"/page.html": (200, {}, body)}), "::1" if host == "[::1]" else "127.0.0.1") as server:
        yield f"http://{host}:{server.server_port}/page.html"


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_chromeless_loopback_url_page_opens(ctl, loopback_page):
    sid = _new(ctl)
    result = assert_ok(
        ctl.request("session.overlay.open", sid, url=loopback_page, chromeless=True, javascript=True, persistent=True)
    )
    assert result["id"] == sid
    node = _loaded(ctl, sid, "Loopback Page")
    (page,) = node["htmlOverlays"]
    assert_subset(
        page,
        {
            "id": result["pageID"],
            "url": loopback_page,
            "chromeless": True,
            "javascript": True,
            "persistent": True,
            "browse": False,
        },
    )
    _closed(ctl, sid)


def _recording(routes, seen):
    """`_routes` that also records every path asked for in `seen`."""
    site = _routes(routes)

    class Recording(site):
        def do_GET(self):
            seen.append(self.path)
            super().do_GET()

    return Recording


@pytest.fixture
def framing_sites():
    """A parent page on 127.0.0.1 framing one page of its own origin and one of a second site on 127.0.0.1; the
    paths each site was asked for."""
    other_seen, own_seen = [], []
    other = b"<!doctype html><title>Other</title><body><p>other origin frame</p></body>"
    with serve(_recording({"/other.html": (200, {}, other)}, other_seen)) as second:
        frame = f"http://127.0.0.1:{second.server_port}/other.html"
        parent = (
            '<!doctype html><title>Parent</title><body><p>parent page</p><iframe src="/inner.html"></iframe>'
            f'<iframe src="{frame}"></iframe></body>'
        ).encode()
        inner = b"<!doctype html><title>Inner</title><body><p>same origin frame</p></body>"
        routes = {"/parent.html": (200, {}, parent), "/inner.html": (200, {}, inner)}
        with serve(_recording(routes, own_seen)) as first:
            yield f"http://127.0.0.1:{first.server_port}/parent.html", own_seen, other_seen


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_chromeless_url_page_keeps_frames_on_its_origin(ctl, framing_sites):
    parent, own_seen, other_seen = framing_sites
    sid = _new(ctl)
    assert_ok(ctl.request("session.overlay.open", sid, url=parent))
    _loaded(ctl, sid, "Parent")
    # a page's load ends after its frames' loads
    assert "/other.html" in other_seen, "a page with a strip loads another origin's frame"
    _closed(ctl, sid)
    other_seen.clear()
    own_seen.clear()

    assert_ok(ctl.request("session.overlay.open", sid, url=parent, chromeless=True))
    (page,) = _loaded(ctl, sid, "Parent")["htmlOverlays"]
    assert page["chromeless"] is True
    assert "/inner.html" in own_seen, "the same-origin frame loads"
    assert other_seen == [], "a chromeless page never asks another origin for a frame"
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_chromeless_url_page_refuses_a_cross_origin_redirect(ctl, redirecting_sites):
    away, target = redirecting_sites
    sid = _new(ctl)
    result = assert_ok(ctl.request("session.overlay.open", sid, url=away, chromeless=True))

    def failed(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page["state"] == "failed", page
        return True

    failed.__qualname__ = "the redirect refused"
    (page,) = find_session(ctl.wait_tree(failed), sid)["htmlOverlays"]
    assert_subset(
        page, {"id": result["pageID"], "url": away, "chromeless": True, "error": f"navigation blocked: {target}"}
    )
    _closed(ctl, sid)


@pytest.fixture
def self_redirecting_sites():
    """A page on 127.0.0.1 whose script, once the test releases it, navigates to its own `/away`, which redirects to a
    second site's root; the page, the target and the release."""
    release = threading.Event()
    second_body = b"<!doctype html><title>Second Site</title><body><p>second</p></body>"
    start = (
        b"<!doctype html><title>Start</title><body><p>start</p><script>"
        b"addEventListener('load', () => fetch('/go').then(() => { location.href = '/away'; }));"
        b"</script></body>"
    )
    with serve(_routes({"/": (200, {}, second_body)})) as second:
        target = f"http://127.0.0.1:{second.server_port}/"
        site = _routes({"/start.html": (200, {}, start), "/away": (302, {"Location": target}, b"")})

        class Held(site):
            def do_GET(self):
                if self.path == "/go":
                    release.wait(60)
                    self.path = "/start.html"
                super().do_GET()

        try:
            with serve(Held) as first:
                yield f"http://127.0.0.1:{first.server_port}/start.html", target, release
        finally:
            release.set()


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_chromeless_url_page_fails_its_own_load_redirected_off_its_origin(ctl, self_redirecting_sites):
    start, target, release = self_redirecting_sites
    sid = _new(ctl)
    result = assert_ok(ctl.request("session.overlay.open", sid, url=start, chromeless=True, javascript=True))
    _loaded(ctl, sid, "Start")
    release.set()

    def failed(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page["state"] == "failed", page
        return True

    failed.__qualname__ = "the page's own redirect refused"
    (page,) = find_session(ctl.wait_tree(failed), sid)["htmlOverlays"]
    assert_subset(
        page, {"id": result["pageID"], "url": start, "chromeless": True, "error": f"navigation blocked: {target}"}
    )
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.result")
@pytest.mark.parametrize(
    ("page", "error"),
    [("nope", "invalid page id"), (str(uuid.uuid4()), "no such page")],
    ids=["invalid", "unknown"],
)
def test_page_result_needs_a_known_page(ctl, page, error):
    assert_error(ctl.request("session.overlay.result", page=page), error)


# -- refusals

# Absolute paths that name nothing, as the OS spells them: `/...` on Unix, on the current drive on Windows.
FILE = os.path.abspath(os.sep + os.path.join("agterm-conformance", "site", "page.html"))
FILESYSTEM_ROOT = os.path.abspath(os.sep)
URL = "http://127.0.0.1:9/page.html"
LOOPBACK_HOST = "session.overlay.open: --chromeless with --url requires a loopback host: 127.0.0.1, ::1 or localhost"


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize(
    ("args", "error"),
    [
        pytest.param({}, "session.overlay.open requires a command", id="no-command"),
        pytest.param({"command": ""}, "session.overlay.open requires a command", id="empty-command"),
        pytest.param({"html": FILE, "url": URL}, "session.overlay.open takes --html or --url, not both", id="html-url"),
        pytest.param(
            {"command": "true", "html": FILE}, "session.overlay.open takes a command or --html, not both", id="cmd-html"
        ),
        pytest.param(
            {"command": "true", "url": URL}, "session.overlay.open takes a command or --url, not both", id="cmd-url"
        ),
        pytest.param(
            {"html": FILE, "wait": True}, "session.overlay.open: --wait cannot be combined with --html", id="wait-html"
        ),
        pytest.param(
            {"url": URL, "wait": True}, "session.overlay.open: --wait cannot be combined with --url", id="wait-url"
        ),
        pytest.param(
            {"url": URL, "cwd": "/tmp"}, "session.overlay.open: --cwd cannot be combined with --url", id="cwd-url"
        ),
        pytest.param(
            {"url": "ftp://127.0.0.1/x"},
            "session.overlay.open: --url must be an absolute http or https URL",
            id="ftp-url",
        ),
        pytest.param(
            {"url": "page.html"}, "session.overlay.open: --url must be an absolute http or https URL", id="relative-url"
        ),
        pytest.param(
            {"command": "true", "navigation": True},
            "session.overlay.open: --navigation requires --html or --url",
            id="navigation-program",
        ),
        pytest.param(
            {"command": "true", "javascript": True},
            "session.overlay.open: --js requires --html or --url",
            id="js-program",
        ),
        pytest.param(
            {"command": "true", "chromeless": True},
            "session.overlay.open: --chromeless requires --html or a loopback --url",
            id="chromeless-program",
        ),
        pytest.param({"url": "http://example.com/", "chromeless": True}, LOOPBACK_HOST, id="chromeless-url"),
        pytest.param(
            {"url": URL, "chromeless": True, "browse": True},
            "session.overlay.open: --chromeless cannot be combined with --browse",
            id="chromeless-browse-url",
        ),
        pytest.param(
            {"html": FILE, "chromeless": True, "navigation": True},
            "session.overlay.open: --chromeless cannot be combined with --navigation",
            id="chromeless-navigation",
        ),
        pytest.param(
            {"command": "true", "persistent": True},
            "session.overlay.open: --persistent requires --url",
            id="persistent-program",
        ),
        pytest.param(
            {"html": FILE, "persistent": True},
            "session.overlay.open: --persistent requires --url",
            id="persistent-html",
        ),
        pytest.param(
            {"command": "true", "browse": True},
            "session.overlay.open: --browse requires --url",
            id="browse-program",
        ),
        pytest.param({"html": FILE, "browse": True}, "session.overlay.open: --browse requires --url", id="browse-html"),
        pytest.param(
            {"html": "site/page.html"},
            "session.overlay.open: html file must be an absolute path",
            id="relative-html",
        ),
        pytest.param(
            {"html": FILE, "cwd": "site"}, "session.overlay.open: cwd must be an absolute path", id="relative-cwd"
        ),
        pytest.param(
            {"html": FILE, "cwd": FILESYSTEM_ROOT},
            "session.overlay.open: cwd must not be / or the home directory",
            id="root-cwd",
        ),
        pytest.param(
            {"html": FILE, "cwd": FILE},
            "session.overlay.open: cwd must be a directory containing the html file",
            id="cwd-is-file",
        ),
        pytest.param(
            {"html": FILE, "cwd": os.path.abspath(os.sep + os.path.join("agterm-conformance", "other"))},
            "session.overlay.open: html file is outside cwd",
            id="outside-cwd",
        ),
        pytest.param({"command": "true", "color": "red"}, "invalid color: red (#rrggbb)", id="color"),
        pytest.param(
            {"command": "true", "pane": "scratch"}, "session.overlay: --pane must be left or right", id="pane"
        ),
        pytest.param(
            {"command": "true", "pane": "left", "sizePercent": 50},
            "session.overlay.open: --pane is mutually exclusive with --size-percent",
            id="pane-size",
        ),
        pytest.param(
            {"url": "http://127.0.0.2:9/page.html", "chromeless": True}, LOOPBACK_HOST, id="chromeless-other-loopback"
        ),
        pytest.param({"url": "http://127.1:9/page.html", "chromeless": True}, LOOPBACK_HOST, id="chromeless-short-ip"),
        pytest.param(
            {"url": URL, "chromeless": True, "navigation": True},
            "session.overlay.open: --chromeless cannot be combined with --navigation",
            id="chromeless-navigation-url",
        ),
        pytest.param(
            {"url": "localhost:5173", "chromeless": True},
            "session.overlay.open: --url must be an absolute http or https URL",
            id="chromeless-malformed-url",
        ),
    ],
)
def test_open_refusals(ctl, args, error):
    sid = _new(ctl)
    assert_error(ctl.request("session.overlay.open", sid, **args), error)
    node = _node(ctl, sid)
    assert node["overlay"] is False
    assert_absent(node, "paneOverlays", "htmlOverlays")


@pytest.mark.covers(
    "session.overlay.close",
    "session.overlay.result",
    "session.overlay.text",
    "session.overlay.hide",
    "session.overlay.show",
    "session.overlay.toggle",
)
@pytest.mark.parametrize(
    ("cmd", "args"),
    [
        ("session.overlay.close", {"pane": "scratch"}),
        ("session.overlay.result", {"pane": "scratch"}),
        ("session.overlay.text", {"pane": "scratch"}),
        ("session.overlay.reload", {"pane": "scratch"}),
        ("session.overlay.submit", {"pane": "scratch", "value": "x"}),
        ("session.overlay.hide", {"pane": "scratch"}),
        ("session.overlay.show", {"pane": "scratch"}),
        ("session.overlay.toggle", {"pane": "scratch"}),
    ],
)
def test_overlay_pane_selector_refusals(ctl, cmd, args):
    assert_error(ctl.request(cmd, _new(ctl), **args), "session.overlay: --pane must be left or right")


@pytest.mark.covers(
    "session.overlay.open",
    "session.overlay.close",
    "session.overlay.resize",
    "session.overlay.result",
    "session.overlay.text",
    "session.overlay.submit",
    "session.overlay.reload",
    "session.overlay.hide",
    "session.overlay.show",
    "session.overlay.toggle",
)
@pytest.mark.parametrize(
    ("cmd", "args"),
    [
        ("session.overlay.open", {"command": HOLD}),
        ("session.overlay.close", {}),
        ("session.overlay.resize", {"sizePercent": 50}),
        ("session.overlay.result", {}),
        ("session.overlay.text", {}),
        ("session.overlay.submit", {"value": "x"}),
        ("session.overlay.reload", {}),
        ("session.overlay.hide", {}),
        ("session.overlay.show", {}),
        ("session.overlay.toggle", {}),
    ],
)
def test_unknown_session_target(ctl, cmd, args):
    assert_error(ctl.request(cmd, UNKNOWN, **args), f"no such session: {UNKNOWN}")


# -- hidden overlays

VISIBILITY = ("session.overlay.hide", "session.overlay.show", "session.overlay.toggle")


def _left(ctl, cmd, sid, text, **args):
    """A hide, show or toggle answers the state it left the scoped overlays in."""
    assert assert_ok(ctl.request(cmd, sid, **args)) == {"id": sid, "text": text}


@pytest.mark.covers(
    "session.overlay.hide",
    "session.overlay.show",
    "session.overlay.toggle",
    "field:tree.session.hiddenOverlays",
    "field:tree.session.overlay",
    "field:tree.session.paneOverlays",
)
def test_hide_show_and_toggle_keep_the_overlays_running(ctl):
    sid = _split(ctl)
    marker = f"HIDDEN-{uuid.uuid4().hex[:8].upper()}"
    _open(ctl, sid, f"echo {marker}; exec cat")
    _open(ctl, sid, pane="right")
    _overlay_text(ctl, sid, marker)

    _left(ctl, "session.overlay.hide", sid, "hidden")
    node = _node(ctl, sid)
    assert node["hiddenOverlays"] == ["session", "right"]
    # occupancy stays: the slots still hold their programs, running
    assert node["overlay"] is True and node["paneOverlays"] == ["right"]
    assert marker in assert_ok(ctl.request("session.overlay.text", sid))["text"]
    assert_error(ctl.request("session.overlay.result", sid), "overlay still running")

    _left(ctl, "session.overlay.show", sid, "shown")
    assert_absent(_node(ctl, sid), "hiddenOverlays")
    _left(ctl, "session.overlay.toggle", sid, "hidden", pane="right")
    assert _node(ctl, sid)["hiddenOverlays"] == ["right"]
    # a bare toggle hides everything while anything is shown, then shows it all
    _left(ctl, "session.overlay.toggle", sid, "hidden")
    assert _node(ctl, sid)["hiddenOverlays"] == ["session", "right"]
    _left(ctl, "session.overlay.toggle", sid, "shown")
    assert_absent(_node(ctl, sid), "hiddenOverlays")
    _closed(ctl, sid, pane="right")
    _closed(ctl, sid)


@pytest.mark.covers("session.overlay.hide", "session.overlay.show", "session.overlay.toggle")
def test_visibility_refusals(ctl):
    sid = _split(ctl)
    for cmd in VISIBILITY:
        assert_error(ctl.request(cmd, sid), "no overlay")
    _open(ctl, sid, pane="right")
    for cmd in VISIBILITY:
        assert_error(ctl.request(cmd, sid, pane="left"), "no overlay")
    _closed(ctl, sid, pane="right")

    # a HUD takes no input and holds nothing to hide
    hud = _new(ctl)
    assert_ok(ctl.request("session.hud.open", hud, message="hud alone"))
    for cmd in VISIBILITY:
        assert_error(ctl.request(cmd, hud), "no overlay")
    assert_absent(_node(ctl, hud), "hiddenOverlays")
    assert_ok(ctl.request("session.hud.close", hud))


@pytest.mark.covers("session.overlay.open", "session.overlay.hide", "session.hud.open", "field:tree.session.hiddenOverlays")
def test_opening_into_a_hidden_slot_is_refused(ctl):
    sid = _split(ctl)
    _open(ctl, sid)
    _left(ctl, "session.overlay.hide", sid, "hidden")
    path, _ = _page(ctl)
    assert_error(ctl.request("session.overlay.open", sid, command=HOLD), "overlay hidden")
    assert_error(ctl.request("session.overlay.open", sid, html=str(path)), "overlay hidden")
    assert_error(ctl.request("session.hud.open", sid, message="over a hidden program"), "overlay hidden")
    assert_absent(_node(ctl, sid), "hud")
    # closing frees the slot, and a new occupant starts shown
    _closed(ctl, sid)
    _open(ctl, sid)
    assert_absent(_node(ctl, sid), "hiddenOverlays")
    _closed(ctl, sid)

    # a hidden pane overlay holds its own pane's slot only
    _open(ctl, sid, pane="left")
    _left(ctl, "session.overlay.hide", sid, "hidden", pane="left")
    assert_error(ctl.request("session.overlay.open", sid, command=HOLD, pane="left"), "overlay hidden")
    assert assert_ok(ctl.request("session.hud.open", sid, message="over a hidden pane"))["id"] == sid
    assert_ok(ctl.request("session.hud.close", sid))
    _open(ctl, sid)
    assert _node(ctl, sid)["hiddenOverlays"] == ["left"]
    _closed(ctl, sid)
    _closed(ctl, sid, pane="left")
    _open(ctl, sid, pane="left")
    assert_absent(_node(ctl, sid), "hiddenOverlays")
    _closed(ctl, sid, pane="left")


@pytest.mark.covers("session.overlay.hide", "session.overlay.show", "field:tree.session.htmlOverlays")
def test_a_hidden_page_reads_back_hidden(ctl):
    sid = _new(ctl)
    _, title, page_id = _open_page(ctl, sid)
    _loaded(ctl, sid, title)
    _left(ctl, "session.overlay.hide", sid, "hidden")
    (page,) = _node(ctl, sid)["htmlOverlays"]
    assert page["hidden"] is True and page["id"] == page_id
    _left(ctl, "session.overlay.show", sid, "shown")
    (page,) = _node(ctl, sid)["htmlOverlays"]
    assert page["id"] == page_id
    assert_absent(page, "hidden")
    _closed(ctl, sid)


def _named(ctl, name):
    """A session whose own name a terminal title never changes, its creation's `tree.changed` already seen."""
    (window,) = window_ids(ctl.request("window.list"))
    cursor = ctl.events_baseline()
    sid = assert_ok(ctl.request("session.new", name=name))["id"]
    ctl.wait_mounted(sid, "left")
    ctl.wait_event(cursor, "tree.changed", {"window": window})
    return window, sid


@pytest.mark.covers("event:tree.changed", "session.overlay.open", "session.overlay.close")
def test_program_and_page_open_and_close_invalidate_the_tree(ctl):
    window, sid = _named(ctl, "overlay events")
    steps = [
        lambda: _open(ctl, sid),
        lambda: _closed(ctl, sid),
        lambda: _open_page(ctl, sid),
        lambda: _closed(ctl, sid),
        lambda: _open(ctl, sid, pane="left"),
        lambda: _closed(ctl, sid, pane="left"),
    ]
    for step in steps:
        cursor = ctl.events_baseline()
        step()
        ctl.wait_event(cursor, "tree.changed", {"window": window})
