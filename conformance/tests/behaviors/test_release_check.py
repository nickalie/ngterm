"""The app's automatic release check at launch meets GitHub's rate limit and asks again with the local GitHub
token (`ngterm-update`'s `metadata_response` and `github_auth`), whatever GitHub then answers.

Only an instrumented build points its check at a stand-in for GitHub's API (`launch(releases_api=...)`, which
`ngterm-update` reads under `cfg(coverage)` alone), so these tests run on the runners' `--coverage` runs only. The
instance gets a fake token and none of the run's."""

from __future__ import annotations

import sys

import pytest
from agterm_conformance import llvm_profiles
from agterm_conformance.loopback import serve
from agterm_conformance.releases_api import (
    API_PATH,
    FAKE_TOKEN,
    StandIn,
    bad_credentials,
    rate_limited,
    release,
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

