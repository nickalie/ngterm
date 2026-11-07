"""Local raster/Markdown/text overlays have source-specific permissions and reload their original document."""

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
        assert_absent(page, "file", "url", "markdown", "document", "cwd")
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
        assert_absent(page, "file", "url", "image", "document", "cwd")
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


ONE_SOURCE = "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text"
KINDS = {"image": ("image", "x.png"), "markdownFile": ("markdown", "x.md"), "textFile": ("text", "x.go")}
TYPES = {
    "image": "a png, jpg, jpeg, gif or webp file",
    "markdownFile": "a .md or .markdown file",
    "textFile": "a source, config, text or log file",
}


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize("source", ["image", "markdownFile", pytest.param("textFile", marks=pytest.mark.since("0.41.0"))])
def test_document_open_refusals(ctl, source):
    kind, filename = KINDS[source]
    path = str(ctl.instance.root / filename)
    source_args = {source: path}
    assert_error(ctl.request("session.overlay.open", "missing", **source_args, html=path), ONE_SOURCE)
    if source != "textFile":
        # a text file beside another local source is a second source too
        text = str(ctl.instance.root / "x.go")
        assert_error(ctl.request("session.overlay.open", "missing", **source_args, textFile=text), ONE_SOURCE)
    for flag in ("wait", "navigation", "javascript", "chromeless", "persistent", "browse"):
        assert_error(ctl.request("session.overlay.open", "missing", **source_args, **{flag: True}),
                     f"session.overlay.open: --{kind} takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse")
    assert_error(ctl.request("session.overlay.open", "missing", **{source: "relative"}),
                 f"session.overlay.open: --{kind} must be an absolute path")
    # a text path is taken literally: a `:line` suffix is part of its name, and no source, config, text or log type
    ineligible = ["x.svg", "x.md", "x.html", "a.go:12"] if source == "textFile" else ["x.svg"]
    for name in ineligible:
        assert_error(ctl.request("session.overlay.open", "missing", **{source: str(ctl.instance.root / name)}),
                     f"session.overlay.open: --{kind} must be {TYPES[source]}")


TEXT_OPTIONS = "session.overlay.open: --text takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse"
LINE_WITHOUT_TEXT = "session.overlay.open: --line requires --text"
LINE_NOT_POSITIVE = "session.overlay.open: --line must be 1 or more"


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open", "session.overlay.reload", "session.overlay.navigate",
                    "field:tree.session.htmlOverlays")
def test_text_page_reads_back_its_source(ctl):
    path = ctl.instance.root / "main.go"
    path.write_text("package main\n\n// the third line\nfunc main() {}\n", encoding="utf-8")
    sid = _new(ctl)
    try:
        assert_ok(ctl.request("session.overlay.open", sid, textFile=str(path), line=3))
        page = _page(ctl, sid)
        assert page["text"] == {"file": str(path), "language": "go", "line": 3}, page
        assert page["javascript"] is False
        assert_absent(page, "file", "url", "image", "markdown", "document", "cwd")
        assert_error(ctl.request("session.overlay.navigate", sid, to="browser"),
                     "a text page opens only in its overlay")
        assert_error(ctl.request("session.overlay.navigate", sid, to="finder"),
                     "show in Finder requires a file page")
        # Reload current is still the text file at its requested line, though its generated page is about:blank.
        path.write_text("package main\n\n// a rewritten third line\n", encoding="utf-8")
        assert_ok(ctl.request("session.overlay.reload", sid, current=True))
        assert _page(ctl, sid)["text"] == {"file": str(path), "language": "go", "line": 3}
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open")
def test_text_open_line_and_file_refusals(ctl):
    root = ctl.instance.root
    go = str(root / "x.go")
    # --line without --text is refused first, ahead of every other source's own checks
    for args in [{}, {"command": "cat"}, {"image": "relative", "wait": True}, {"markdownFile": str(root / "x.md")},
                 {"html": str(root / "x.html"), "url": "http://localhost:1/"}]:
        assert_error(ctl.request("session.overlay.open", "missing", line=4, **args), LINE_WITHOUT_TEXT)
    for line in (0, -1):
        assert_error(ctl.request("session.overlay.open", "missing", textFile=go, line=line), LINE_NOT_POSITIVE)
    # the options and the path come before the line, the line before the file's type
    assert_error(ctl.request("session.overlay.open", "missing", textFile=go, line=0, wait=True), TEXT_OPTIONS)
    assert_error(ctl.request("session.overlay.open", "missing", textFile="x.go", line=0),
                 "session.overlay.open: --text must be an absolute path")
    assert_error(ctl.request("session.overlay.open", "missing", textFile=str(root / "x.svg"), line=0), LINE_NOT_POSITIVE)
    assert_error(ctl.request("session.overlay.open", "missing", textFile=go, cwd=str(root)), TEXT_OPTIONS)
    for source in ("command", "url"):
        value = "cat" if source == "command" else "http://localhost:1/"
        assert_error(ctl.request("session.overlay.open", "missing", textFile=go, **{source: value}), ONE_SOURCE)

    # The app reads the file before it opens anything: a missing, oversized or binary file opens no page.
    missing = root / "missing.go"
    folder = root / "sub.txt"
    folder.mkdir(exist_ok=True)
    big = root / "big.log"
    with big.open("wb") as handle:
        handle.truncate((4 << 20) + 1)
    nul = root / "nul.txt"
    nul.write_bytes(b"a\x00b")
    invalid = root / "bad.txt"
    invalid.write_bytes(b"\xff\xfea")
    sid = _new(ctl)
    try:
        for path, error in [
            (missing, "unreadable text file: {}"),
            (folder, "unreadable text file: {}"),
            (big, "text file too large (over 4 MB): {}"),
            (nul, "not a UTF-8 text file: {}"),
            (invalid, "not a UTF-8 text file: {}"),
        ]:
            assert_error(ctl.request("session.overlay.open", sid, textFile=str(path), line=2),
                         "session.overlay.open: " + error.format(path))
            assert "htmlOverlays" not in find_session(ctl.tree(), sid), path
    finally:
        ctl.request("session.close", sid)


@pytest.mark.covers("session.companion.page", "session.overlay.open")
def test_local_document_pages_are_not_remote_pages(ctl):
    # The public companion seam refuses these source kinds before target discovery; they cannot become remote pages.
    for source, filename in [("image", "x.png"), ("markdownFile", "x.md"), ("textFile", "x.go")]:
        assert_error(ctl.request("session.companion.page", "missing", **{source: str(ctl.instance.root / filename)}),
                     "a page companion needs --html or --url")
