"""A Markdown page that an origin shows with its link grant (`session.overlay.open --markdown --navigation`), followed on
the viewer that presents its session (#931): a click on a link loads the linked file from the origin, with its pictures,
and Back, Forward and Reload work as on a local page. A link the origin cannot serve keeps the shown file, and a page
the origin sends with no grant keeps its links as text. The origin is a second instance of the build behind the
harness's own sshd (`remote_origin`), and the page files are on the origin only. A link is followed by a click alone,
so the cases are ui tests."""

import uuid
from contextlib import contextmanager
from io import BytesIO

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import WaitTimeout, wait_until

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]

RED = (255, 0, 0)
BLUE = (0, 0, 255)
# A picture's share of the screen that no window chrome or theme color reaches alone.
PICTURE_PIXELS = 20_000
# How long a click that the origin refuses has to change the page before the case holds that it did not.
REFUSAL_WINDOW = 3.0


def _follows(tree, sid):
    (left,) = [surface for surface in find_session(tree, sid)["surfaces"] if surface["kind"] == "left"]
    assert left.get("lead") == "follower", left
    return True


def _shown(viewer, local, name, **expected):
    """The one page of `local` once it loaded the file `name` and reads back `expected`."""

    def shown(tree):
        pages = find_session(tree, local).get("htmlOverlays") or []
        assert len(pages) == 1, pages
        (page,) = pages
        assert page["state"] == "loaded" and str(page.get("page", "")).endswith(name), page
        assert all(page.get(key) == value for key, value in expected.items()), page
        return True

    shown.__qualname__ = f"{name} loaded in {local}"
    (page,) = find_session(viewer.wait_tree(shown), local)["htmlOverlays"]
    return page


def _folder(origin):
    folder = origin.instance.root / f"remote-links-{uuid.uuid4().hex}"
    (folder / "sub").mkdir(parents=True)
    return folder


def _link_page(path, target):
    # one long link fills the page, so a click on the page lands on it
    path.write_text("[" + "guide " * 4000 + f"]({target})\n", encoding="utf-8")


def _picture(path, color):
    from PIL import Image

    path.parent.mkdir(parents=True, exist_ok=True)
    Image.new("RGB", (640, 480), color).save(path, format="PNG")


def _pixels(ui, color):
    """How many pixels of the viewer's screen have `color`, near enough for the renderer's rounding."""
    from PIL import Image

    image = Image.open(BytesIO(ui.screenshot())).convert("RGB")
    return sum(
        count
        for count, pixel in image.getcolors(image.width * image.height)
        if all(abs(channel - wanted) <= 24 for channel, wanted in zip(pixel, color))
    )


def _shows_picture(ui, color):
    def shown():
        count = _pixels(ui, color)
        assert count >= PICTURE_PIXELS, f"{count} pixels of {color}"
        return count

    return wait_until(shown)


@contextmanager
def _presented(remote_origin, remote_viewer, page, **options):
    """The viewer's session for an origin session that shows the Markdown file `page` with `options`."""
    origin = Ctl(remote_origin.instance)
    before = set(session_ids(origin.tree()))
    sid = assert_ok(origin.request("session.new"))["id"]
    origin.wait_mounted(sid, "left")
    local = None
    try:
        # the origin hands a page over only while its pane follows the viewer's lead, and its own window leads the
        # session it shows: it shows another one
        assert_ok(origin.request("session.select", sorted(before)[0]))
        local = assert_ok(remote_viewer.request("live.attach", sid, host=remote_origin.host))["id"]
        remote_viewer.wait_tree(lambda tree: find_session(tree, local).get("realized"))
        origin.wait_tree(lambda tree: _follows(tree, sid))
        assert_ok(origin.request("session.overlay.open", sid, markdownFile=str(page), **options))
        yield local
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        origin.request("session.close", sid)


def _stays(viewer, local, name, **expected):
    """`local` still shows `name` with `expected` once a click had its time to load another page."""

    def moved(tree):
        (page,) = find_session(tree, local)["htmlOverlays"]
        assert not str(page.get("page", "")).endswith(name) or page["state"] != "loaded", page
        return True

    with pytest.raises(WaitTimeout):
        viewer.wait_tree(moved, timeout=REFUSAL_WINDOW)
    page = _shown(viewer, local, name, **expected)
    assert page.get("error") is None, page
    return page


