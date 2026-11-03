#!/usr/bin/env python3
"""Check an instrumented run's code coverage against the floors: no crate's line % may drop below its floor.

Reads the per-crate table of `summary.md` (agterm_conformance/code_coverage.py writes it into a `--coverage` run's
`DIR/coverage`) and a floor file (`conformance/coverage-floor-linux.txt`, `coverage-floor-windows.txt`): `#` header
comments, then one `crate line%` per line, sorted by crate. A crate more than TOLERANCE points below its floor breaks
the floors and is named with both numbers. A crate the summary measures but the file lacks is new to the workspace;
one the file lists but the summary lacks is gone from it: neither breaks. `--summary` may repeat: each crate is judged
by its lowest measurement. A crate with no lines (n/a) is neither judged nor added. Appends a markdown summary to
`$GITHUB_STEP_SUMMARY` when it is set. Exit 0: holds, 1: broken, 2: usage error or a malformed input file.

`--add` (when the floors hold) rewrites the file from a batch of at least MIN_ADD_SUMMARIES summaries of the same build:
the GUI's timing makes one run's line % vary by more than TOLERANCE (ngterm-gpui 63.80-64.38% over three runs of one
build), so a floor taken from a single lucky run would break the next. Each floor is raised to the crate's lowest line %
over the batch (rounded down to two places), never lowered; a crate measured by every summary and with no floor is
added, one no summary measures is dropped, and one only some summaries measure keeps its floor.
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
# A row of code_coverage.render's table: `| crate | 45.90% (1234/2688) | ... |` or `| crate | n/a | ... |`.
_ROW = re.compile(r"^\|\s*([^|\s]+)\s*\|\s*(n/a|[0-9.]+% \((\d+)/(\d+)\))\s*\|")
_NAME = re.compile(r"^[A-Za-z0-9_-]+$")
# The separator row under the table's header: `|---|---:|...|`.
_SEPARATOR = re.compile(r"^\|(\s*:?-+:?\s*\|)+\s*$")


class InputError(Exception):
    """A missing or malformed summary or floor file; the message names the file and, where it applies, the line."""


@dataclass
class Floors:
    header: list[str]
    floors: dict[str, float]


@dataclass
class Verdict:
    breaks: list[str] = field(default_factory=list)
    held: list[str] = field(default_factory=list)
    new: list[str] = field(default_factory=list)
    gone: list[str] = field(default_factory=list)

    @property
    def holds(self) -> bool:
        return not self.breaks

    def line(self) -> str:
        judged = len(self.breaks) + len(self.held)
        if not self.holds:
            return f"coverage floors broken: {len(self.breaks)} crate(s) of {judged} below their floor"
        notes = [f"{len(self.new)} new", f"{len(self.gone)} gone"]
        return f"coverage floors hold: {judged} crate(s) judged ({', '.join(notes)})"


def read_summary(path: Path) -> dict[str, float]:
    """Line % per crate from summary.md's table; an n/a crate (no lines) and the total row left out."""
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
    crates: dict[str, float] = {}
    total = False
    for number, line in enumerate(lines[start + 2 :], start=start + 3):
        if not line.startswith("|"):
            break
        match = _ROW.match(line)
        if match is None:
            raise InputError(f"{path}:{number}: not a crate row: {line!r}")
        name, cell, covered, count = match.groups()
        if name == "total":
            total = True
            break
        if name in crates:
            raise InputError(f"{path}:{number}: a second row for {name}")
        if cell != "n/a":
            crates[name] = 100.0 * int(covered) / int(count) if int(count) else 0.0
    if not total:
        raise InputError(f"{path}: the table has no total row (a truncated summary)")
    return crates


