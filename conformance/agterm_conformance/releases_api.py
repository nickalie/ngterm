"""A stand-in for GitHub's releases API, which an instrumented Rust build checks (`launch(releases_api=...)`).

Each answer is a function, so a rate limit's reset time comes from the clock when the request arrives."""

from __future__ import annotations

import json
import threading
import time
from collections.abc import Callable
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler

API_PATH = "/repos/nickalie/ngterm/releases/latest"
FAKE_TOKEN = "conformance-fake-github-token"
# Below every release: the check finds no newer one and downloads nothing.
OLD_RELEASE = {"tag_name": "v0.0.1", "draft": False, "prerelease": False, "assets": []}
# Above every release, with no asset for any installation: a check of it fails, and no next check clears that.
NEWER_RELEASE = {"tag_name": "v999.0.0", "draft": False, "prerelease": False, "assets": []}

Answer = tuple[int, dict[str, str], bytes]


def rate_limited(reset_in: int = 600) -> Answer:
    """GitHub's primary limit: a 403 that names the reset time. http.server adds the `Date` it is read against."""
    headers = {"x-ratelimit-remaining": "0", "x-ratelimit-reset": str(int(time.time()) + reset_in)}
    return 403, headers, b'{"message": "API rate limit exceeded"}'


def too_many_requests(retry_after: int) -> Answer:
    """GitHub's secondary limit: a 429 with `Retry-After`."""
    return 429, {"retry-after": str(retry_after)}, b'{"message": "You have exceeded a secondary rate limit."}'


def server_error() -> Answer:
    return 503, {}, b'{"message": "Service Unavailable"}'


def release(metadata: dict | None = None) -> Answer:
    return 200, {"Content-Type": "application/json"}, json.dumps(metadata or OLD_RELEASE).encode()


def bad_credentials() -> Answer:
    return 401, {}, b'{"message": "Bad credentials"}'


@dataclass(frozen=True)
class Request:
    path: str
    headers: dict[str, str]
    at: float


class StandIn:
    """Records each request with its arrival time and gives the next answer of `answers`, then `then`."""

    def __init__(self, answers: list[Callable[[], Answer]], then: Callable[[], Answer] = rate_limited) -> None:
        self.answers = answers
        self.then = then
        self.requests: list[Request] = []
        self.lock = threading.Lock()

    def handler(self) -> type[BaseHTTPRequestHandler]:
        stand_in = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:  # noqa: N802 - http.server's name
                with stand_in.lock:
                    index = len(stand_in.requests)
                    headers = {k.lower(): v for k, v in self.headers.items()}
                    stand_in.requests.append(Request(self.path, headers, time.monotonic()))
                status, headers, body = (stand_in.answers[index] if index < len(stand_in.answers) else stand_in.then)()
                self.send_response(status)
                for name, value in headers.items():
                    self.send_header(name, value)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, format: str, *args: object) -> None:  # noqa: A002 - http.server's signature
                pass

        return Handler

    def received(self, count: int) -> list[Request] | None:
        with self.lock:
            return list(self.requests) if len(self.requests) >= count else None

    def count(self) -> int:
        with self.lock:
            return len(self.requests)
