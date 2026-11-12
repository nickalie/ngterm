#!/usr/bin/env python3
"""The lines one coverage run hit and another run of the same build did not: where a suite leaves code to timing.

    coverage_diff.py LCOV LCOV [LCOV...] [--label NAME]... [--repo DIR] [--max-spread PT] [--out FILE]

Reads the `lcov.info` of two or more runs of one build (`run-linux.sh --coverage`, `run-windows.sh --coverage`: their
`DIR/coverage/lcov.info`) and writes a markdown report:

- each workspace crate's line % per run and its spread (the highest run's % less the lowest's, in points);
- the varying line groups: lines every run measures that some runs hit and others missed, grouped per file by runs of
  neighbouring measured lines with the same pattern, each with the runs that hit it, the function it starts in (lcov
  has only a function's start: below a closure's start, the closure or the function around it) and, when the source
  is under `--repo`, its first line;
- lines only some runs measure (a build that differs between the runs, not timing).

Files are compared by their path from `crates/` (or the whole path, `/` separated, outside it), so runs checked out
on other roots or OSes line up. `--label` names the runs (one per LCOV, default 1, 2, ...). Exit 0, or 1 when a
crate's spread is over `--max-spread` (each named on stderr), 2 on bad input (fewer than two runs, a missing, empty,
truncated or malformed file).
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
_HASH = re.compile(r"^h[0-9a-f]{16}$")
_HASH_SUFFIX = re.compile(r"::h[0-9a-f]{16}$")
_DISAMBIGUATOR = re.compile(r"\[[0-9a-f]+\]")
_ESCAPES = {"SP": "@", "BP": "*", "RF": "&", "LT": "<", "GT": ">", "LP": "(", "RP": ")", "C": ","}
_ESCAPE = re.compile(r"\$([A-Z]+|u[0-9a-f]+)\$")
# a record line inside an SF record: `DA:`, `FN:`, `FNDA:`, `LF:`, `BRDA:` and the like
_RECORD = re.compile(r"^[A-Z]+:")
_CLOSURE = re.compile(r"(\{\{closure\}\}|\{closure#\d+\})$")


class InputError(Exception):
    """A missing or malformed input; the message names the file and, where it applies, the line."""


@dataclass
class Run:
    # per file: line number -> count
    lines: dict[str, dict[int, int]] = field(default_factory=dict)
    # per file: function start line -> name
    functions: dict[str, dict[int, str]] = field(default_factory=dict)


def repo_path(name: str) -> str:
    """`name` with `/` separators, from its `crates/` segment on when it has one."""
    parts = name.replace("\\", "/").split("/")
    if "crates" in parts:
        parts = parts[parts.index("crates") :]
    return "/".join(parts)


def read_lcov(path: Path) -> Run:
    """One run's lcov; an empty, truncated or non-lcov file is an `InputError`, never an empty run."""
    try:
        text = path.read_bytes().decode("utf-8-sig")
    except (OSError, UnicodeDecodeError) as error:
        raise InputError(f"{path}: cannot read: {error}") from error
    run = Run()
    current: str | None = None
    for number, line in enumerate(text.splitlines(), start=1):
        try:
            if line.startswith("SF:"):
                if current is not None:
                    raise ValueError
                current = repo_path(line[3:])
                run.lines.setdefault(current, {})
                run.functions.setdefault(current, {})
            elif current is None:
                # between records: only a test name or a blank line
                if line.strip() and not line.startswith("TN:"):
                    raise ValueError
            elif line == "end_of_record":
                current = None
            elif line.startswith("DA:"):
                line_number, count = (int(part) for part in line[3:].split(",")[:2])
                lines = run.lines[current]
                lines[line_number] = max(lines.get(line_number, 0), count)
            elif line.startswith("FN:"):
                start, name = line[3:].split(",", 1)
                run.functions[current].setdefault(int(start), name)
            elif not _RECORD.match(line):
                raise ValueError
        except ValueError:
            raise InputError(f"{path}:{number}: not an lcov record: {line!r}") from None
    if current is not None:
        raise InputError(f"{path}: the record of {current} has no end_of_record (a truncated file?)")
    if not run.lines:
        raise InputError(f"{path}: no SF record (an empty or non-lcov file?)")
    return run