def parse_floors(text: str, path: Path) -> Floors:
    lines = text.split("\n")
    if lines[-1] == "":
        lines.pop()  # the file's final newline
    header: list[str] = []
    floors: dict[str, float] = {}
    for number, line in enumerate(lines, start=1):
        where = f"{path}:{number}"
        if "\r" in line:
            raise InputError(f"{where}: a carriage return (the file uses \\n line ends)")
        if line.startswith("#"):
            if floors:
                raise InputError(f"{where}: a comment after the first floor (comments go in the header)")
            header.append(line)
            continue
        parts = line.split(" ")
        if len(parts) != 2 or not _NAME.match(parts[0]):
            raise InputError(f"{where}: not `crate line%`: {line!r}")
        name, value = parts
        try:
            percent = float(value)
        except ValueError:
            raise InputError(f"{where}: not a percentage: {value!r}") from None
        if not math.isfinite(percent) or not 0.0 <= percent <= 100.0:
            raise InputError(f"{where}: out of 0..100: {value}")
        if name in floors:
            raise InputError(f"{where}: a second floor for {name}")
        if floors and name < list(floors)[-1]:
            raise InputError(f"{where}: out of order (the crates are sorted): {name}")
        floors[name] = percent
    return Floors(header, floors)


def read_floors(path: Path) -> Floors:
    try:
        text = path.read_bytes().decode("utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise InputError(f"{path}: cannot read the floor file: {error}") from error
    return parse_floors(text, path)


def judge(measured: dict[str, float], floors: Floors) -> Verdict:
    verdict = Verdict()
    for name, floor in floors.floors.items():
        if name not in measured:
            verdict.gone.append(f"{name} (floor {floor:.2f}%)")
            continue
        now = measured[name]
        line = f"{name}: {now:.2f}%, floor {floor:.2f}%"
        if floor - now > TOLERANCE + _EPSILON:
            verdict.breaks.append(f"{line} ({floor - now:.2f} pt below, more than {TOLERANCE} pt)")
        else:
            verdict.held.append(line)
    verdict.new = [f"{name}: {now:.2f}%" for name, now in measured.items() if name not in floors.floors]
    return verdict


def lowest(batch: list[dict[str, float]]) -> tuple[dict[str, float], set[str]]:
    """Each crate's lowest line % over the summaries that measure it, and the crates every summary measures."""
    measured: dict[str, float] = {}
    for crates in batch:
        for name, now in crates.items():
            measured[name] = min(now, measured.get(name, now))
    everywhere = set.intersection(*(set(crates) for crates in batch)) if batch else set()
    return measured, everywhere


def raised(measured: dict[str, float], floors: Floors, everywhere: set[str] | None = None) -> Floors:
    """The floors after a batch: each crate every summary measures raised to its lowest line % (two places, down),
    never lowered, and added when new; a crate only some summaries measure keeps its floor; one none measures dropped."""
    everywhere = set(measured) if everywhere is None else everywhere
    merged = {name: floor for name, floor in floors.floors.items() if name in measured}
    for name in everywhere:
        merged[name] = max(floors.floors.get(name, 0.0), math.floor(measured[name] * 100 + _EPSILON) / 100)
    return Floors(floors.header, dict(sorted(merged.items())))


def render_floors(floors: Floors) -> str:
    return "".join(f"{line}\n" for line in floors.header) + "".join(
        f"{name} {floor:.2f}\n" for name, floor in floors.floors.items()
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
                        help="a --coverage run's coverage/summary.md (repeatable: each crate judged by its lowest)")
    parser.add_argument("--floor", type=Path, required=True, help="the floor file (crate line%% per line)")
    parser.add_argument("--add", action="store_true",
                        help=f"raise the floors to the batch's lowest, add new crates (at least {MIN_ADD_SUMMARIES} "
                             "--summary of the same build)")
    args = parser.parse_args(argv)
    if args.add and len(args.summary) < MIN_ADD_SUMMARIES:
        print(f"check_coverage: --add needs at least {MIN_ADD_SUMMARIES} --summary of the same build (one run's "
              "line % varies by more than the tolerance)", file=sys.stderr)
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
