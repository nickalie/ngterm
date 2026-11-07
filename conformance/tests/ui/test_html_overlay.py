"""An HTML page in a session's overlay slot: its web view, the strip that names it, the navigation buttons and the
failure cover. Buttons that leave the app (browser, Finder, copy to the clipboard) are found, never pressed."""

import socket
from http.server import BaseHTTPRequestHandler

import pytest
from agterm_conformance.drivers.base import ElementNotFound
from agterm_conformance.loopback import serve
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.wait import WaitTimeout, wait_until


def _open_settings(ui):
    if ui.name in ("atspi", "uia"):
        # Linux and Windows have no Settings chord (the GTK build's menu, which ngterm's follows): the app menu's item
        # opens it.
        ui.menu("ngterm", "Settings…")
    else:
        ui.press("cmd+,")


def _new(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _state(ctl, sid, state, title=None):
    def reached(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page["state"] == state and (title is None or page.get("title") == title), page
        return True

    ctl.wait_tree(reached)


def _closed(ctl, sid):
    ctl.wait_tree(lambda tree: "htmlOverlays" not in find_session(tree, sid))


class _Page(BaseHTTPRequestHandler):
    def do_GET(self):
        body = b"<!doctype html><title>Served UI Page</title><body><p>served</p></body>"
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


@pytest.mark.covers(
    "ui:htmlOverlay.page",
    "ui:htmlOverlay.identity",
    "ui:htmlOverlay.title",
    "ui:htmlOverlay.back",
    "ui:htmlOverlay.forward",
    "ui:htmlOverlay.reload",
    "ui:htmlOverlay.browser",
    "ui:htmlOverlay.finder",
    "ui:htmlOverlay.close",
)
def test_file_page_strip_names_the_page_and_closes_it(ctl, ui):
    sid = _new(ctl)
    page = ctl.instance.root / "ui-page.html"
    page.write_text("<!doctype html><title>UI Page</title><body><p>page</p></body>")
    assert_ok(ctl.request("session.overlay.open", sid, html=str(page), navigation=True))
    try:
        _state(ctl, sid, "loaded", "UI Page")
        ui.find("htmlOverlay.page")
        assert "ui-page.html" in ui.text("htmlOverlay.identity")
        wait_until(lambda: ui.text("htmlOverlay.title") == "UI Page")
        # A first page has no history either way.
        ui.find("htmlOverlay.back", role="button", enabled=False)
        ui.find("htmlOverlay.forward", role="button", enabled=False)
        ui.find("htmlOverlay.browser", role="button", enabled=True)
        ui.find("htmlOverlay.finder", role="button", enabled=True)
        ui.absent("htmlOverlay.copy")
        ui.click(ui.find("htmlOverlay.reload", role="button", enabled=True))
        _state(ctl, sid, "loaded", "UI Page")
        ui.click(ui.find("htmlOverlay.close", role="button"))
        _closed(ctl, sid)
        ui.absent("htmlOverlay.page")
    finally:
        ctl.request("session.overlay.close", sid)
        ctl.request("session.close", sid)


@pytest.mark.covers("ui:htmlOverlay.hide")
def test_page_strip_hide_hides_its_own_pane(launch, ui_for):
    # The strip's Hide hides its page with the page's scope: a pane page its pane, the session overlay left as it was
    # (upstream's testThePageStripHideButtonHidesItsOwnPane). The strip offers Hide only while the title bar's overlays
    # button is enabled, which the launch seeds.
    ctl = launch(settings={"shownInterfaceElements": ["overlayToggle"]})
    ui = ui_for(ctl)
    sid = _new(ctl)
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")
    page = ctl.instance.root / "ui-strip.html"
    page.write_text("<!doctype html><title>UI Strip</title><body><p>strip page</p></body>")
    try:
        assert_ok(ctl.request("session.overlay.open", sid, command="sh -c 'cat'"))
        assert_ok(ctl.request("session.overlay.open", sid, html=str(page), pane="right"))
        assert assert_ok(ctl.request("session.overlay.hide", sid))["text"] == "hidden"
        assert assert_ok(ctl.request("session.overlay.show", sid, pane="right"))["text"] == "shown"
        _state(ctl, sid, "loaded", "UI Strip")
        ui.click(ui.find("htmlOverlay.hide", role="button", enabled=True))
        ctl.wait_tree(lambda tree: find_session(tree, sid).get("hiddenOverlays") == ["session", "right"])
        ui.absent("htmlOverlay.hide")
    finally:
        ctl.request("session.overlay.close", sid, pane="right")
        ctl.request("session.overlay.close", sid)
        ctl.request("session.close", sid)


@pytest.mark.since("0.39.1")
@pytest.mark.covers("ui:htmlOverlay.hide", "ui:settings-interface-{element}")
def test_the_page_strip_offers_hide_only_while_the_overlays_button_is_on(launch, ui_for):
    # Without the title bar's overlays button nothing on screen brings a hidden page back, so the strip offers Hide only
    # while Settings ▸ Interface enables that button, live on an open page (upstream's
    # testThePageStripOffersHideOnlyWhileTheOverlaysButtonIsOn).
    ctl = launch()
    ui = ui_for(ctl)
    sid = _new(ctl)
    page = ctl.instance.root / "ui-gated.html"
    page.write_text("<!doctype html><title>UI Gated</title><body><p>gated page</p></body>")
    try:
        assert_ok(ctl.request("session.overlay.open", sid, html=str(page)))
        _state(ctl, sid, "loaded", "UI Gated")
        ui.find("htmlOverlay.close", role="button")
        ui.absent("htmlOverlay.hide")
        toggle = ui.ui_ids.expand("settings-interface-{element}", element="overlayToggle")
        try:
            found = ui.tab("Interface", timeout=1)
        except ElementNotFound:
            _open_settings(ui)
            found = ui.tab("Interface")
        ui.click(found)
        ui.click(ui.find(toggle, role="checkbox", value=0))
        ui.find("htmlOverlay.hide", role="button")
        ui.click(ui.find(toggle, role="checkbox", value=1))
        ui.absent("htmlOverlay.hide")
    finally:
        ctl.request("session.overlay.close", sid)
        ctl.request("session.close", sid)


@pytest.mark.covers("ui:htmlOverlay.copy")
def test_url_page_offers_to_copy_its_link(ctl, ui):
    sid = _new(ctl)
    with serve(_Page) as server:
        url = f"http://127.0.0.1:{server.server_port}/page.html"
        assert_ok(ctl.request("session.overlay.open", sid, url=url, navigation=True))
        try:
            _state(ctl, sid, "loaded", "Served UI Page")
            ui.find("htmlOverlay.copy", role="button", enabled=True)
            ui.absent("htmlOverlay.finder")
        finally:
            ctl.request("session.overlay.close", sid)
            ctl.request("session.close", sid)


def _closed_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


@pytest.mark.covers("ui:htmlOverlay.error")
def test_a_page_that_fails_to_load_is_covered(ctl, ui):
    sid = _new(ctl)
    assert_ok(ctl.request("session.overlay.open", sid, url=f"http://127.0.0.1:{_closed_port()}/gone.html"))
    try:
        _state(ctl, sid, "failed")
        ui.find("htmlOverlay.error")
    finally:
        ctl.request("session.overlay.close", sid)
        ctl.request("session.close", sid)


@pytest.mark.since("0.40.0")
@pytest.mark.covers("ui:htmlOverlay.page", "field:tree.session.htmlOverlays")
def test_a_markdown_file_renders_and_the_tree_names_its_source(ctl, ui):
    sid = _new(ctl)
    doc = ctl.instance.root / "notes.md"
    doc.write_text("# Rendered Heading\n\nsome *text*\n", encoding="utf-8")
    try:
        assert_ok(ctl.request("session.overlay.open", sid, markdownFile=str(doc)))
        _state(ctl, sid, "loaded")
        page, = find_session(ctl.tree(), sid)["htmlOverlays"]
        assert page["markdown"] == str(doc)
        assert "file" not in page and "image" not in page and page["javascript"] is False
        ui.find("htmlOverlay.page")
        assert ui.text("htmlOverlay.identity") == "notes.md"
        ui.click(ui.find("htmlOverlay.close", role="button"))
        _closed(ctl, sid)
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("ui:htmlOverlay.page", "field:tree.session.htmlOverlays")
def test_a_text_file_renders_at_its_line_and_the_tree_names_its_source(ctl, ui):
    sid = _new(ctl)
    source = ctl.instance.root / "main.go"
    source.write_text("package main\n\n// Rendered Text Line\n", encoding="utf-8")
    try:
        assert_ok(ctl.request("session.overlay.open", sid, textFile=str(source), line=3))
        _state(ctl, sid, "loaded")
        page, = find_session(ctl.tree(), sid)["htmlOverlays"]
        assert page["text"] == {"file": str(source), "language": "go", "line": 3}, page
        assert "file" not in page and "markdown" not in page and page["javascript"] is False
        ui.find("htmlOverlay.page")
        # a text page is named by its file, never by the generated page's title
        assert ui.text("htmlOverlay.identity") == "main.go"
        ui.click(ui.find("htmlOverlay.close", role="button"))
        _closed(ctl, sid)
    finally:
        ctl.request("session.close", sid)


def _search_query(ctl, sid, query, timeout=2.0):
    """Waits for the session-wide text page's search to read back `query`."""

    def searched(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page.get("text", {}).get("search", {}).get("query") == query, page
        return True

    ctl.wait_tree(searched, timeout)


def _typed_into_the_page(ctl, ui, sid, letter):
    """Types `letter`, taken back until it reaches the page's field: the keyboard moves to the page a frame after Find,
    which a key sent at once can beat."""
    ui.press(letter)
    try:
        _search_query(ctl, sid, letter)
        return True
    except WaitTimeout:
        ui.press("delete")
        return False


@pytest.mark.since("0.41.0")
@pytest.mark.covers("ui:search-field", "ui:htmlOverlay.close", "field:tree.session.htmlOverlays")
def test_find_over_a_text_page_searches_the_page(ctl, ui):
    # What is on top owns Find (upstream's testCommandFOverAFloatingTextPageSearchesThePageAndLeavesTheTerminalSearch):
    # over a floating text page it focuses the page's own field, and the terminal search behind stays as it was.
    sid = _new(ctl)
    notes = ctl.instance.root / "notes.txt"
    notes.write_text("alpha\nbeta\n", encoding="utf-8")
    try:
        ui.menu("View", "Find…")
        ui.find("search-field", role="text-field")
        assert_ok(ctl.request("session.overlay.open", sid, textFile=str(notes), sizePercent=60))
        _state(ctl, sid, "loaded")
        ui.menu("View", "Find…")
        wait_until(lambda: _typed_into_the_page(ctl, ui, sid, "b"), 20)
        ui.find("search-field", role="text-field")
        ui.click(ui.find("htmlOverlay.close", role="button"))
        _closed(ctl, sid)
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("ui:search-field")
def test_find_with_a_page_over_the_other_pane_searches_the_terminal(ctl, ui):
    # A page over the other pane of a split covers nothing the user types into, so Find opens the focused terminal's
    # search (upstream's testCommandFWithAPageOverTheOtherPaneSearchesTheFocusedTerminal).
    sid = _new(ctl)
    assert_ok(ctl.request("session.split", sid, mode="on"))
    ctl.wait_mounted(sid, "right")
    notes = ctl.instance.root / "notes.txt"
    notes.write_text("alpha\n", encoding="utf-8")
    try:
        assert_ok(ctl.request("session.overlay.open", sid, textFile=str(notes), pane="left"))
        assert_ok(ctl.request("session.focus", sid, pane="right"))
        ui.menu("View", "Find…")
        ui.find("search-field", role="text-field")
    finally:
        ctl.request("session.overlay.close", sid, pane="left")
        ctl.request("session.close", sid)
