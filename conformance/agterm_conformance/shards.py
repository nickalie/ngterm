"""`--shard I/N`: run one of N parts of the collection, so N jobs run the suite side by side.

The unit is the test file: a file's tests stay together and in their order, so a module's shared instance and a test
that relies on the one before it in its file (the dashboard and zoom tests on one shared instance) work as in a whole
run. No list names the files: every shard collects the whole suite and splits it the same way, so a new file lands in
a shard by itself.

The split weighs each file by its recorded duration (`--shard-weights FILE`: `seconds path` lines, written by
`tools/shard_durations.py` from a run's nodeid report) and gives the files, heaviest first, to the lightest shard so
far. A file the weights do not name weighs its number of tests times the mean recorded seconds per test, so stale or
missing weights cost balance only, never a test. The nodeid report's run header then carries
`"shard": {"index", "count", "collected", "split"}`: `split` is a digest of the whole file-to-shard table, which
`tools/merge_shards.py` checks to be the same in every shard before it joins their reports, with every index there
once. Such a report alone is judged like a partial run (`tools/check_ratchet.py`); the merged one is judged as the
whole run it is.
"""

from __future__ import annotations

import hashlib
import re
from collections.abc import Iterable, Mapping
from dataclasses import dataclass
from pathlib import Path

import pytest

PLUGIN_NAME = "agterm-shard"
_SHARD = re.compile(r"([1-9][0-9]*)/([1-9][0-9]*)")


@dataclass(frozen=True)
class Shard:
    index: int
    count: int

    def __str__(self) -> str:
        return f"{self.index}/{self.count}"


def parse(text: str) -> Shard:
    found = _SHARD.fullmatch(text)
    if found is None or int(found.group(1)) > int(found.group(2)):
        raise pytest.UsageError(f"--shard: {text!r} is not I/N with 1 <= I <= N")
    return Shard(int(found.group(1)), int(found.group(2)))


def file_of(nodeid: str) -> str:
    """The test file of a node id, as the id spells it."""
    return nodeid.split("::", 1)[0]


def parse_weights(text: str, where: str) -> dict[str, float]:
    """`seconds path` lines after `#` comments; the path is the rest of the line."""
    weights: dict[str, float] = {}
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip() or line.startswith("#"):
            continue
        seconds, _, path = line.partition(" ")
        try:
            weight = float(seconds)
        except ValueError:
            weight = -1.0
        if not path or weight < 0 or weight != weight or weight == float("inf"):
            raise ValueError(f"{where}:{number}: not `seconds path`: {line!r}")
        if path in weights:
            raise ValueError(f"{where}:{number}: a second weight for {path}")
        weights[path] = weight
    return weights


def read_weights(path: Path) -> dict[str, float]:
    try:
        return parse_weights(path.read_text(encoding="utf-8"), str(path))
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise pytest.UsageError(f"--shard-weights: {error}") from None


def file_weights(tests: Mapping[str, int], recorded: Mapping[str, float]) -> dict[str, float]:
    """Each file's weight: its recorded seconds, else its number of tests times the mean recorded seconds per test of
    the files that have a record (one second per test when none has)."""
    known_seconds = sum(recorded[file] for file in tests if file in recorded)
    known_tests = sum(count for file, count in tests.items() if file in recorded)
    per_test = known_seconds / known_tests if known_tests and known_seconds else 1.0
    return {file: recorded.get(file, per_test * count) for file, count in tests.items()}


def assign(tests: Mapping[str, int], recorded: Mapping[str, float], count: int) -> dict[str, int]:
    """Each file's shard (1-based): the files, heaviest first (by path among equals), each to the lightest shard so
    far (the lowest index among equals)."""
    weights = file_weights(tests, recorded)
    loads = [0.0] * count
    assignment: dict[str, int] = {}
    for file in sorted(weights, key=lambda name: (-weights[name], name)):
        index = min(range(count), key=lambda shard: (loads[shard], shard))
        loads[index] += weights[file]
        assignment[file] = index + 1
    return assignment


def split_digest(assignment: Mapping[str, int]) -> str:
    """What two shards of one split agree on: the whole file-to-shard table."""
    table = "".join(f"{file}\t{index}\n" for file, index in sorted(assignment.items()))
    return hashlib.sha256(table.encode("utf-8")).hexdigest()


def tests_per_file(nodeids: Iterable[str]) -> dict[str, int]:
    tests: dict[str, int] = {}
    for nodeid in nodeids:
        file = file_of(nodeid)
        tests[file] = tests.get(file, 0) + 1
    return tests


def capability_problems(counts: Mapping[str, Mapping[str, int]]) -> list[str]:
    """The `--must-pass` problems of whole capabilities: none of the run's tests requires it, or none of them passed."""
    return [f"no collected test requires {capability}" for capability in sorted(counts)
            if not counts[capability]["required"]] + [
        f"no test requiring {capability} passed" for capability in sorted(counts)
        if counts[capability]["required"] and not counts[capability]["passed"]
    ]


class Split:
    """Keeps this shard's files and deselects the rest, before any other selection: the split is of the whole
    collection, whatever `-k`, `-m` or `--ratchet-only` then narrow in it."""

    def __init__(self, shard: Shard, recorded: Mapping[str, float]) -> None:
        self.shard = shard
        self.recorded = dict(recorded)
        self.collected = 0
        self.split = ""

    def header(self) -> dict[str, object]:
        return {"index": self.shard.index, "count": self.shard.count, "collected": self.collected, "split": self.split}

    # tryfirst and registered last (plugin.pytest_configure): ahead of `--must-pass`, which then holds only this
    # shard's tests to its capabilities, and of pytest's own -k/-m.
    @pytest.hookimpl(tryfirst=True)
    def pytest_collection_modifyitems(self, config: pytest.Config, items: list[pytest.Item]) -> None:
        assignment = assign(tests_per_file(item.nodeid for item in items), self.recorded, self.shard.count)
        self.collected = len(items)
        self.split = split_digest(assignment)
        kept = [item for item in items if assignment[file_of(item.nodeid)] == self.shard.index]
        dropped = [item for item in items if assignment[file_of(item.nodeid)] != self.shard.index]
        if dropped:
            # the other shards run these: the nodeid report must not read the run as narrowed by them
            report = config.pluginmanager.get_plugin("agterm-nodeid-report")
            partial = report.partial if report is not None else False
            config.hook.pytest_deselected(items=dropped)
            if report is not None:
                report.partial = partial
        items[:] = kept
