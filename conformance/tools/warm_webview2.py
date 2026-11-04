#!/usr/bin/env python3
"""Start the WebView2 runtime once before a Windows suite: an instance of the staged app opens one page and waits for
its load, then stops.

The first web view a freshly booted machine makes pays the runtime's cold start: on a CI runner the first page of the
suite took 11.5 s to make (`test_companions.py`, the first module that opens one), every later one well under a
second, each in an instance of its own. The protocol tests give a page's load the harness's read-back wait (10 s), so
whichever test opened the first page failed on the machine, not on the app. Run by `run-windows.sh` before the suite;
a warm-up that fails is reported and the suite runs anyway.

    python conformance/tools/warm_webview2.py --bin <stage>\\bin\\ngterm.exe [--timeout SECONDS]

Exits 0 once the page loaded, 1 when it did not (failed, or not within the timeout), 2 on a usage error.
"""

from __future__ import annotations

import argparse
import sys
import time
from pathlib import Path

from agterm_conformance.impls.rust import RustAdapter
from agterm_conformance.match import find_session, session_ids

PAGE = "<!doctype html><title>Warm</title><body><p>warm</p></body>"


def warm(bin: Path, timeout: float) -> str | None:
    """Opens one page in a fresh instance of `bin`; `None` once it loaded, else why not."""
    adapter = RustAdapter(bin)
    instance = adapter.launch()
    try:
        client = instance.client(timeout=timeout)
        page = instance.root / "warm.html"
        page.write_text(PAGE)
        (sid,) = session_ids(client.request("tree").result)
        opened = client.request("session.overlay.open", sid, html=str(page))
        if not opened.ok:
            return f"session.overlay.open: {opened.error}"
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            pages = find_session(client.request("tree").result, sid).get("htmlOverlays") or []
            state = pages[0].get("state") if pages else None
            if state == "loaded":
                return None
            if state == "failed":
                return f"the page failed: {pages[0].get('error')}"
            time.sleep(0.2)
        return f"the page did not load within {timeout:g} s"
    finally:
        adapter.stop(instance)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", type=Path, required=True, help="the staged ngterm.exe")
    parser.add_argument("--timeout", type=float, default=120.0, help="how long the page may take (seconds)")
    args = parser.parse_args()
    if not args.bin.is_file():
        print(f"warm_webview2: no app at {args.bin}", file=sys.stderr)
        return 2
    started = time.monotonic()
    failure = warm(args.bin, args.timeout)
    took = time.monotonic() - started
    if failure is not None:
        print(f"warm_webview2: {failure} (after {took:.1f} s)", file=sys.stderr)
        return 1
    print(f"warm_webview2: the WebView2 runtime is up ({took:.1f} s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