@pytest.mark.since("0.42.0")
@pytest.mark.covers(
    "session.overlay.open", "session.overlay.navigate", "ui:htmlOverlay.page", "field:tree.session.htmlOverlays"
)
def test_a_markdown_page_shown_for_the_origin_follows_a_link_and_goes_back(remote_origin, remote_viewer, ui_for):
    folder = _folder(Ctl(remote_origin.instance))
    (folder / "sub" / "guide.md").write_text("# Guide\n", encoding="utf-8")
    readme = folder / "README.md"
    _link_page(readme, "sub/guide.md")
    with _presented(remote_origin, remote_viewer, readme, navigation=True) as local:
        page = _shown(remote_viewer, local, "README.md", canGoBack=False)
        assert page.get("replica") is True and page.get("navigation") is True, page

        ui = ui_for(remote_viewer)
        ui.mouse_click("htmlOverlay.page", offset=(0.5, 0.3))
        _shown(remote_viewer, local, "guide.md", canGoBack=True)
        assert ui.text("htmlOverlay.identity") == "guide.md"

        assert_ok(remote_viewer.request("session.overlay.navigate", local, to="back"))
        _shown(remote_viewer, local, "README.md", canGoBack=False, canGoForward=True)
        assert ui.text("htmlOverlay.identity") == "README.md"


@pytest.mark.since("0.42.0")
@pytest.mark.covers(
    "session.overlay.open", "session.overlay.navigate", "session.overlay.reload", "ui:htmlOverlay.page",
    "field:tree.session.htmlOverlays",
)
def test_a_followed_link_shows_its_pictures_from_the_origin_and_goes_forward_and_reloads(
    remote_origin, remote_viewer, ui_for
):
    folder = _folder(Ctl(remote_origin.instance))
    # a picture elsewhere in the page's folder, which the viewer has no copy of
    _picture(folder / "pictures" / "red.png", RED)
    _picture(folder / "pictures" / "blue.png", BLUE)
    guide = folder / "sub" / "guide.md"
    guide.write_text("![red](../pictures/red.png)\n", encoding="utf-8")
    readme = folder / "README.md"
    _link_page(readme, "sub/guide.md")
    with _presented(remote_origin, remote_viewer, readme, navigation=True) as local:
        _shown(remote_viewer, local, "README.md", canGoBack=False)
        ui = ui_for(remote_viewer)
        ui.mouse_click("htmlOverlay.page", offset=(0.5, 0.3))
        _shown(remote_viewer, local, "guide.md", canGoBack=True, canGoForward=False)
        _shows_picture(ui, RED)

        assert_ok(remote_viewer.request("session.overlay.navigate", local, to="back"))
        _shown(remote_viewer, local, "README.md", canGoBack=False, canGoForward=True)
        assert_ok(remote_viewer.request("session.overlay.navigate", local, to="forward"))
        _shown(remote_viewer, local, "guide.md", canGoBack=True, canGoForward=False)
        assert ui.text("htmlOverlay.identity") == "guide.md"
        _shows_picture(ui, RED)

        # a reload reads the file from the origin again
        guide.write_text("![blue](../pictures/blue.png)\n", encoding="utf-8")
        assert_ok(remote_viewer.request("session.overlay.reload", local))
        _shown(remote_viewer, local, "guide.md", canGoBack=True)
        _shows_picture(ui, BLUE)
        assert _pixels(ui, RED) < PICTURE_PIXELS


@pytest.mark.since("0.42.0")
@pytest.mark.covers("session.overlay.open", "ui:htmlOverlay.page", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize("target", ["sub/missing.md", "../outside.md"], ids=["missing", "outside-the-root"])
def test_a_link_the_origin_cannot_serve_keeps_the_shown_file(remote_origin, remote_viewer, ui_for, target):
    folder = _folder(Ctl(remote_origin.instance))
    (folder.parent / "outside.md").write_text("# Outside\n", encoding="utf-8")
    readme = folder / "README.md"
    _link_page(readme, target)
    with _presented(remote_origin, remote_viewer, readme, navigation=True) as local:
        _shown(remote_viewer, local, "README.md", canGoBack=False)
        ui = ui_for(remote_viewer)
        ui.mouse_click("htmlOverlay.page", offset=(0.5, 0.3))
        _stays(remote_viewer, local, "README.md", canGoBack=False)
        assert ui.text("htmlOverlay.identity") == "README.md"


@pytest.mark.since("0.42.0")
@pytest.mark.covers("session.overlay.open", "ui:htmlOverlay.page", "field:tree.session.htmlOverlays")
def test_a_markdown_page_the_origin_shows_with_no_grant_keeps_its_links_as_text(remote_origin, remote_viewer, ui_for):
    folder = _folder(Ctl(remote_origin.instance))
    (folder / "sub" / "guide.md").write_text("# Guide\n", encoding="utf-8")
    readme = folder / "README.md"
    _link_page(readme, "sub/guide.md")
    with _presented(remote_origin, remote_viewer, readme) as local:
        page = _shown(remote_viewer, local, "README.md", canGoBack=False)
        assert page.get("replica") is True and not page.get("navigation"), page
        ui = ui_for(remote_viewer)
        ui.mouse_click("htmlOverlay.page", offset=(0.5, 0.3))
        _stays(remote_viewer, local, "README.md", canGoBack=False)
