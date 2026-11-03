"""The code coverage report of an instrumented run (`run-linux.sh --coverage`, `run-windows.sh --coverage`).

After the suite, the runner calls this inside the build's `cargo llvm-cov show-env` environment:

    python -m agterm_conformance.code_coverage --profiles DIR --stage-bin DIR --release-dir DIR --repo DIR --out DIR

It refuses (exit 2, the reason on stderr after `code coverage:`) a run that left no `*.profraw` in the profile
directory, and a stage whose programs are not the instrumented build's own byte for byte (`--release-dir`, the
binaries whose coverage maps `cargo llvm-cov report` reads): its profiles would be read against the wrong maps. Otherwise it merges the profiles with `cargo llvm-cov report`
into `OUT/lcov.info`, `OUT/html/` and `OUT/summary.json`, and writes `OUT/summary.md`: line, region and function % per
workspace crate and their total. `vendor/` (patched dependencies), the `-sys` crates and their generated bindings are
left out (`EXCLUDE`).
"""

from __future__ import annotations

import argparse
import filecmp
import json
import os
import re
import subprocess
import sys
import tomllib
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from agterm_conformance.llvm_profiles import stage_binaries

# Source paths left out of the report: patched dependencies and the -sys crates (whose sources are the generated
# bindings). Both separators: llvm-cov names Windows files with backslashes.
EXCLUDE = r"([\\/]vendor[\\/]|[\\/]crates[\\/][^\\/]+-sys[\\/])"
METRICS = ("lines", "regions", "functions")
# llvm-cov's warning when a function's profile hash does not match a coverage map. A workspace's binaries share
# functions built differently (inlining, generics), so a stage that is the build's own still gets a few: noted in the
# summary, not refused. A foreign stage is refused before, by its bytes (`check_binaries`).
MISMATCH = re.compile(r"mismatched data|profile data may be out of date", re.IGNORECASE)
_ANSI = re.compile(r"\x1b\[[0-9;]*m")

Runner = Callable[..., subprocess.CompletedProcess[str]]


class ReportError(Exception):
    """The run cannot yield a code coverage report; the message says why."""


@dataclass
class Counts:
    count: int = 0
    covered: int = 0

    def add(self, summary: Mapping[str, Any]) -> None:
        self.count += int(summary.get("count", 0))
        self.covered += int(summary.get("covered", 0))

    @property
    def percent(self) -> float | None:
        return 100.0 * self.covered / self.count if self.count else None

    def cell(self) -> str:
        percent = self.percent
        return "n/a" if percent is None else f"{percent:.2f}% ({self.covered}/{self.count})"


@dataclass
class CrateCoverage:
    metrics: dict[str, Counts] = field(default_factory=lambda: {metric: Counts() for metric in METRICS})

    def add(self, summary: Mapping[str, Any]) -> None:
        for metric in METRICS:
            self.metrics[metric].add(summary.get(metric, {}))


def check_profiles(profiles: Path) -> list[Path]:
    found = sorted(profiles.glob("*.profraw")) if profiles.is_dir() else []
    if not found:
        raise ReportError(
            f"no profiles in {profiles}: no process of the build wrote one (lost-profiles.txt lists the ones the "
            "harness ended)"
        )
    return found


def check_binaries(stage_bin: Path, release: Path) -> None:
    """Every staged program of the build must be the instrumented build's own binary, whose maps the report reads."""
    staged = stage_binaries(stage_bin)
    if not staged:
        raise ReportError(f"binary mismatch: no program of the build in {stage_bin}")
    for binary in staged:
        built = release / binary.name
        if not built.is_file() or not filecmp.cmp(binary, built, shallow=False):
            raise ReportError(
                f"binary mismatch: {binary} is not {built}, whose coverage map the report reads; run --coverage "
                "without --stage, or stage that build"
            )


def crate_of(filename: str, repo: Path, names: dict[str, str]) -> str | None:
    """The workspace crate a source file belongs to (its package name), or None outside `crates/` or excluded."""
    try:
        parts = Path(filename).relative_to(repo).parts
    except ValueError:
        return None
    if len(parts) < 3 or parts[0] != "crates" or parts[1].endswith("-sys"):
        return None
    directory = parts[1]
    if directory not in names:
        names[directory] = _package_name(repo / "crates" / directory)
    name = names[directory]
    return None if name.endswith("-sys") else name


