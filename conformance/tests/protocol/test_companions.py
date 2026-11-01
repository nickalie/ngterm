"""Companion sessions: a session created under a parent, nested after it in the parent's workspace, folded with it and
moved with it. The module shares one instance, so every test builds the group it needs in a workspace of its own; the
parent's respawn also runs on an instance of its own under Live sessions."""

import itertools

import pytest
from agterm_conformance.match import (
    assert_absent,
    assert_error,
    assert_ok,
    assert_subset,
    find_session,
    session_ids,
)
from agterm_conformance.plugin import Ctl
from agterm_conformance.wait import wait_until

UNKNOWN = "zzz"
PLACEMENT = "a companion takes no workspace or placement: it lands under its parent"
MOVE_REFUSED = "a companion moves only among its siblings"

LIVE = pytest.mark.requires(
    "live-sessions", reason="needs panes in zmx daemons, which upstream creates only for a zsh or fish login shell"
)
_markers = itertools.count(1)

pytestmark = pytest.mark.since("0.39.1")


def _workspace(ctl, sessions=1):
    wid = assert_ok(ctl.request("workspace.new"))["id"]
    return wid, [assert_ok(ctl.request("session.new", workspace=wid, cwd="/tmp"))["id"] for _ in range(sessions)]


def _companion(ctl, parent, **args):
    return assert_ok(ctl.request("session.companion.new", parent, noSelect=True, **args))["id"]


@pytest.mark.covers("session.companion.new", "field:result.parent", "field:tree.session.parent")
def test_a_companion_lands_under_its_parent_and_reads_back_its_parent(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    created = assert_ok(ctl.request("session.companion.new", parent, name="logs", noSelect=True))
    first = created["id"]
    assert created["parent"] == parent
    # a companion target means its parent, and a new companion goes after the group's last one
    second = _companion(ctl, first)
    tree = ctl.tree()
    assert session_ids(tree, wid) == [parent, first, second, after]
    assert_subset(find_session(tree, first), {"name": "logs", "parent": parent, "active": False})
    assert find_session(tree, second)["parent"] == parent
    # with no --cwd a companion starts in its parent's directory
    assert find_session(tree, second)["cwd"] == find_session(tree, parent)["cwd"]
    assert_absent(find_session(tree, parent), "parent", "collapsed", "content")
    assert_absent(find_session(tree, first), "collapsed", "content")
    # a top-level session inserted inside the group lands after it
    inserted = assert_ok(ctl.request("session.new", after=first, noSelect=True))["id"]
    assert session_ids(ctl.tree(), wid) == [parent, first, second, inserted, after]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.companion.new")
def test_companion_new_refusals(ctl):
    wid, (parent,) = _workspace(ctl)
    for placement in ({"workspace": wid}, {"workspaceName": "w"}, {"createWorkspace": True}, {"after": parent}):
        # the placement refusal comes before the --wait and target checks
        assert_error(ctl.request("session.companion.new", UNKNOWN, wait=True, **placement), PLACEMENT)
    assert_error(ctl.request("session.companion.new", parent, wait=True), "--wait requires --command")
    assert_error(ctl.request("session.companion.new", UNKNOWN), f"no such session: {UNKNOWN}")
    assert session_ids(ctl.tree(), wid) == [parent]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.collapse", "session.expand", "field:tree.session.collapsed")
def test_collapse_and_expand_fold_a_parent(ctl):
    wid, (parent, lone) = _workspace(ctl, sessions=2)
    child = _companion(ctl, parent)
    assert assert_ok(ctl.request("session.collapse", parent))["id"] == parent
    assert find_session(ctl.tree(), parent)["collapsed"] is True
    # an unchanged value still answers ok
    assert assert_ok(ctl.request("session.collapse", parent))["id"] == parent
    assert_absent(find_session(ctl.tree(), child), "collapsed")
    assert assert_ok(ctl.request("session.expand", parent))["id"] == parent
    assert_absent(find_session(ctl.tree(), parent), "collapsed")
    for cmd in ("session.collapse", "session.expand"):
        assert_error(ctl.request(cmd, lone), "session has no companions")
        assert_error(ctl.request(cmd, child), "session has no companions")
    # a fold outlives the last companion, but nothing folded is left to report
    assert_ok(ctl.request("session.collapse", parent))
    assert_ok(ctl.request("session.close", child))
    assert_absent(find_session(ctl.tree(), parent), "collapsed")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.move")
def test_a_companion_moves_only_among_its_siblings(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    first, second = _companion(ctl, parent), _companion(ctl, parent)
    other, () = _workspace(ctl, sessions=0)

    assert assert_ok(ctl.request("session.move", second, to="up"))["id"] == second
    assert session_ids(ctl.tree(), wid) == [parent, second, first, after]
    assert_ok(ctl.request("session.move", second, after=first))
    assert session_ids(ctl.tree(), wid) == [parent, first, second, after]
    # a top-level session steps over a whole group
    assert_ok(ctl.request("session.move", after, to="up"))
    assert session_ids(ctl.tree(), wid) == [after, parent, first, second]

    assert_error(ctl.request("session.move", first, workspace=other), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, workspace=other, targets=[first, second]), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, before=after), MOVE_REFUSED)
    assert_error(ctl.request("session.move", first, before=parent, targets=[first, second]), MOVE_REFUSED)
    # a batch dropped back where it is moves nothing, which is no refusal
    assert assert_ok(ctl.request("session.move", first, after=second, targets=[first, second]))["affected"] == 0
    assert session_ids(ctl.tree(), wid) == [after, parent, first, second]
    # an ordinary no-op stays ok
    assert assert_ok(ctl.request("session.move", workspace=wid, targets=[after, parent]))["affected"] == 0

    # a parent carries its group
    assert assert_ok(ctl.request("session.move", parent, workspace=other))["id"] == parent
    tree = ctl.tree()
    assert session_ids(tree, wid) == [after]
    assert session_ids(tree, other) == [parent, first, second]
    assert find_session(tree, first)["parent"] == parent
    for workspace in (wid, other):
        assert_ok(ctl.request("workspace.delete", workspace))


