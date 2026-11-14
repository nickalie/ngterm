"""The Linux build's page worker (`ngterm-web`, the process its HTML pages' engine runs in) when it breaks: a worker that
ends or stops reading during its start fails the start, one that breaks its protocol later fails every page for the
rest of the run, a frame it sends for a page the app closed is dropped, and a worker whose app quit ends. A
Rust-only extension (capability `web-worker-faults`, conformance/docs/rust-build.md > Rust-only extensions): each test
launches an instance of its own whose worker plays one fault (`launch(web_worker_fault=...)`), since the engine starts
once per process and a failed one stays failed."""

import os
import signal
import time
from pathlib import Path

import pytest
from agterm_conformance.match import assert_error, assert_ok, find_session
from agterm_conformance.plugin import Ctl

pytestmark = pytest.mark.requires(
    "web-worker-faults", reason="a Rust-only extension: upstream's pages run in the app, with no worker to break"
)

NOT_STARTED = "the web engine did not start: the WPE worker failed: "
ENDED = "the web engine ended unexpectedly; restart ngterm to show pages again"
REFUSED = "session.overlay.open: html overlays are not available here: "
WORKER_PROGRAMS = ("ngterm-web", "bwrap")
WAIT = 20.0


def _page(ctl, sid, name, body):
    path = ctl.instance.root / name
    path.write_text(f"<!doctype html><title>{body}</title><p>{body}</p>", encoding="utf-8")
    return assert_ok(ctl.request("session.overlay.open", sid, html=str(path)))["pageID"]


def _new(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    ctl.wait_mounted(sid, "left")
    return sid


def _page_state(ctl, sid, page, state):
    def reached(tree):
        (found,) = [entry for entry in find_session(tree, sid).get("htmlOverlays", []) if entry["id"] == page]
        assert found["state"] == state, found
        return found

    reached.__qualname__ = f"page {page} {state}"
    return reached(ctl.wait_tree(reached))


def _refused(ctl, reason):
    path = ctl.instance.root / "later.html"
    path.write_text("<!doctype html><title>later</title>", encoding="utf-8")
    assert_error(ctl.request("session.overlay.open", _new(ctl), html=str(path)), REFUSED + reason)


def _parent(pid):
    """`pid`'s parent, or None once it is gone."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return None
    # the program's name is in parentheses and may hold spaces: the fields after it are space-separated
    return int(stat.rsplit(")", 1)[1].split()[1])


def _ended(pid):
    """Whether `pid` is gone, or a zombie nobody reaped yet."""
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
    except OSError:
        return True
    return stat.rsplit(")", 1)[1].split()[0] == "Z"


def _workers(instance):
    """The app's page worker processes still running: its children running `ngterm-web`, or the `bwrap` a worker
    re-executes itself under."""
    found = []
    for entry in os.scandir("/proc"):
        if not entry.name.isdigit() or _parent(int(entry.name)) != instance.process.pid:
            continue
        try:
            program = os.path.basename(os.readlink(f"/proc/{entry.name}/exe"))
        except OSError:
            continue
        if program in WORKER_PROGRAMS and not _ended(int(entry.name)):
            found.append(int(entry.name))
    return found


def _worker(instance):
    """The app's page worker, once it started one."""
    deadline = time.monotonic() + WAIT
    while not (found := _workers(instance)):
        assert time.monotonic() < deadline, f"the app started no page worker within {WAIT}s"
        time.sleep(0.05)
    (worker,) = found
    return worker


def _wait_ended(pid, what):
    deadline = time.monotonic() + WAIT
    while not _ended(pid):
        assert time.monotonic() < deadline, f"{what} within {WAIT}s"
        time.sleep(0.05)


@pytest.fixture
def faulty(adapter, request):
    """An instance of its own whose page worker plays the fault the test is parametrized with."""
    instance = adapter.launch(web_worker_fault=request.param)
    try:
        yield Ctl(instance)
    finally:
        adapter.stop(instance)


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize(
    ("faulty", "reason"),
    [
        # the worker's socket ends before its hello
        ("end-before-hello", "the worker ended while starting"),
        # the worker reads nothing past its hello, so the configuration the app sends it cannot go
        ("deaf-after-hello", "Broken pipe (os error 32)"),
    ],
    indirect=["faulty"],
    ids=["end-before-hello", "deaf-after-hello"],
)
def test_a_page_worker_that_breaks_its_start_fails_the_pages(faulty, reason):
    first_session = _new(faulty)
    first = _page(faulty, first_session, "first.html", "first")
    assert _page_state(faulty, first_session, first, "failed")["error"] == NOT_STARTED + reason
    # the engine starts once per process: a later page is refused for the same reason
    _refused(faulty, NOT_STARTED + reason)


@pytest.mark.covers("session.overlay.open", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize("faulty", ["hello-on-evaluate"], indirect=True)
def test_a_page_worker_that_breaks_its_protocol_fails_every_page(faulty):
    # a text page's first evaluation (its text) gets a second hello back: the app drops the worker, the evaluation
    # still waiting, and its socket's end ends the worker
    sid = _new(faulty)
    path = faulty.instance.root / "evaluated.txt"
    path.write_text("a text page\n", encoding="utf-8")
    assert_ok(faulty.request("session.overlay.open", sid, textFile=str(path)))
    (text,) = find_session(faulty.wait_tree(_failed_page(sid)), sid)["htmlOverlays"]
    assert text["error"] == ENDED, text
    deadline = time.monotonic() + WAIT
    while _workers(faulty.instance):
        assert time.monotonic() < deadline, f"the page worker did not end within {WAIT}s of its app's hang-up"
        time.sleep(0.05)
    # the engine stays ended: a later page is refused
    _refused(faulty, ENDED)


def _failed_page(sid):
    def failed(tree):
        (page,) = find_session(tree, sid)["htmlOverlays"]
        assert page["state"] == "failed", page
        return True

    failed.__qualname__ = "the text page failed"
    return failed


@pytest.mark.covers("session.overlay.open", "session.overlay.close", "field:tree.session.htmlOverlays")
@pytest.mark.parametrize("faulty", ["frame-after-close"], indirect=True)
def test_a_frame_of_a_page_the_app_closed_is_dropped(faulty):
    # the worker answers the close with a frame of the closed page, which the app no longer knows
    sid = _new(faulty)
    closed = _page(faulty, sid, "closed.html", "closed")
    _page_state(faulty, sid, closed, "loaded")
    assert_ok(faulty.request("session.overlay.close", sid))
    faulty.wait_tree(lambda tree: "htmlOverlays" not in find_session(tree, sid))
    shown = _page(faulty, sid, "shown.html", "shown")
    assert _page_state(faulty, sid, shown, "loaded")["id"] == shown


@pytest.mark.covers("session.overlay.open")
@pytest.mark.parametrize("faulty", ["hello-after-hang-up"], indirect=True)
def test_a_page_worker_whose_app_quits_during_its_start_ends(faulty):
    # the worker says its hello only once the app hung up: the app quits while its start waits for it
    _page(faulty, _new(faulty), "starting.html", "starting")
    worker = _worker(faulty.instance)
    faulty.instance.process.send_signal(signal.SIGTERM)
    faulty.instance.process.wait(timeout=WAIT)
    _wait_ended(worker, "the page worker did not end after its app quit")