def _package_name(crate: Path) -> str:
    try:
        manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
        return str(manifest["package"]["name"])
    except (OSError, KeyError, tomllib.TOMLDecodeError):
        return crate.name


def crate_rows(export: Mapping[str, Any], repo: Path) -> dict[str, CrateCoverage]:
    """Per-file summaries of `llvm-cov export` summed per workspace crate, sorted by name."""
    rows: dict[str, CrateCoverage] = {}
    names: dict[str, str] = {}
    for data in export.get("data", []):
        for entry in data.get("files", []):
            crate = crate_of(str(entry.get("filename", "")), repo, names)
            if crate is not None:
                rows.setdefault(crate, CrateCoverage()).add(entry.get("summary", {}))
    return dict(sorted(rows.items()))


def render(rows: Mapping[str, CrateCoverage], notes: Iterable[str] = ()) -> str:
    total = CrateCoverage()
    lines = [
        "Line, region and function coverage per workspace crate (`vendor/`, the `-sys` crates and their generated "
        "bindings excluded).",
        "",
        "| crate | lines | regions | functions |",
        "|---|---:|---:|---:|",
    ]
    for name, row in rows.items():
        lines.append(f"| {name} | " + " | ".join(row.metrics[metric].cell() for metric in METRICS) + " |")
        for metric in METRICS:
            total.metrics[metric].count += row.metrics[metric].count
            total.metrics[metric].covered += row.metrics[metric].covered
    lines.append("| total | " + " | ".join(total.metrics[metric].cell() for metric in METRICS) + " |")
    lines += [f"\nllvm-cov: {note}" for note in notes]
    return "\n".join(lines) + "\n"


def _cargo(runner: Runner, repo: Path, *args: str) -> list[str]:
    """Runs one `cargo llvm-cov report`; returns llvm-cov's mismatch warnings."""
    command = ["cargo", "llvm-cov", "report", "--release", "--color", "never", "--ignore-filename-regex", EXCLUDE,
               *args]
    try:
        done = runner(command, cwd=repo, capture_output=True, text=True, check=False)
    except OSError as error:
        raise ReportError(f"cannot run cargo llvm-cov report: {error}") from None
    output = _ANSI.sub("", (done.stdout or "") + (done.stderr or ""))
    if done.returncode != 0:
        tail = "\n".join(output.strip().splitlines()[-5:])
        raise ReportError(f"cargo llvm-cov report {' '.join(args)} exited {done.returncode}: {tail}")
    return [line.strip() for line in output.splitlines() if MISMATCH.search(line)]


def report(
    profiles: Path, stage_bin: Path, release: Path, repo: Path, out: Path, runner: Runner = subprocess.run
) -> str:
    """Checks the inputs, writes the reports into `out` and returns the summary's markdown."""
    check_profiles(profiles)
    check_binaries(stage_bin, release)
    out.mkdir(parents=True, exist_ok=True)
    summary_json = out / "summary.json"
    warnings = _cargo(runner, repo, "--json", "--summary-only", "--output-path", os.fspath(summary_json))
    warnings += _cargo(runner, repo, "--lcov", "--output-path", os.fspath(out / "lcov.info"))
    warnings += _cargo(runner, repo, "--html", "--output-dir", os.fspath(out))
    try:
        export = json.loads(summary_json.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReportError(f"unreadable llvm-cov summary {summary_json}: {error}") from None
    markdown = render(crate_rows(export, repo.resolve()), dict.fromkeys(warnings))
    (out / "summary.md").write_text(markdown, encoding="utf-8")
    return markdown


def main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(prog="python -m agterm_conformance.code_coverage")
    for option in ("--profiles", "--stage-bin", "--release-dir", "--repo", "--out"):
        parser.add_argument(option, type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        markdown = report(args.profiles, args.stage_bin, args.release_dir, args.repo, args.out)
    except ReportError as error:
        print(f"code coverage: {error}", file=sys.stderr)
        return 2
    print(markdown, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
