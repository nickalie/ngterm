"""Open Recent entries a launch reads from `recent-closed.json`. Each test starts its own instance through the `launch`
fixture, writes the list into its state directory and relaunches it on that list."""

import json
import uuid

import pytest
from agterm_conformance.match import assert_error, assert_ok, session_ids

COMPANION_WITHOUT_PARENT = "parent session is not open or in Recent Closed"


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
