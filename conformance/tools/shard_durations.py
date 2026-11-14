#!/usr/bin/env python3
"""Write the weights the shard split balances: each test file's seconds in a run.

    shard_durations.py REPORT... [--write FILE]

Each REPORT is a nodeid report of a whole run (`nodeids.jsonl`: a runner's, or `merge_shards.py`'s of a sharded
one), whose test records carry their `duration` (setup and teardown included). A file's seconds are summed per
report and averaged over the reports that ran it. Prints `seconds path` lines sorted by path, or writes them to FILE
(`conformance/shard-weights-<platform>.txt`, which the runners hand to `pytest --shard-weights`).

The weights only balance the shards: a file they do not name is weighed by its number of tests
(agterm_conformance/shards.py), so nothing breaks when they go stale. Refresh them from CI's merged report when its
table shows the shards far apart. Exit 2: a report that cannot be read or carries no durations.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

HEADER = (
    "# Seconds per test file in a CI run, which the shard split balances (agterm_conformance/shards.py).\n"
    "# Written by conformance/tools/shard_durations.py from a merged nodeid report; never edited by hand.\n"
)


def file_seconds(path: Path) -> dict[str, float]:
    seconds: dict[str, float] = {}
    measured = False
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{number}: not JSON: {error}") from None
        if not isinstance(record, dict) or record.get("kind") != "test":
            continue
        duration = record.get("duration")
        if isinstance(duration, (int, float)) and not isinstance(duration, bool):
            measured = True
            file = str(record.get("nodeid", "")).split("::", 1)[0]
            seconds[file] = seconds.get(file, 0.0) + float(duration)
    if not measured:
        raise ValueError(f"{path}: no test record with a duration")
    return seconds


def render(reports: list[dict[str, float]]) -> str:
    files = sorted({file for report in reports for file in report})
    lines = []
    for file in files:
        runs = [report[file] for report in reports if file in report]
        lines.append(f"{sum(runs) / len(runs):.1f} {file}\n")
    return HEADER + "".join(lines)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("reports", nargs="+", type=Path, metavar="REPORT", help="a whole run's nodeid report")
    parser.add_argument("--write", type=Path, default=None, metavar="FILE", help="write the weights to FILE")
    args = parser.parse_args(argv)
    try:
        text = render([file_seconds(path) for path in args.reports])
    except (OSError, UnicodeDecodeError, ValueError) as error:
        print(f"shard_durations: {error}", file=sys.stderr)
        return 2
    if args.write is None:
        print(text, end="")
    else:
        args.write.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    sys.exit(main())
