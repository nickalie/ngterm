"""The instance's agterm-scoped `<state>/config/ghostty.conf`, changed for one test and put back afterwards."""

from __future__ import annotations

import contextlib
from collections.abc import Iterator
from typing import Any

from agterm_conformance.match import assert_ok


@contextlib.contextmanager
def configured(ctl: Any, lines: str) -> Iterator[None]:
    """`lines` added to the instance's ghostty config and in force (`config.reload` with no diagnostics), the config
    put back afterwards."""
    path = ctl.instance.state_dir / "config" / "ghostty.conf"
    original = path.read_text(encoding="utf-8")
    path.write_text(original.rstrip("\n") + "\n" + lines, encoding="utf-8")
    try:
        assert assert_ok(ctl.request("config.reload")) == {"count": 0}
        yield
    finally:
        path.write_text(original, encoding="utf-8")
        ctl.request("config.reload")
