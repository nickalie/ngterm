#!/usr/bin/env python3
"""Check a run's code coverage against the floors: no crate's coverage may drop below its floor.

Reads the per-crate table of `summary.md` (agterm_conformance/code_coverage.py writes it into a `--coverage` run's
`DIR/coverage`, scripts/unit-coverage.sh into its OUT_DIR) and a floor file: `#` header comments, then one floor line
per crate, sorted by crate, in one of two forms throughout the file:

- `crate line%`: the line % alone (the suite's `conformance/coverage-floor-linux.txt`, `coverage-floor-windows.txt`);
- `crate lines% functions% regions%`: the three of them (`scripts/unit-coverage-floor-linux.txt`).

A crate more than TOLERANCE points below its floor in any of them breaks the floors and is named with the metric and
both numbers. A crate the summary measures but the file lacks is new to the workspace; one the file lists but the
summary lacks is gone from it: neither breaks. `--summary` may repeat: each crate's metric is judged by its lowest
measurement. A crate with no lines (an n/a cell) is neither judged nor added. Appends a markdown summary to
`$GITHUB_STEP_SUMMARY` when it is set. Exit 0: holds, 1: broken, 2: usage error or a malformed input file.

`--add` (when the floors hold) rewrites the file from a batch of at least MIN_ADD_SUMMARIES summaries of the same build:
the GUI's timing makes one run's line % vary by more than TOLERANCE (ngterm-gpui 63.80-64.38% over three runs of one
build), so a floor taken from a single lucky run would break the next. Each floor is raised to the crate's lowest %
over the batch (rounded down to two places), never lowered; a crate measured by every summary and with no floor is
added, one no summary measures is dropped, and one only some summaries measure keeps its floor. The file keeps its
form; one with no floor yet takes the `crate line%` form.
"""

from __future__ import annotations

import argparse
import math
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# The drop below a floor that still holds, in percentage points: the run-to-run noise of a suite with timing in it.
TOLERANCE = 0.5
# The fewest summaries `--add` raises the floors from: a batch's minimum, not one run's luck.
MIN_ADD_SUMMARIES = 2
# Below this a difference of two percentages is float noise, not a drop (floors keep two decimals).
_EPSILON = 1e-9
# The metrics a floor line holds, by its number of values, in the file's column order.
FORMS = {1: ("lines",), 3: ("lines", "functions", "regions")}
# A cell of code_coverage.render's table: `45.90% (1234/2688)` or `n/a`.
_CELL = re.compile(r"^(?:n/a|[0-9.]+% \((\d+)/(\d+)\))$")
_NAME = re.compile(r"^[A-Za-z0-9_-]+$")
# The separator row under the table's header: `|---|---:|...|`.
_SEPARATOR = re.compile(r"^\|(\s*:?-+:?\s*\|)+\s*$")


class InputError(Exception):
    """A missing or malformed summary or floor file; the message names the file and, where it applies, the line."""


# A crate's measurement: % per metric (lines, functions, regions).
Coverage = dict[str, float]


@dataclass
class Floors:
    header: list[str]
    # each crate's floors, one per metric of `metrics`
    floors: dict[str, tuple[float, ...]]
    metrics: tuple[str, ...] = FORMS[1]


@dataclass
class Verdict:
    breaks: list[str] = field(default_factory=list)
    new: list[str] = field(default_factory=list)
    gone: list[str] = field(default_factory=list)
    judged: int = 0
    broken: set[str] = field(default_factory=set)

    @property
    def holds(self) -> bool:
        return not self.breaks

    def line(self) -> str:
        judged = self.judged
        if not self.holds:
            return f"coverage floors broken: {len(self.broken)} crate(s) of {judged} below their floor"
        notes = [f"{len(self.new)} new", f"{len(self.gone)} gone"]
        return f"coverage floors hold: {judged} crate(s) judged ({', '.join(notes)})"


def _cells(line: str) -> list[str]:
    return [cell.strip() for cell in line.strip().strip("|").split("|")]


