"""Text pages (`session.overlay.open` with `textFile`) and what control does to them: a log page follows its file or
pauses (`session.overlay.follow`), a text page searches (`session.overlay.search`), and a diff page steps between its
changes (`session.overlay.changes`). Each reads back under the page's `htmlOverlays[].text` node."""

import time

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, find_session

UNKNOWN = "missing"
NOT_LOADED = "page not loaded"


def _new(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _file(ctl, name, text):
    path = ctl.instance.root / name
    path.write_text(text, encoding="utf-8")
    return path


def _text(ctl, sid, check=lambda text: True):
    """The session-wide page's `text` node once the page has loaded and `check` holds for it."""

    def ready(tree):
        pages = find_session(tree, sid).get("htmlOverlays", [])
        assert len(pages) == 1 and pages[0]["state"] == "loaded", pages
        text = pages[0].get("text")
        assert text is not None and check(text), text
        return text

    return ready(ctl.wait_tree(ready))


def _once_loaded(ctl, cmd, sid, **args):
    """A step goes to the page's renderer, which answers `page not loaded` (changing nothing) until it is up."""
    return ctl.wait_response(cmd, sid, lambda response: response.ok or response.error != NOT_LOADED, **args)


def _open_text(ctl, sid, path, **args):
    assert_ok(ctl.request("session.overlay.open", sid, textFile=str(path), **args))


# -- log pages: session.overlay.open with tail or followFile

TAIL_WITHOUT_TEXT = "session.overlay.open: --tail and --follow-file require --text"
TAIL_WITH_LINE = "session.overlay.open: --tail and --follow-file cannot be combined with --line"
TAIL_RANGE = "session.overlay.open: --tail must be 1...100000"
LOG_FIELDS = ("tail", "following", "lines", "clipped", "waiting", "unreadable")


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open")
def test_log_open_refusals(ctl):
    root = ctl.instance.root
    log = str(root / "app.log")
    # a log option with no text file is refused ahead of any source's own checks, after a stray line
    for args in [{}, {"command": "cat"}, {"image": "relative", "wait": True}, {"markdownFile": str(root / "x.md")},
                 {"html": str(root / "x.html")}]:
        for option in ({"tail": 10}, {"followFile": True}):
            assert_error(ctl.request("session.overlay.open", UNKNOWN, **args, **option), TAIL_WITHOUT_TEXT)
    assert_error(ctl.request("session.overlay.open", UNKNOWN, line=3, tail=10),
                 "session.overlay.open: --line requires --text")
    # beside a line, then out of range, after the text page's own options and line checks
    for option in ({"tail": 10}, {"followFile": True}):
        assert_error(ctl.request("session.overlay.open", UNKNOWN, textFile=log, line=3, **option), TAIL_WITH_LINE)
    for tail in (0, -1, 100001):
        assert_error(ctl.request("session.overlay.open", UNKNOWN, textFile=log, tail=tail), TAIL_RANGE)
    assert_error(ctl.request("session.overlay.open", UNKNOWN, textFile=log, line=0, tail=10),
                 "session.overlay.open: --line must be 1 or more")
    assert_error(ctl.request("session.overlay.open", UNKNOWN, textFile=str(root / "x.md"), tail=10),
                 "session.overlay.open: --text must be a source, config, text or log file")

    # the app reads the log's last lines before it opens anything: a missing log or a binary end opens no page
    sid = _new(ctl)
    try:
        missing = root / "missing.log"
        assert_error(ctl.request("session.overlay.open", sid, textFile=str(missing), followFile=True),
                     f"session.overlay.open: unreadable text file: {missing}")
        binary = root / "binary.log"
        binary.write_bytes(b"first line\n\xff\xfe\x00\n")
        assert_error(ctl.request("session.overlay.open", sid, textFile=str(binary), tail=5),
                     f"session.overlay.open: not a UTF-8 text file: {binary}")
        assert_absent(find_session(ctl.tree(), sid), "htmlOverlays")
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_log_page_reads_back_its_tail_and_follow_state(ctl):
    path = _file(ctl, "follow.log", "first log line\nsecond log line\n")
    sid = _new(ctl)
    try:
        # a tail alone shows the log once, paused
        _open_text(ctl, sid, path, tail=20)
        text = _text(ctl, sid)
        assert (text["file"], text["tail"], text["following"]) == (str(path), 20, False), text
        for field in LOG_FIELDS:
            assert field in text, (field, text)
        assert_absent(text, "line", "language")
        assert_ok(ctl.request("session.overlay.close", sid))
        # follow-file follows, with the default tail
        _open_text(ctl, sid, path, followFile=True)
        text = _text(ctl, sid)
        assert (text["tail"], text["following"]) == (5000, True), text
        assert_ok(ctl.request("session.overlay.close", sid))
        # a plain text page has none of the log fields
        _open_text(ctl, sid, path)
        text = _text(ctl, sid)
        assert_absent(text, *LOG_FIELDS)
    finally:
        ctl.request("session.close", sid)


def _lines(ctl, sid, lines, **expected):
    """The log page's node once it shows `lines` lines and holds `expected`'s fields."""
    return _text(ctl, sid, lambda text: text["lines"] == lines and all(text[k] == v for k, v in expected.items()))


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_log_page_follows_appends_rotation_and_truncation(ctl):
    path = _file(ctl, "follow.log", "first log line\nsecond log line\n")
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path, tail=50, followFile=True)
        _lines(ctl, sid, 2, following=True, waiting=False)
        # an append is read at the next check
        with path.open("a", encoding="utf-8") as log:
            log.write("third log line\n")
        _lines(ctl, sid, 3)
        # a rotation: the path names a new file, which the page follows from its start
        path.rename(path.with_name("follow.log.1"))
        _lines(ctl, sid, 3, waiting=True)
        path.write_text("fresh\n", encoding="utf-8")
        _lines(ctl, sid, 1, waiting=False)
        # copytruncate: the file shrinks in place and the page shows what follows the truncation
        with path.open("a", encoding="utf-8") as log:
            log.write("a\nb\nc\n")
        _lines(ctl, sid, 4)
        with path.open("r+", encoding="utf-8") as log:
            log.truncate(0)
            log.write("after truncation\n")
        _lines(ctl, sid, 1, clipped=False, unreadable=False)
        # an end that turns binary keeps the text shown and marks it unreadable
        with path.open("ab") as log:
            log.write(b"\x00\xff\n")
        _lines(ctl, sid, 1, unreadable=True)
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.follow", "field:tree.session.htmlOverlays")
def test_a_paused_log_page_reads_nothing_until_resumed(ctl):
    path = _file(ctl, "follow.log", "first log line\nsecond log line\n")
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path, tail=50, followFile=True)
        _lines(ctl, sid, 2, following=True)
        assert assert_ok(ctl.request("session.overlay.follow", sid, mode="off")) == {"id": sid, "text": "paused"}
        _lines(ctl, sid, 2, following=False)
        with path.open("a", encoding="utf-8") as log:
            log.write("third log line\n")
        # a paused page checks nothing: well past two checks, it still shows what it had
        time.sleep(2.5)
        assert _text(ctl, sid)["lines"] == 2
        # a resume reads at once
        assert assert_ok(ctl.request("session.overlay.follow", sid, mode="on")) == {"id": sid, "text": "following"}
        _lines(ctl, sid, 3, following=True)
        # a page opened paused (a tail alone) shows the log once and reads it again only when followed
        assert_ok(ctl.request("session.overlay.close", sid))
        _open_text(ctl, sid, path, tail=50)
        _lines(ctl, sid, 3, following=False)
        with path.open("a", encoding="utf-8") as log:
            log.write("fourth log line\n")
        time.sleep(2.5)
        assert _text(ctl, sid)["lines"] == 3
        assert_ok(ctl.request("session.overlay.follow", sid))
        _lines(ctl, sid, 4, following=True)
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
def test_a_log_reads_only_its_tail(ctl):
    # over the whole-file cap a text page is refused, while a log page reads only its last lines
    path = ctl.instance.root / "big.log"
    line = "x" * 1023 + "\n"
    path.write_text(line * (5 << 10), encoding="utf-8")
    sid = _new(ctl)
    try:
        assert_error(ctl.request("session.overlay.open", sid, textFile=str(path)),
                     f"session.overlay.open: text file too large (over 4 MB): {path}")
        _open_text(ctl, sid, path, tail=10)
        _lines(ctl, sid, 10, tail=10, clipped=False)
        assert_ok(ctl.request("session.overlay.close", sid))
        # one line longer than the read window shows clipped, from its first whole character
        long_line = ctl.instance.root / "long.log"
        long_line.write_text("y" * (3 << 20) + "\n", encoding="utf-8")
        _open_text(ctl, sid, long_line, tail=5)
        _lines(ctl, sid, 1, clipped=True)
    finally:
        ctl.request("session.close", sid)


