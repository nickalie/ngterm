"""Local raster/Markdown overlays have source-specific permissions and reload their original document."""

import base64

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session

pytestmark = pytest.mark.since("0.40.0")

PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")


def _new(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _page(ctl, sid, state="loaded"):
    def ready(tree):
        pages = find_session(tree, sid).get("htmlOverlays", [])
        assert len(pages) == 1 and pages[0]["state"] == state, pages
        return pages[0]
    tree = ctl.wait_tree(ready)
    return find_session(tree, sid)["htmlOverlays"][0]


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_image_page_reads_back_its_source(ctl):
    path = ctl.instance.root / "tiny.PNG"
    path.write_bytes(PNG)
    sid = _new(ctl)
    try:
        assert_ok(ctl.request("session.overlay.open", sid, image=str(path)))
        page = _page(ctl, sid)
        assert_subset(page, {"image": str(path), "javascript": False})
        assert_absent(page, "file", "url", "markdown", "cwd")
        assert find_session(ctl.tree(), sid)["overlaySizePercent"] == 95
    finally:
        ctl.request("session.close", sid)


@pytest.mark.covers("session.overlay.open", "session.overlay.reload", "field:tree.session.htmlOverlays")
def test_markdown_page_reads_back_its_source(ctl):
    path = ctl.instance.root / "notes.md"
    path.write_text("# Rendered Heading\n\nsome *text*\n", encoding="utf-8")
    sid = _new(ctl)
    try:
        assert_ok(ctl.request("session.overlay.open", sid, markdownFile=str(path)))
        page = _page(ctl, sid)
        assert_subset(page, {"markdown": str(path), "javascript": False})
        assert_absent(page, "file", "url", "image", "cwd")
        # Reload current is still the authored Markdown file, even though its generated page is about:blank.
        path.write_text("# Rewritten Heading\n", encoding="utf-8")
        assert_ok(ctl.request("session.overlay.reload", sid, current=True))
        assert _page(ctl, sid)["markdown"] == str(path)
        assert_error(ctl.request("session.overlay.navigate", sid, to="browser"),
                     "a markdown page opens only in its overlay")
        assert_error(ctl.request("session.overlay.navigate", sid, to="finder"),
                     "show in Finder requires a file page")
    finally:
        ctl.request("session.close", sid)


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize("source", ["image", "markdownFile"])
def test_document_open_refusals(ctl, source):
    kind = "image" if source == "image" else "markdown"
    path = str(ctl.instance.root / ("x.png" if source == "image" else "x.md"))
    source_args = {source: path}
    assert_error(ctl.request("session.overlay.open", "missing", **source_args, html=path),
                 "session.overlay.open takes one of a command, --html, --url, --image or --markdown")
    for flag in ("wait", "navigation", "javascript", "chromeless", "persistent", "browse"):
        assert_error(ctl.request("session.overlay.open", "missing", **source_args, **{flag: True}),
                     f"session.overlay.open: --{kind} takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse")
    assert_error(ctl.request("session.overlay.open", "missing", **{source: "relative"}),
                 f"session.overlay.open: --{kind} must be an absolute path")
    suffixes = "a png, jpg, jpeg, gif or webp file" if source == "image" else "a .md or .markdown file"
    assert_error(ctl.request("session.overlay.open", "missing", **{source: str(ctl.instance.root / "x.svg")}),
                 f"session.overlay.open: --{kind} must be {suffixes}")


@pytest.mark.covers("session.companion.page", "session.overlay.open")
def test_local_document_pages_are_not_remote_pages(ctl):
    # The public companion seam refuses these source kinds before target discovery; they cannot become remote pages.
    for source, filename in [("image", "x.png"), ("markdownFile", "x.md")]:
        assert_error(ctl.request("session.companion.page", "missing", **{source: str(ctl.instance.root / filename)}),
                     "a page companion needs --html or --url")