def demangle(symbol: str) -> str:
    """A legacy-mangled Rust symbol (`_ZN...E`) as its path, hash dropped; any other symbol, or one with a malformed
    escape, as it is."""
    if not symbol.startswith("_ZN") or not symbol.endswith("E"):
        return symbol
    body, parts = symbol[3:-1], []
    while body:
        digits = re.match(r"\d+", body)
        if not digits:
            return symbol
        size = int(digits.group())
        start = digits.end()
        parts.append(body[start : start + size])
        body = body[start + size :]
    if parts and _HASH.match(parts[-1]):
        parts.pop()

    def char(escape: re.Match[str]) -> str:
        code = escape.group(1)
        if code in _ESCAPES:
            return _ESCAPES[code]
        if code[0] != "u":
            return escape.group()
        value = int(code[1:], 16)
        if value > 0x10FFFF or 0xD800 <= value <= 0xDFFF:
            raise ValueError(code)
        return chr(value)

    def unescape(part: str) -> str:
        return _ESCAPE.sub(char, part.replace("..", "::"))

    try:
        return "::".join(unescape(part.removeprefix("_")) if part.startswith("_$") else unescape(part) for part in parts)
    except ValueError:
        return symbol


def crate_of(path: str, repo: Path, names: dict[str, str]) -> str:
    parts = path.split("/")
    if len(parts) < 3 or parts[0] != "crates":
        return "(outside crates/)"
    directory = parts[1]
    if directory not in names:
        try:
            manifest = tomllib.loads((repo / "crates" / directory / "Cargo.toml").read_text(encoding="utf-8"))
            names[directory] = str(manifest["package"]["name"])
        except (OSError, KeyError, tomllib.TOMLDecodeError):
            names[directory] = directory
    return names[directory]


@dataclass
class Group:
    path: str
    first: int
    last: int
    lines: int
    hit: tuple[bool, ...]


def varying_groups(runs: Sequence[Run]) -> tuple[list[Group], list[tuple[str, int]]]:
    """The groups of lines every run measures but not every run hit, and the lines only some runs measure."""
    groups: list[Group] = []
    partial: list[tuple[str, int]] = []
    for path in sorted({path for run in runs for path in run.lines}):
        measured = [run.lines.get(path, {}) for run in runs]
        numbers = sorted({number for lines in measured for number in lines})
        current: Group | None = None
        for number in numbers:
            if any(number not in lines for lines in measured):
                partial.append((path, number))
                continue
            hit = tuple(lines[number] > 0 for lines in measured)
            if all(hit) or not any(hit):
                current = None
                continue
            if current is not None and current.hit == hit:
                current.last, current.lines = number, current.lines + 1
            else:
                current = Group(path, number, number, 1, hit)
                groups.append(current)
    return groups, partial


def demangle_all(symbols: Sequence[str]) -> dict[str, str]:
    """Each symbol demangled, crate disambiguators (`[95244b09b13463f6]`) and legacy hashes dropped: a legacy one
    (`_ZN...E`) by `demangle`, the rest (Rust's v0 `_R...`) by `c++filt` or `llvm-cxxfilt` when one is on PATH; LLVM's
    demangler leaves legacy escapes (`$LT$`, `..`) as they are, so it never gets those."""
    names = {symbol: demangle(symbol) for symbol in set(symbols)}
    rest = sorted(symbol for symbol, name in names.items() if name == symbol)
    tool = shutil.which("c++filt") or shutil.which("llvm-cxxfilt")
    if tool and rest:
        try:
            done = subprocess.run([tool], input="\n".join(rest) + "\n", capture_output=True, text=True, check=False)
            lines = done.stdout.splitlines()
            if done.returncode == 0 and len(lines) == len(rest):
                names.update(zip(rest, lines, strict=True))
        except OSError:
            pass
    return {symbol: _HASH_SUFFIX.sub("", _DISAMBIGUATOR.sub("", name)) for symbol, name in names.items()}


def function_starts(runs: Sequence[Run], path: str) -> dict[int, str]:
    """The mangled names of the functions in `path` by their start line, over all runs."""
    starts: dict[int, str] = {}
    for run in runs:
        for start, name in run.functions.get(path, {}).items():
            starts.setdefault(start, name)
    return starts


def function_at(starts: Mapping[int, str], readable: Mapping[str, str], line: int) -> str | None:
    """Where `line` is, from the function starting closest above it. lcov gives a function its start only, so when
    that one is a closure the line is in it or after it, in the closest function above it that is not a closure."""
    before = sorted((start for start in starts if start <= line), reverse=True)
    if not before:
        return None
    nearest = readable[starts[before[0]]]
    if not _CLOSURE.search(nearest):
        return f"in `{nearest}`"
    outer = next((readable[starts[start]] for start in before if not _CLOSURE.search(readable[starts[start]])), None)
    return f"in `{nearest}`" + (f", or after it in `{outer}`" if outer else "")


def source_line(repo: Path, path: str, line: int) -> str | None:
    """`path`'s line `line` when the file is under `repo`, else None (an absolute or `..` path quotes nothing)."""
    try:
        source = (repo / path).resolve()
        if not source.is_relative_to(repo.resolve()):
            return None
        lines = source.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    return lines[line - 1].strip() if 0 < line <= len(lines) else None


