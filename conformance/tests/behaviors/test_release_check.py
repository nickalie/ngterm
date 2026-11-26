"""The app's automatic release check at launch meets GitHub's rate limit and asks again with the local GitHub
token (`ngterm-update`'s `metadata_response` and `github_auth`), whatever GitHub then answers.

Only an instrumented build points its check at a stand-in for GitHub's API (`launch(releases_api=...)`, which
`ngterm-update` reads under `cfg(coverage)` alone), so these tests run on the runners' `--coverage` runs only. The
instance gets a fake token and none of the run's."""

from __future__ import annotations

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
    bad_credentials,
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


@pytest.mark.parametrize("second", [release, bad_credentials], ids=["token-accepted", "token-refused"])
def test_a_rate_limited_check_asks_again_with_the_local_token(launch, second):
    stand_in = StandIn([rate_limited, second])
    with serve(stand_in.handler()) as server:
        launch(
            releases_api=f"http://127.0.0.1:{server.server_port}{API_PATH}",
            github_token=FAKE_TOKEN,
        )
        first, retry = wait_until(lambda: stand_in.received(2), timeout=CHECK_TIMEOUT)[:2]

    for request in (first, retry):
        assert request.path == API_PATH
        assert request.headers["accept"] == "application/vnd.github+json"
        assert request.headers["user-agent"].startswith("ngterm/")
    assert "authorization" not in first.headers
    assert retry.headers["authorization"] == f"Bearer {FAKE_TOKEN}"


# The updater's RETRY_MARGIN: how long after the limit resets the automatic check tries again.
RETRY_MARGIN = 30.0
# The reset in seconds that a limit names here; `Date` and the reset have whole seconds, so the wait is off by one.
RESET_IN = 2
DEFERRED = "automatic update check deferred"
FAILED = "release update failed"


def _log(ctl) -> str:
    return ctl.instance.log.read_text(encoding="utf-8", errors="replace")


@pytest.mark.parametrize(
    "limit",
    [lambda: rate_limited(RESET_IN), lambda: too_many_requests(RESET_IN)],
    ids=["403-ratelimit-reset", "429-retry-after"],
)
def test_an_automatic_check_waits_until_the_rate_limit_resets(launch, limit):
    stand_in = StandIn([limit, release])
    with serve(stand_in.handler()) as server:
        ctl = launch(releases_api=f"http://127.0.0.1:{server.server_port}{API_PATH}")
        wait_until(lambda: stand_in.received(1), timeout=CHECK_TIMEOUT)
        wait_until(lambda: DEFERRED in _log(ctl), timeout=CHECK_TIMEOUT)
        first, second = wait_until(lambda: stand_in.received(2), timeout=RETRY_MARGIN + CHECK_TIMEOUT)[:2]
        time.sleep(3)

    gap = second.at - first.at
    assert RETRY_MARGIN + RESET_IN - 1 <= gap <= RETRY_MARGIN + RESET_IN + 10, gap
    assert stand_in.count() == 2
    log = _log(ctl)
    assert log.count(DEFERRED) == 1 and FAILED not in log, log


def test_an_automatic_check_defers_a_server_error_and_fails_on_a_missing_asset(launch):
    quiet = StandIn([server_error], then=release)
    failing = StandIn([lambda: release(NEWER_RELEASE)])
    with serve(quiet.handler()) as server, serve(failing.handler()) as other:
        deferred = launch(releases_api=f"http://127.0.0.1:{server.server_port}{API_PATH}")
        failed = launch(releases_api=f"http://127.0.0.1:{other.server_port}{API_PATH}")
        wait_until(lambda: DEFERRED in _log(deferred), timeout=CHECK_TIMEOUT)
        wait_until(lambda: FAILED in _log(failed), timeout=CHECK_TIMEOUT)

    assert FAILED not in _log(deferred)
    assert "503" in _log(deferred)
    assert quiet.count() == 1
    assert DEFERRED not in _log(failed)
    assert "Release is missing a unique" in _log(failed)
