"""Open Recent entries a launch reads from `recent-closed.json`. Each test starts its own instance through the `launch`
fixture, writes the list into its state directory and relaunches it on that list."""

import json
import uuid

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session, session_ids
from agterm_conformance.plugin import REFERENCE_IMPL

COMPANION_WITHOUT_PARENT = "parent session is not open or in Recent Closed"
UNREADABLE_PAGE = "the saved page names no file or http(s) URL this version can open"
UNOPENABLE_URL = "ftp://127.0.0.1/saved"


def _id():
    return str(uuid.uuid4()).upper()


@pytest.mark.since("0.39.1")
@pytest.mark.covers("recent.open", "recent.list")
def test_a_companion_whose_parent_is_nowhere_is_refused_by_name(launch):
    ctl = launch()
    entry = _id()
    snapshot = {"id": _id(), "customName": "logs", "cwd": "/tmp", "parentID": _id()}
    place = {"workspaceID": _id(), "workspaceName": "gone", "workspaceIndex": 0, "sessionIndex": 0}
    item = {"id": entry, "kind": "session", "title": "logs", "closedAt": 0, "session": {**place, "snapshot": snapshot}}
    (ctl.instance.state_dir / "recent-closed.json").write_text(json.dumps({"version": 1, "items": [item]}))
    ctl = launch.relaunch(ctl)
    (listed,) = assert_ok(ctl.request("recent.list"))["recent"]
    assert listed["id"] == entry

    # the companion's parent is neither open nor held by another entry, so nothing can take it in
    assert_error(ctl.request("recent.open", entry), COMPANION_WITHOUT_PARENT)

    assert [item["id"] for item in assert_ok(ctl.request("recent.list"))["recent"]] == [entry]
    assert snapshot["id"] not in session_ids(ctl.tree())


def _saved_page(reference):
    """A page session's saved page that names an ftp URL, in each build's snapshot format: the reference's one `page`
    descriptor, the Rust build's `content` and `pages` list."""
    if reference:
        return {"page": {"url": UNOPENABLE_URL}}
    return {"content": "page", "pages": [{"slot": "session", "source": {"kind": "url", "url": UNOPENABLE_URL}}]}


@pytest.mark.since("0.42.0")
@pytest.mark.covers("recent.open", "field:tree.session.content")
def test_a_page_session_whose_saved_page_this_build_cannot_open_reopens_as_a_page_row(launch, impl_options):
    reference = impl_options.impl == REFERENCE_IMPL
    ctl = launch()
    entry, row = _id(), _id()
    snapshot = {"id": row, "customName": "docs", "cwd": "/tmp", **_saved_page(reference)}
    place = {"workspaceID": _id(), "workspaceName": "gone", "workspaceIndex": 0, "sessionIndex": 0}
    item = {"id": entry, "kind": "session", "title": "docs", "closedAt": 0, "session": {**place, "snapshot": snapshot}}
    (ctl.instance.state_dir / "recent-closed.json").write_text(json.dumps({"version": 1, "items": [item]}))
    ctl = launch.relaunch(ctl)

    assert_ok(ctl.request("recent.open", entry))
    node = find_session(ctl.tree(), row)
    # a page row, never a terminal
    assert_subset(node, {"name": "docs", "content": "page", "overlay": False, "realized": False})
    if reference:
        # upstream shows the reason as its row's failed page
        tree = ctl.wait_tree(lambda tree: find_session(tree, row).get("htmlOverlays"))
        (shown,) = find_session(tree, row)["htmlOverlays"]
        assert_subset(shown, {"state": "failed", "error": UNREADABLE_PAGE})
    else:
        # the Rust build opens no page and its row shows the reason in place of one
        assert_absent(node, "htmlOverlays")