# -- session.overlay.follow


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.follow", "session.overlay.open", "field:tree.session.htmlOverlays")
def test_follow_answers_the_state_it_left(ctl):
    path = _file(ctl, "follow.log", "first log line\nsecond log line\n")
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path, tail=50, followFile=True)
        text = _text(ctl, sid)
        assert (text["file"], text["tail"], text["following"]) == (str(path), 50, True)
        # an absent mode toggles; on and off are idempotent
        for mode, state in [("off", "paused"), ("off", "paused"), ("toggle", "following"), ("on", "following"),
                            (None, "paused"), ("on", "following")]:
            response = ctl.request("session.overlay.follow", sid, **({} if mode is None else {"mode": mode}))
            assert assert_ok(response) == {"id": sid, "text": state}, mode
            assert _text(ctl, sid)["following"] is (state == "following"), mode
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.follow")
def test_follow_refusals(ctl):
    # the mode is checked before anything is looked up
    assert_error(ctl.request("session.overlay.follow", UNKNOWN, mode="maybe"), "invalid overlay follow mode: maybe")
    sid = _new(ctl)
    try:
        assert_error(ctl.request("session.overlay.follow", sid, mode="on"), "no overlay")
        # a text page opened without --tail or --follow-file is no log page
        _open_text(ctl, sid, _file(ctl, "notes.txt", "plain text\n"))
        _text(ctl, sid)
        assert_error(ctl.request("session.overlay.follow", sid, mode="on"), "no log page")
    finally:
        ctl.request("session.close", sid)