def percent(covered: int, count: int) -> float:
    return 100.0 * covered / count if count else 0.0


def crate_table(runs: Sequence[Run], repo: Path) -> dict[str, list[tuple[int, int]]]:
    """Each crate's (covered, count) lines per run, sorted by crate, with `total` last."""
    names: dict[str, str] = {}
    table: dict[str, list[tuple[int, int]]] = {}
    for index, run in enumerate(runs):
        for path, lines in run.lines.items():
            for crate in (crate_of(path, repo, names), "total"):
                row = table.setdefault(crate, [(0, 0)] * len(runs))
                covered, count = row[index]
                row[index] = (covered + sum(1 for value in lines.values() if value), count + len(lines))
    total = table.pop("total", [(0, 0)] * len(runs))
    return {**dict(sorted(table.items())), "total": total}


def spread(row: Sequence[tuple[int, int]]) -> float:
    values = [percent(*cell) for cell in row]
    return max(values) - min(values)


def render(runs: Sequence[Run], labels: Sequence[str], repo: Path) -> tuple[str, dict[str, float]]:
    table = crate_table(runs, repo)
    out = [
        f"Line coverage of {len(runs)} runs of one build, per crate, and the lines some runs hit and others missed.",
        "",
        "| crate | " + " | ".join(labels) + " | spread |",
        "|---|" + "---:|" * (len(labels) + 1),
    ]
    spreads = {crate: spread(row) for crate, row in table.items()}
    for crate, row in table.items():
        cells = [f"{percent(*cell):.2f}% ({cell[0]}/{cell[1]})" for cell in row]
        out.append(f"| {crate} | " + " | ".join(cells) + f" | {spreads[crate]:.2f} |")
    groups, partial = varying_groups(runs)
    names: dict[str, str] = {}
    starts = {path: function_starts(runs, path) for path in {group.path for group in groups}}
    readable = demangle_all([name for names in starts.values() for name in names.values()])
    out += ["", f"## Varying line groups ({len(groups)}, {sum(group.lines for group in groups)} lines)", ""]
    if not groups:
        out.append("no line varies between the runs")
    for group in sorted(groups, key=lambda g: (crate_of(g.path, repo, names), g.path, g.first)):
        where = f"{group.path}:{group.first}" + (f"-{group.last}" if group.last != group.first else "")
        hit = ", ".join(label for label, ran in zip(labels, group.hit, strict=True) if ran)
        missed = ", ".join(label for label, ran in zip(labels, group.hit, strict=True) if not ran)
        line = f"- `{where}` ({group.lines} line{'s' if group.lines != 1 else ''}): hit by {hit}; missed by {missed}"
        function = function_at(starts[group.path], readable, group.first)
        if function:
            line += f"; {function}"
        text = source_line(repo, group.path, group.first)
        if text:
            line += f"\n  `{text}`"
        out.append(line)
    if partial:
        out += ["", f"## Lines measured in some runs only ({len(partial)})", ""]
        out += [f"- `{path}:{number}`" for path, number in partial]
    return "\n".join(out) + "\n", spreads


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("lcov", type=Path, nargs="+", help="a run's lcov.info (two or more, one build)")
    parser.add_argument("--label", action="append", default=[], help="a run's name, one per LCOV in order")
    parser.add_argument("--repo", type=Path, default=REPO, help="the checkout the sources and crate names come from")
    parser.add_argument("--max-spread", type=float, help="exit 1 when a crate's spread is over this many points")
    parser.add_argument("--out", type=Path, help="write the report here instead of stdout")
    args = parser.parse_args(argv)
    try:
        if len(args.lcov) < 2:
            raise InputError(f"needs at least 2 runs to compare, got {len(args.lcov)}")
        if args.label and len(args.label) != len(args.lcov):
            raise InputError(f"--label given {len(args.label)} times for {len(args.lcov)} runs")
        runs = [read_lcov(path) for path in args.lcov]
    except InputError as error:
        print(f"coverage_diff: {error}", file=sys.stderr)
        return 2
    labels = args.label or [str(index) for index in range(1, len(runs) + 1)]
    report, spreads = render(runs, labels, args.repo)
    if args.out:
        args.out.write_text(report, encoding="utf-8")
    else:
        sys.stdout.write(report)
    over = _over(spreads, args.max_spread)
    for crate, value in over.items():
        print(f"coverage_diff: {crate} varies by {value:.2f} pt, over {args.max_spread:g}", file=sys.stderr)
    return 1 if over else 0


def _over(spreads: Mapping[str, float], limit: float | None) -> dict[str, float]:
    if limit is None:
        return {}
    return {crate: value for crate, value in spreads.items() if value > limit + 1e-9}


if __name__ == "__main__":
    sys.exit(main())
