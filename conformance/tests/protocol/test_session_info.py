"""Ephemeral session info: source read-back, replacement, refusal and clearing."""

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, assert_subset, find_session

pytestmark = pytest.mark.since("0.40.0")


@pytest.mark.covers("session.info", "field:tree.session.info", "field:tree.session.infoMarkdown", "field:tree.session.infoHideAfter")
def test_info_set_reject_and_clear(ctl):
    sid = assert_ok(ctl.request("session.new"))["id"]
    source = '**3** ^[ok](color: "green")'
    try:
        assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")
        assert_ok(ctl.request("session.info", sid, mode="set", text=source, markdown=True, hideAfter=600))
        assert_subset(find_session(ctl.tree(), sid), {"info": source, "infoMarkdown": True, "infoHideAfter": 600})
        assert_error(ctl.request("session.info", sid, mode="set", text='^[x](color: "pink")', markdown=True),
                     "session.info: unknown info color: pink (red|orange|yellow|green|blue|purple|gray)")
        assert find_session(ctl.tree(), sid)["info"] == source
        assert_ok(ctl.request("session.info", sid, mode="set", text="literal **stars**"))
        node = find_session(ctl.tree(), sid)
        assert node["info"] == "literal **stars**"
        assert not node.get("infoMarkdown", False) and node.get("infoHideAfter", 0) == 0
        assert_ok(ctl.request("session.info", sid, mode="clear"))
        assert_absent(find_session(ctl.tree(), sid), "info", "infoMarkdown", "infoHideAfter")
    finally:
        assert_ok(ctl.request("session.close", sid))