@pytest.mark.covers("session.companion.new", "event:session.created")
def test_a_companion_created_event_names_its_parent(ctl):
    wid, (parent,) = _workspace(ctl)
    cursor = ctl.events_baseline()
    child = _companion(ctl, parent, name="side")
    ctl.wait_event(cursor, "session.created", {"session": child, "payload": {"name": "side", "parent": parent}})
    top = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
    created = ctl.wait_event(cursor, "session.created", {"session": top})
    assert_absent(created["payload"], "parent")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.close", "field:result.closed", "event:session.closed")
def test_closing_a_parent_closes_its_group(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    first, second, third = _companion(ctl, parent), _companion(ctl, parent), _companion(ctl, parent)
    # closing a companion closes only it, with the bare receipt
    assert assert_ok(ctl.request("session.close", third)) == {"id": third, "closed": [third]}
    assert session_ids(ctl.tree(), wid) == [parent, first, second, after]

    cursor = ctl.events_baseline()
    receipt = assert_ok(ctl.request("session.close", parent))
    assert receipt == {"id": parent, "affected": 3, "closed": [parent, first, second]}
    assert session_ids(ctl.tree(), wid) == [after]
    # each member announces its own close, the companions last first, then the parent
    read = ctl.request("events.read", run=cursor.run, after=str(cursor.after), kinds=["session.closed"])
    group = {parent, first, second}
    closed = [event["session"] for event in assert_ok(read)["events"]["items"] if event.get("session") in group]
    assert closed == [second, first, parent]

    # a batch naming a companion beside its parent counts each session once
    first, second = _companion(ctl, after), _companion(ctl, after)
    other = assert_ok(ctl.request("session.new", workspace=wid, noSelect=True))["id"]
    batch = assert_ok(ctl.request("session.close", targets=[after, first, other]))
    assert (batch["affected"], batch["closed"]) == (4, [after, first, second, other])
    assert session_ids(ctl.tree(), wid) == []
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.fixture
def shells(request, ctl, adapter):
    """The module's instance for `plain`; for `live`, an instance of its own with Live sessions on."""
    if request.param == "plain":
        yield ctl
        return
    instance = adapter.launch(settings={"restoreMode": "live"})
    try:
        live = Ctl(instance)
        assert assert_ok(live.request("restore.mode"))["restore"]["active"] == "live"
        yield live
    finally:
        adapter.stop(instance)


def _left_pane_id(ctl, sid):
    return next(s for s in find_session(ctl.tree(), sid)["surfaces"] if s["kind"] == "left")["paneID"]


def _prints(ctl, sid):
    """Types a marker line into `sid` until its shell prints it: between an exit and the fresh shell the pane has no
    terminal to type into, and a line typed into the exiting one is lost with it."""
    marker = f"CMARK{next(_markers)}"

    def printed():
        typed = ctl.request("session.type", sid, text=f"printf '%s%s\\n' {marker[:5]} {marker[5:]}\n")
        if not typed.ok:
            return False
        try:
            return ctl.wait_text(sid, rf"(?m)^{marker}$", timeout=1)
        except AssertionError:
            return False

    wait_until(printed)


@pytest.mark.parametrize("shells", ["plain", pytest.param("live", marks=LIVE)], indirect=True)
@pytest.mark.covers("session.companion.new")
def test_a_parent_whose_shell_exits_restarts_it(shells):
    """A parent whose last pane exits while it has companions restarts a plain login shell in place, keeping its
    pane; a command parent restarts as a shell, not its command (#840)."""
    ctl = shells
    wid, (parent,) = _workspace(ctl)
    _companion(ctl, parent)
    ctl.wait_mounted(parent, "left")
    pane = _left_pane_id(ctl, parent)
    _prints(ctl, parent)
    assert_ok(ctl.request("session.type", parent, text="exit\n"))
    _prints(ctl, parent)
    assert parent in session_ids(ctl.tree(), wid)
    assert _left_pane_id(ctl, parent) == pane

    command = assert_ok(ctl.request("session.new", workspace=wid, cwd="/tmp", command="sleep 600", noSelect=True))["id"]
    _companion(ctl, command)
    ctl.wait_tree(lambda tree: (find_session(tree, command).get("foreground") or [])[:1] == ["sleep"])
    assert_ok(ctl.request("session.type", command, text="\x03"))
    _prints(ctl, command)
    node = find_session(ctl.tree(), command)
    assert (node.get("foreground") or [])[:1] != ["sleep"], node
    assert_ok(ctl.request("workspace.delete", wid))


# -- page companions

_pages = itertools.count(1)
PAGE_FULL_SIZE = "a page companion is full size: it takes no --pane, --size-percent, --width or --height"


def _page_file(ctl):
    """A static page in a folder of its own under the instance's root: `(path, title)`."""
    number = next(_pages)
    folder = ctl.instance.root / f"companion-pages-{number}"
    folder.mkdir()
    path = folder / f"page-{number}.html"
    path.write_text(f"<!doctype html><title>Companion {number}</title><body><p>companion page</p></body>")
    return path, f"Companion {number}"


def _loaded(ctl, row, title=None):
    """The tree once row `row`'s page loaded (titled `title` when given)."""

    def loaded(tree):
        pages = find_session(tree, row).get("htmlOverlays") or []
        return any(page["state"] == "loaded" and title in (None, page.get("title")) for page in pages)

    return ctl.wait_tree(loaded)


def _page_companion(ctl, parent, **args):
    """A page companion of `parent` showing a fresh page, loaded: `(row id, page id, page path)`."""
    path, title = _page_file(ctl)
    result = assert_ok(ctl.request("session.companion.page", parent, html=str(path), **args))
    _loaded(ctl, result["id"], title)
    return result["id"], result["pageID"], path


@pytest.mark.covers(
    "session.companion.page",
    "field:result.pageID",
    "field:result.parent",
    "field:tree.session.content",
    "field:tree.session.htmlOverlays",
)
def test_page_companion_opens_a_page_row_with_no_shell_under_its_parent(ctl):
    wid, (parent, after) = _workspace(ctl, sessions=2)
    path, _ = _page_file(ctl)
    created = assert_ok(ctl.request("session.companion.page", parent, html=str(path), color="#102030"))
    page = created["id"]
    assert created["parent"] == parent and created["pageID"]

    tree = _loaded(ctl, page)
    assert session_ids(tree, wid) == [parent, page, after]
    node = find_session(tree, page)
    # named for its page, where its parent stands, with no terminal ever created under it
    assert_subset(node, {"content": "page", "parent": parent, "name": path.name, "realized": False, "overlay": True})
    assert node["cwd"] == find_session(tree, parent)["cwd"]
    (shown,) = node["htmlOverlays"]
    assert_subset(shown, {"id": created["pageID"], "file": str(path)})
    assert_absent(shown, "pane")
    assert_absent(node, "overlaySizePercent")
    # the parent's own slot stays free, and the page opened in the background
    assert find_session(tree, parent)["overlay"] is False
    assert node["active"] is False
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.companion.page")
def test_page_companion_refusals(ctl):
    wid, (parent,) = _workspace(ctl)
    html = str(_page_file(ctl)[0])
    for args in ({"pane": "right"}, {"sizePercent": 50}, {"width": 300}, {"height": 300}):
        assert_error(ctl.request("session.companion.page", parent, html=html, **args), PAGE_FULL_SIZE)
    assert_error(ctl.request("session.companion.page", parent, command="top"), "a page companion needs --html or --url")
    assert_error(ctl.request("session.companion.page", parent), "session.overlay.open requires a command")
    assert_error(ctl.request("session.companion.page", parent, html=html, color="red"), "invalid color: red (#rrggbb)")
    assert_error(
        ctl.request("session.companion.page", parent, html=html, wait=True),
        "session.overlay.open: --wait cannot be combined with --html",
    )
    assert_error(ctl.request("session.companion.page", UNKNOWN, html=html), f"no such session: {UNKNOWN}")
    assert session_ids(ctl.tree(), wid) == [parent]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.companion.page", "session.type", "session.split", "session.rename")
def test_a_page_companion_refuses_terminal_commands(ctl):
    wid, (parent,) = _workspace(ctl)
    page, _, _ = _page_companion(ctl, parent)
    for cmd, args in (
        ("session.type", {"text": "ls\n"}),
        ("session.split", {}),
        ("session.scratch", {}),
        ("session.hud.open", {"text": "hi"}),
        ("session.overlay.open", {"command": "top"}),
        ("session.overlay.resize", {"sizePercent": 50}),
    ):
        assert_error(ctl.request(cmd, page, **args), f"{cmd}: not available on a page companion")
    # the parent takes them as before, and the page row is renamed like any row
    assert_ok(ctl.request("session.type", parent, text="true\n"))
    assert_ok(ctl.request("session.rename", page, name="notes"))
    assert find_session(ctl.tree(), page)["name"] == "notes"
    # a page is never hidden: showing it is already done, hiding it has nothing to hide
    assert_ok(ctl.request("session.overlay.show", page))
    assert_error(ctl.request("session.overlay.hide", page), "no overlay")
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers("session.companion.page", "session.overlay.close", "field:result.pageOutcome")
def test_closing_the_page_closes_its_row(ctl):
    wid, (parent,) = _workspace(ctl)
    page, page_id, _ = _page_companion(ctl, parent)
    assert assert_ok(ctl.request("session.overlay.close", page))["id"] == page
    ctl.wait_tree(lambda tree: page not in session_ids(tree, wid))
    assert session_ids(ctl.tree(), wid) == [parent]
    outcome = assert_ok(ctl.request("session.overlay.result", page=page_id))["pageOutcome"]
    assert outcome == {"pageID": page_id, "outcome": "dismissed"}
    # closing the parent takes a page companion along, and Open Recent keeps no page
    page, _, _ = _page_companion(ctl, parent)
    assert assert_ok(ctl.request("session.close", parent))["closed"] == [parent, page]
    entry = assert_ok(ctl.request("recent.list"))["recent"][0]
    assert entry["sessionIDs"] == [parent]
    assert_ok(ctl.request("workspace.delete", wid))


@pytest.mark.covers(
    "session.companion.page.attach",
    "session.overlay.page.serve",
    "field:result.companionPage",
    streams=("session.overlay.page.serve",),
)
def test_page_attach_hands_out_a_claim_only_for_a_page_companion(ctl):
    wid, (parent,) = _workspace(ctl)
    assert_error(ctl.request("session.companion.page.attach", parent), "not a page companion")
    page, page_id, path = _page_companion(ctl, parent)
    attached = assert_ok(ctl.request("session.companion.page.attach", page))
    assert (attached["id"], attached["parent"]) == (page, parent)
    handed = attached["companionPage"]
    assert_subset(handed, {"page": page_id, "identity": path.name, "source": {"file": {"path": str(path)}}})
    assert handed["token"]
    # the token is claimed once, under a claim of its own; the attachment serves the page's file
    with ctl.stream("session.overlay.page.serve", handed["token"]) as helper:
        assert helper.reply.ok, helper.reply.raw
        assert helper.reply.result["id"] not in (page, page_id)
        with ctl.stream("session.overlay.page.serve", handed["token"]) as again:
            assert_error(again.reply, "page not claimable")
        helper.send({"kind": "resource", "id": 1, "path": str(path)})
        assert helper.read() == {"kind": "head", "id": 1, "mimeType": "text/html"}
    # a viewer leaving ends nothing here
    assert page in session_ids(ctl.tree(), wid)
    assert_ok(ctl.request("workspace.delete", wid))