# -- session.overlay.search

SEARCHED = "alpha one\nbeta\nalpha two\ngamma\nalpha three\n"


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.search", "field:tree.session.htmlOverlays")
def test_text_page_search_sets_steps_and_closes(ctl):
    path = _file(ctl, "searched.txt", SEARCHED)
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path)
        assert_absent(_text(ctl, sid), "search")
        assert assert_ok(ctl.request("session.overlay.search", sid, text="alpha")) == {"id": sid}
        # the page counts the matches and picks the current one; a step moves on from it and wraps
        search = _text(ctl, sid, lambda text: text.get("search", {}).get("matches") == 3)["search"]
        assert search["query"] == "alpha" and 1 <= search["current"] <= 3, search
        start = search["current"]
        forward = assert_ok(_once_loaded(ctl, "session.overlay.search", sid, to="next"))
        assert forward == {"id": sid, "text": f"{start % 3 + 1} of 3"}
        back = assert_ok(ctl.request("session.overlay.search", sid, to="prev"))
        assert back == {"id": sid, "text": f"{start} of 3"}
        _text(ctl, sid, lambda text: text["search"] == {"query": "alpha", "matches": 3, "current": start})
        assert assert_ok(ctl.request("session.overlay.search", sid, to="close")) == {"id": sid}
        assert_absent(_text(ctl, sid), "search")
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.search")
def test_overlay_search_refusals(ctl):
    # one of a query and a step, checked before anything is looked up
    for args, error in [
        ({}, "session.overlay.search needs text, --next, --prev or --close"),
        ({"text": "a", "to": "next"}, "session.overlay.search: text cannot be combined with --next, --prev or --close"),
        ({"text": ""}, "session.overlay.search: text must not be empty"),
        ({"to": "sideways"}, "invalid overlay search direction: sideways"),
    ]:
        assert_error(ctl.request("session.overlay.search", UNKNOWN, **args), error)
    sid = _new(ctl)
    try:
        assert_error(ctl.request("session.overlay.search", sid, text="a"), "no overlay")
        markdown = _file(ctl, "searched.md", "# Heading\n")
        assert_ok(ctl.request("session.overlay.open", sid, markdownFile=str(markdown)))
        assert_error(ctl.request("session.overlay.search", sid, text="a"), "no text page")
        assert_ok(ctl.request("session.overlay.close", sid))
        _open_text(ctl, sid, _file(ctl, "unsearched.txt", SEARCHED))
        _text(ctl, sid)
        assert_error(ctl.request("session.overlay.search", sid, to="next"), "no search query: set one first")
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.0")
@pytest.mark.covers("session.overlay.search", "session.overlay.follow", "field:tree.session.htmlOverlays")
def test_a_search_step_pauses_a_following_log_page(ctl):
    path = _file(ctl, "searched.log", SEARCHED)
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path, tail=50, followFile=True)
        _lines(ctl, sid, 5, following=True)
        assert_ok(ctl.request("session.overlay.search", sid, text="alpha"))
        # a following page starts its search at the last match
        _text(ctl, sid, lambda text: text.get("search") == {"query": "alpha", "matches": 3, "current": 3})
        # a step moves off the end the page follows, so it pauses first
        assert assert_ok(_once_loaded(ctl, "session.overlay.search", sid, to="next")) == {"id": sid, "text": "1 of 3"}
        _text(ctl, sid, lambda text: text["following"] is False and text["search"]["current"] == 1)
        assert assert_ok(ctl.request("session.overlay.follow", sid)) == {"id": sid, "text": "following"}
    finally:
        ctl.request("session.close", sid)


