"""`notify`: the unseen badge on the session (`unseen`) and the `notify` event. A headless Linux instance posts to its
own session bus, where nothing shows it; under CONFORMANCE_LINUX_DISPLAY=session (and on macOS) each post reaches the
desktop, so the module posts as few as it can and selects the notified session afterwards, which withdraws them."""

import pytest
from agterm_conformance.match import assert_absent, assert_error, assert_ok, find_session, session_ids

UNKNOWN = "zzz"


def _node(ctl, sid):
    return find_session(ctl.tree(), sid)


@pytest.fixture
def background(ctl):
    """A session that is not selected, so a notification counts as unseen."""
    sid = assert_ok(ctl.request("session.new", noSelect=True))["id"]
    yield sid
    ctl.request("session.select", sid)


@pytest.mark.covers("notify", "event:notify", "field:tree.session.unseen", "field:result.id")
def test_notify_raises_the_badge_and_emits_the_event(ctl, background):
    tree = ctl.tree()
    node = find_session(tree, background)
    (workspace,) = [ws["id"] for ws in tree["tree"]["workspaces"] if node in ws["sessions"]]
    (window,) = [w["id"] for w in assert_ok(ctl.request("window.list"))["windows"]]
    cursor = ctl.events_baseline()

    # Upstream adds a note in `result.text` when banners are off in settings; only the id is shared.
    assert assert_ok(ctl.request("notify", background, title="Build", body="tests passed"))["id"] == background
    assert _node(ctl, background)["unseen"] == 1
    first = ctl.wait_event(cursor, "notify", {"session": background})
    where = {"session": background, "workspace": workspace, "window": window}
    assert {key: first[key] for key in where} == where
    assert first["payload"] == {"name": node["name"], "title": "Build", "body": "tests passed"}

    # An omitted title falls back to the session's name.
    assert_ok(ctl.request("notify", background, body="second"))
    assert _node(ctl, background)["unseen"] == 2
    second = ctl.wait_event(cursor.past(first), "notify", {"session": background})
    assert second["payload"] == {"name": node["name"], "title": node["name"], "body": "second"}

    assert_ok(ctl.request("session.seen", background))
    assert_absent(_node(ctl, background), "unseen")


@pytest.mark.covers("notify")
@pytest.mark.parametrize(
    ("target", "args", "error"),
    [
        ("background", {}, "notify requires a body"),
        ("background", {"body": "", "title": "t"}, "notify requires a body"),
        (UNKNOWN, {"body": "b"}, f"no such session: {UNKNOWN}"),
    ],
)
def test_notify_refusals_post_nothing(ctl, background, target, args, error):
    cursor = ctl.events_baseline()
    assert_error(ctl.request("notify", background if target == "background" else target, **args), error)
    assert_absent(_node(ctl, background), "unseen")
    response = ctl.request("events.read", run=cursor.run, after=str(cursor.after), kinds=["notify"])
    assert assert_ok(response)["events"]["items"] == []


@pytest.mark.requires("posix-shell", reason="the program printing the escape runs in a known shell")
@pytest.mark.covers("session.type", "event:notify", "field:tree.session.unseen")
@pytest.mark.parametrize(
    ("printed", "title", "body"),
    [
        # OSC 9 carries a body alone: the title falls back to the session's name
        ("'\\033]9;%s\\007' osc-nine", None, "osc-nine"),
        ("'\\033]777;notify;%s;%s\\007' Build osc-777", "Build", "osc-777"),
    ],
    ids=["osc-9", "osc-777"],
)
def test_a_programs_desktop_notification_raises_the_badge(ctl, printed, title, body):
    """A terminal's desktop notification (OSC 9, OSC 777) from a session in the background notifies as `notify` does."""
    (home, *_) = session_ids(ctl.tree())
    sid = assert_ok(ctl.request("session.new"))["id"]
    try:
        ctl.wait_mounted(sid, "left")
        name = _node(ctl, sid)["name"]
        cursor = ctl.events_baseline()
        # printed once the session is in the background, so its pane is not the one in front
        assert_ok(ctl.request("session.type", sid, text=f"(sleep 2; printf {printed}) &\n"))
        assert_ok(ctl.request("session.select", home))
        event = ctl.wait_event(cursor, "notify", {"session": sid})
        assert event["payload"] == {"name": name, "title": title or name, "body": body}
        assert _node(ctl, sid)["unseen"] == 1
    finally:
        ctl.request("session.close", sid)
