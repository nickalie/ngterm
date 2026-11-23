"""A Markdown page that an origin shows with its link grant (`session.overlay.open --markdown --navigation`), followed on
the viewer that presents its session (#931): a click on a link loads the linked file from the origin, and Back goes to
the page's own file again. The origin is a second instance of the build behind the harness's own sshd
(`remote_origin`), and the page files are on the origin only. A link is followed by a click alone, so the case is a ui
test."""

import uuid

import pytest
from agterm_conformance.match import assert_ok, find_session, session_ids
from agterm_conformance.plugin import Ctl

pytestmark = [
    pytest.mark.requires(
        "posix-command-line",
        reason="the remote origin's sshd is POSIX only, and a Windows viewer runs a remote pane through viewer-run",
    ),
    pytest.mark.requires("live-any-shell", reason="the origin's session must be Live in any login shell"),
]


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


@pytest.mark.since("0.42.0")
@pytest.mark.covers(
    "session.overlay.open", "session.overlay.navigate", "ui:htmlOverlay.page", "field:tree.session.htmlOverlays"
)
def test_a_markdown_page_shown_for_the_origin_follows_a_link_and_goes_back(remote_origin, remote_viewer, ui_for):
    origin = Ctl(remote_origin.instance)
    folder = origin.instance.root / f"remote-links-{uuid.uuid4().hex}"
    (folder / "sub").mkdir(parents=True)
    (folder / "sub" / "guide.md").write_text("# Guide\n", encoding="utf-8")
    readme = folder / "README.md"
    # one long link fills the page, so a click on the page lands on it
    readme.write_text("[" + "guide " * 4000 + "](sub/guide.md)\n", encoding="utf-8")
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

        assert_ok(origin.request("session.overlay.open", sid, markdownFile=str(readme), navigation=True))
        page = _shown(remote_viewer, local, "README.md", canGoBack=False)
        assert page.get("replica") is True and page.get("navigation") is True, page

        ui = ui_for(remote_viewer)
        ui.mouse_click("htmlOverlay.page", offset=(0.5, 0.3))
        _shown(remote_viewer, local, "guide.md", canGoBack=True)
        assert ui.text("htmlOverlay.identity") == "guide.md"

        assert_ok(remote_viewer.request("session.overlay.navigate", local, to="back"))
        _shown(remote_viewer, local, "README.md", canGoBack=False, canGoForward=True)
        assert ui.text("htmlOverlay.identity") == "README.md"
    finally:
        if local is not None:
            remote_viewer.request("session.close", local)
        origin.request("session.close", sid)