# -- session.overlay.changes

# two changes, one per hunk, short enough to fit on screen
DIFF = """\
--- a/notes.txt
+++ b/notes.txt
@@ -1,3 +1,3 @@
 one
-two
+TWO
 three
@@ -10,3 +10,3 @@
 ten
-eleven
+ELEVEN
 twelve
"""


@pytest.mark.since("0.41.1")
@pytest.mark.covers("session.overlay.changes", "field:tree.session.htmlOverlays")
def test_diff_page_steps_between_changes(ctl):
    path = _file(ctl, "change.diff", DIFF)
    sid = _new(ctl)
    try:
        _open_text(ctl, sid, path)
        # the page reports its count before any step
        assert _text(ctl, sid, lambda text: "changes" in text)["changes"] == {"count": 2, "current": 0}
        first = assert_ok(_once_loaded(ctl, "session.overlay.changes", sid, to="next"))
        assert first == {"id": sid, "text": "1 of 2"}
        for to, reached in [("next", 2), ("next", 1), ("prev", 2)]:
            response = ctl.request("session.overlay.changes", sid, to=to)
            assert assert_ok(response) == {"id": sid, "text": f"{reached} of 2"}, to
        _text(ctl, sid, lambda text: text["changes"] == {"count": 2, "current": 2})
    finally:
        ctl.request("session.close", sid)


@pytest.mark.since("0.41.1")
@pytest.mark.covers("session.overlay.changes")
def test_overlay_changes_refusals(ctl):
    # the step is checked before anything is looked up
    assert_error(ctl.request("session.overlay.changes", UNKNOWN), "session.overlay.changes requires to next or prev")
    assert_error(ctl.request("session.overlay.changes", UNKNOWN, to="sideways"),
                 "invalid overlay changes direction: sideways")
    sid = _new(ctl)
    try:
        assert_error(ctl.request("session.overlay.changes", sid, to="next"), "no overlay")
        _open_text(ctl, sid, _file(ctl, "plain.txt", DIFF))
        _text(ctl, sid)
        assert_error(ctl.request("session.overlay.changes", sid, to="next"), "no diff page")
        assert_ok(ctl.request("session.overlay.close", sid))
        # a patch with no hunk has nothing to step to
        _open_text(ctl, sid, _file(ctl, "empty.diff", "nothing changed here\n"))
        _text(ctl, sid)
        assert_error(_once_loaded(ctl, "session.overlay.changes", sid, to="next"), "no changes")
    finally:
        ctl.request("session.close", sid)
