"""The automatic release check waits out GitHub's rate limit and a server error with no titlebar button and no
alert, and checks again once the limit resets; Help > Check for Updates still tells the user when to retry; an error
that no later check clears keeps the "Update needs attention" button (`ngterm-gpui`'s `updater`).

Only an instrumented build points its check at a stand-in for GitHub's API (`launch(releases_api=...)`, which
`ngterm-update` reads under `cfg(coverage)` alone), so these tests run on the runners' `--coverage` runs only. The
six-hour interval between automatic checks is the updater's unit tests' (GPUI's test clock); here a limit that resets
in seconds shows the wait."""

from __future__ import annotations

import re
import sys
import time

import pytest
from agterm_conformance import llvm_profiles
from agterm_conformance.loopback import serve
from agterm_conformance.releases_api import (
    API_PATH,
    FAKE_TOKEN,
    NEWER_RELEASE,
    StandIn,
    rate_limited,
    release,
    server_error,
    too_many_requests,
)
from agterm_conformance.wait import wait_until

pytestmark = [
    pytest.mark.skipif(
        llvm_profiles.profile_dir() is None,
        reason="only an instrumented build reads a stand-in releases API (the runners' --coverage)",
    ),
    pytest.mark.skipif(sys.platform == "darwin", reason="no update format fits a private install on macOS"),
]

CHECK_TIMEOUT = 30.0
# The updater's RETRY_MARGIN: how long after the limit resets the automatic check tries again.
RETRY_MARGIN = 30.0
# The reset in seconds that a limit names here; `Date` and the reset have whole seconds, so the wait is off by one.
RESET_IN = 2
# The second button of the alert that a failed check opens: no other alert of the app has it.
RELEASE_DOWNLOADS = "button name='Release Downloads'"
DEFERRED = "automatic update check deferred"


def _url(server) -> str:
    return f"http://127.0.0.1:{server.server_port}{API_PATH}"


def _log(ctl) -> str:
    return ctl.instance.log.read_text(encoding="utf-8", errors="replace")


def _assert_quiet(ui) -> None:
    assert not ui.all("update-status"), ui.tree_dump()
    tree = ui.tree_dump()
    assert RELEASE_DOWNLOADS not in tree, tree


def _stay_quiet(ui, until) -> None:
    """Checks the titlebar and the alerts again and again until `until()` is true."""
    while True:
        _assert_quiet(ui)
        if until():
            return
        time.sleep(0.5)


@pytest.mark.covers("ui:update-status")
@pytest.mark.parametrize(
    "limit",
    [lambda: rate_limited(RESET_IN), lambda: too_many_requests(RESET_IN)],
    ids=["403-ratelimit-reset", "429-retry-after"],
)
def test_an_automatic_check_waits_out_a_rate_limit_with_no_button(launch, ui_for, limit):
    stand_in = StandIn([limit, release])
    with serve(stand_in.handler()) as server:
        ctl = launch(releases_api=_url(server))
        ui = ui_for(ctl)
        wait_until(lambda: stand_in.received(1), timeout=CHECK_TIMEOUT)
        wait_until(lambda: DEFERRED in _log(ctl), timeout=CHECK_TIMEOUT)
        _stay_quiet(ui, lambda: stand_in.received(2))
        first, second = stand_in.received(2)
        # The next check returns to a quiet, current app: nothing to show.
        deadline = time.monotonic() + 3
        _stay_quiet(ui, lambda: time.monotonic() > deadline)

    gap = second.at - first.at
    assert RETRY_MARGIN + RESET_IN - 1 <= gap <= RETRY_MARGIN + RESET_IN + 10, gap
    assert stand_in.count() == 2
    assert "release update failed" not in _log(ctl)


def test_an_automatic_check_keeps_a_server_error_quiet(launch, ui_for):
    stand_in = StandIn([server_error], then=release)
    with serve(stand_in.handler()) as server:
        ctl = launch(releases_api=_url(server))
        ui = ui_for(ctl)
        wait_until(lambda: stand_in.received(1), timeout=CHECK_TIMEOUT)
        wait_until(lambda: DEFERRED in _log(ctl), timeout=CHECK_TIMEOUT)
        deadline = time.monotonic() + 3
        _stay_quiet(ui, lambda: time.monotonic() > deadline)

    # A server error names no wait: the next automatic check comes at the six-hour interval.
    assert stand_in.count() == 1
    assert "503" in _log(ctl)


@pytest.mark.covers("ui:update-status")
def test_an_automatic_check_with_no_matching_asset_shows_the_button(launch, ui_for):
    stand_in = StandIn([lambda: release(NEWER_RELEASE)], then=lambda: release(NEWER_RELEASE))
    with serve(stand_in.handler()) as server:
        ctl = launch(releases_api=_url(server))
        ui = ui_for(ctl)
        # The button carries no accessible name (QA 1028 Q1): its id and the alert it opens tell its state.
        button = ui.find("update-status", role="button", timeout=CHECK_TIMEOUT)
        assert RELEASE_DOWNLOADS not in ui.tree_dump()
        ui.click(button)
        wait_until(lambda: RELEASE_DOWNLOADS in ui.tree_dump(), timeout=ui.timeout)
        ui.press("return")
        ui.absent("update-status")
    log = _log(ctl)
    assert DEFERRED not in log
    assert re.search(r"release update failed.*Release is missing a unique", log), log


@pytest.mark.parametrize(
    ("answers", "low", "high"),
    [
        # The local token's retry meets the limit too; the alert names the wait of that last answer.
        ([rate_limited], 590, 600),
        ([lambda: too_many_requests(120)], 120, 120),
    ],
    ids=["403-ratelimit-reset", "429-retry-after"],
)
def test_check_for_updates_reports_a_rate_limit_with_the_retry_time(launch, ui_for, answers, low, high):
    stand_in = StandIn(answers)
    with serve(stand_in.handler()) as server:
        # No automatic check: only the menu's check reaches the stand-in.
        ctl = launch(releases_api=_url(server), github_token=FAKE_TOKEN, settings={"autoUpdate": False})
        ui = ui_for(ctl)
        ui.menu("Help", "Check for Updates…")
        wait_until(lambda: RELEASE_DOWNLOADS in ui.tree_dump(), timeout=CHECK_TIMEOUT)
        tree = ui.tree_dump()
        ui.press("return")
        wait_until(lambda: RELEASE_DOWNLOADS not in ui.tree_dump(), timeout=ui.timeout)

    assert "button name='OK'" in tree, tree
    # The alert's heading and body are not in the accessibility tree (QA 1028 Q2). Its body is the check's error,
    # which the app logs with it.
    log = _log(ctl)
    match = re.search(r"release update failed.*GitHub's API rate limit was reached\. Retry in (\d+) seconds\.", log)
    assert match, log
    assert low <= int(match.group(1)) <= high, match.group(0)
    assert DEFERRED not in log