def read_summary(path: Path) -> dict[str, Coverage]:
    """Each crate's % per metric from summary.md's table; a crate with an n/a cell (no lines) and the total row left
    out."""
    try:
        text = path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise InputError(f"{path}: cannot read the coverage summary: {error}") from error
    lines = text.splitlines()
    try:
        start = next(i for i, line in enumerate(lines) if line.replace(" ", "").startswith("|crate|lines|"))
    except StopIteration:
        raise InputError(f"{path}: no per-crate table (`| crate | lines | ...`)") from None
    separator = lines[start + 1] if start + 1 < len(lines) else ""
    if not _SEPARATOR.match(separator):
        raise InputError(f"{path}:{start + 2}: no separator row under the table's header: {separator!r}")
    columns = _cells(lines[start])[1:]
    lacking = [metric for metric in FORMS[3] if metric not in columns]
    if lacking:
        raise InputError(f"{path}:{start + 1}: the table has no {', '.join(lacking)} column")
    crates: dict[str, Coverage] = {}
    total = False
    for number, line in enumerate(lines[start + 2 :], start=start + 3):
        if not line.startswith("|"):
            break
        name, *cells = _cells(line)
        matches = [_CELL.match(cell) for cell in cells]
        if not _NAME.match(name) or len(cells) != len(columns) or None in matches:
            raise InputError(f"{path}:{number}: not a crate row: {line!r}")
        if name == "total":
            total = True
            break
        if name in crates:
            raise InputError(f"{path}:{number}: a second row for {name}")
        counts = [match.groups() for match in matches if match is not None and match.group(1) is not None]
        if len(counts) == len(columns):
            crates[name] = {
                metric: 100.0 * int(covered) / int(count) if int(count) else 0.0
                for metric, (covered, count) in zip(columns, counts)
            }
    if not total:
        raise InputError(f"{path}: the table has no total row (a truncated summary)")
    return crates


def parse_floors(text: str, path: Path) -> Floors:
    lines = text.split("\n")
    if lines[-1] == "":
        lines.pop()  # the file's final newline
    header: list[str] = []
    floors: dict[str, tuple[float, ...]] = {}
    metrics = FORMS[1]
    for number, line in enumerate(lines, start=1):
        where = f"{path}:{number}"
        if "\r" in line:
            raise InputError(f"{where}: a carriage return (the file uses \\n line ends)")
        if line.startswith("#"):
            if floors:
                raise InputError(f"{where}: a comment after the first floor (comments go in the header)")
            header.append(line)
            continue
        name, *values = line.split(" ")
        if len(values) not in FORMS or not _NAME.match(name):
            raise InputError(f"{where}: not `crate line%` or `crate lines% functions% regions%`: {line!r}")
        if floors and FORMS[len(values)] != metrics:
            raise InputError(f"{where}: {len(values)} value(s), where the file's floors have {len(metrics)}")
        metrics = FORMS[len(values)]
        if name in floors:
            raise InputError(f"{where}: a second floor for {name}")
        if floors and name < list(floors)[-1]:
            raise InputError(f"{where}: out of order (the crates are sorted): {name}")
        floors[name] = tuple(_percent(value, where) for value in values)
    return Floors(header, floors, metrics)


def _percent(value: str, where: str) -> float:
    try:
        percent = float(value)
    except ValueError:
        raise InputError(f"{where}: not a percentage: {value!r}") from None
    if not math.isfinite(percent) or not 0.0 <= percent <= 100.0:
        raise InputError(f"{where}: out of 0..100: {value}")
    return percent


def read_floors(path: Path) -> Floors:
    try:
        text = path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise InputError(f"{path}: cannot read the floor file: {error}") from error
    return parse_floors(text, path)


def _described(metrics: tuple[str, ...], values: list[float] | tuple[float, ...]) -> str:
    """`61.23%` for the line % alone, `lines 61.23%, functions 58.00%, regions 60.10%` for the three."""
    if len(metrics) == 1:
        return f"{values[0]:.2f}%"
    return ", ".join(f"{metric} {value:.2f}%" for metric, value in zip(metrics, values))


def judge(measured: dict[str, Coverage], floors: Floors) -> Verdict:
    verdict = Verdict()
    for name, values in floors.floors.items():
        if name not in measured:
            verdict.gone.append(f"{name} (floor {_described(floors.metrics, values)})")
            continue
        verdict.judged += 1
        for metric, floor in zip(floors.metrics, values):
            now = measured[name][metric]
            label = name if len(floors.metrics) == 1 else f"{name} {metric}"
            if floor - now > TOLERANCE + _EPSILON:
                verdict.broken.add(name)
                verdict.breaks.append(
                    f"{label}: {now:.2f}%, floor {floor:.2f}% ({floor - now:.2f} pt below, more than {TOLERANCE} pt)"
                )
    verdict.new = [
        f"{name}: {_described(floors.metrics, [now[metric] for metric in floors.metrics])}"
        for name, now in measured.items()
        if name not in floors.floors
    ]
    return verdict


def lowest(batch: list[dict[str, Coverage]]) -> tuple[dict[str, Coverage], set[str]]:
    """Each crate's lowest % per metric over the summaries that measure it, and the crates every summary measures."""
    measured: dict[str, Coverage] = {}
    for crates in batch:
        for name, now in crates.items():
            low = measured.setdefault(name, dict(now))
            for metric, value in now.items():
                low[metric] = min(value, low.get(metric, value))
    everywhere = set.intersection(*(set(crates) for crates in batch)) if batch else set()
    return measured, everywhere


def raised(measured: dict[str, Coverage], floors: Floors, everywhere: set[str] | None = None) -> Floors:
    """The floors after a batch: each crate every summary measures raised to its lowest % (two places, down), never
    lowered, and added when new; a crate only some summaries measure keeps its floor; one none measures dropped."""
    everywhere = set(measured) if everywhere is None else everywhere
    merged = {name: values for name, values in floors.floors.items() if name in measured}
    for name in everywhere:
        old = floors.floors.get(name, (0.0,) * len(floors.metrics))
        merged[name] = tuple(
            max(floor, math.floor(measured[name][metric] * 100 + _EPSILON) / 100)
            for metric, floor in zip(floors.metrics, old)
        )
    return Floors(floors.header, dict(sorted(merged.items())), floors.metrics)


def render_floors(floors: Floors) -> str:
    return "".join(f"{line}\n" for line in floors.header) + "".join(
        f"{name} {' '.join(f'{value:.2f}' for value in values)}\n" for name, values in floors.floors.items()
    )


def print_verdict(verdict: Verdict) -> None:
    for line in verdict.breaks:
        print(f"BROKEN {line}")
    for line in verdict.new:
        print(f"new, no floor yet: {line}")
    for line in verdict.gone:
        print(f"gone from the run, floor dropped by --add: {line}")
    print(verdict.line())


def summary_markdown(verdict: Verdict) -> str:
    parts = ["### Coverage floors", "", f"**{verdict.line()}**", ""]
    for title, lines in (
        ("Below their floor", verdict.breaks),
        ("New, no floor yet", verdict.new),
        ("Gone from the run", verdict.gone),
    ):
        if lines:
            parts += [f"{title}:", "", "```text", *lines, "```", ""]
    return "\n".join(parts) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--summary", type=Path, action="append", required=True,
                        help="a coverage run's summary.md (repeatable: each crate judged by its lowest)")
    parser.add_argument("--floor", type=Path, required=True,
                        help="the floor file (`crate line%%` or `crate lines%% functions%% regions%%` per line)")
    parser.add_argument("--add", action="store_true",
                        help=f"raise the floors to the batch's lowest, add new crates (at least {MIN_ADD_SUMMARIES} "
                             "--summary of the same build)")
    args = parser.parse_args(argv)
    if args.add and len(args.summary) < MIN_ADD_SUMMARIES:
        print(f"check_coverage: --add needs at least {MIN_ADD_SUMMARIES} --summary of the same build (one run's "
              "% varies by more than the tolerance)", file=sys.stderr)
        return 2
    try:
        measured, everywhere = lowest([read_summary(path) for path in args.summary])
        floors = read_floors(args.floor)
    except InputError as error:
        print(f"check_coverage: {error}", file=sys.stderr)
        return 2
    verdict = judge(measured, floors)
    print_verdict(verdict)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as stream:
            stream.write(summary_markdown(verdict))
    if not verdict.holds:
        if args.add:
            print("check_coverage: --add skipped: the floors are broken", file=sys.stderr)
        return 1
    if args.add:
        updated = raised(measured, floors, everywhere)
        if updated != floors:
            args.floor.write_bytes(render_floors(updated).encode("utf-8"))
            print(f"floors updated in {args.floor}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
