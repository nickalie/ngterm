"""The code coverage report of an instrumented run (`run-linux.sh --coverage`, `run-windows.sh --coverage`).

After the suite, the runner calls this inside the build's `cargo llvm-cov show-env` environment:

    python -m agterm_conformance.code_coverage --profiles DIR --stage-bin DIR --release-dir DIR --repo DIR --out DIR

It refuses (exit 2, the reason on stderr after `code coverage:`) a run that left no `*.profraw` in the profile
directory, and a stage whose programs are not the instrumented build's own byte for byte (`--release-dir`, the
binaries whose coverage maps the report reads): its profiles would be read against the wrong maps. Otherwise it merges
the profiles (`llvm-profdata merge`), reads them against each staged program on its own (`llvm-cov export`) and joins
those: a line, a region or a function counts as run when any program ran it. It writes `OUT/lcov.info` and
`OUT/summary.json` from the join, `OUT/html/` (`cargo llvm-cov report --html`, a browsing aid only) and
`OUT/summary.md`: line, region and function % per workspace crate and their total. `vendor/` (patched dependencies),
the `-sys` crates and their generated bindings are left out (`EXCLUDE`).

Why each program on its own: given several objects, llvm-cov keeps the first record it meets of a function and drops
the rest. A function every program links but only some run (a small one inlined into its callers, such as
`ngterm_proto::Request::new` in `ngtermctl`) has an unused, never-counted record in the others, so whichever object
comes first decides; `cargo llvm-cov report` takes every program of the target directory (the Android library and the
programs the suite never starts too) in the filesystem's order, and a run's counts vanished that way.

`scripts/unit-coverage.sh` makes the unit-test report the same way, from the programs its `cargo test` build built:

    python -m agterm_conformance.code_coverage unit --messages FILE --profiles DIR --repo DIR --out DIR \
        [--crate NAME]...

with test code left out of every metric (rust_test_code.py) and a per-file table beside the per-crate one
(`unit_report`).

The e2e report of one app on one platform (`e2e_report`), the line coverage its real programs got from tests that
drive them from outside:

    python -m agterm_conformance.code_coverage e2e --app APP --platform OS --repo DIR --out DIR \
        [--profiles DIR --release-dir DIR [--stage-bin DIR]] [--lcov FILE]... [--apps FILE]

reads the profiles against the app's own programs alone (`conformance/coverage/apps.toml`: another app's never count)
and joins the lcov files given (a Kotlin or Swift adapter's output, or other jobs' e2e `lcov.info`: a line run in any
counts as run). Every input reduces to lcov records, `SF` per file, `DA` per line and the `FN`/`FNDA` function records
(a function's first line, inside the span its source marks), which is the interface an adapter writes. Only the app's
files are kept: its modules' (apps.toml) and its workspace crates', `vendor/`, the `-sys` crates and the app's
`exclude` patterns left out. The `// e2e-coverage: off` markers of the measured sources (e2e_exclusions.py) leave
their code out and are listed; a bad marker, a measured source missing from `--repo`, or a missing input exits 2. It
writes `OUT/lcov.info` (the app's data before the markers, repo-relative paths: what a later merge joins),
`OUT/lcov-measured.info` (after them), `OUT/summary.json` and `OUT/summary.md`: the total line % and its distance to
90%, the excluded share (excluded lines out of measured and excluded) and per crate or module, and the marker list.
"""

from __future__ import annotations

import argparse
import filecmp
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from agterm_conformance.e2e_exclusions import FILE, Marker, MarkerError, find_markers, language_of
from agterm_conformance.llvm_profiles import stage_binaries
from agterm_conformance.rust_test_code import TestCode, find_test_code

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

    @classmethod
    def of(cls, summary: Mapping[str, Any]) -> Counts:
        counts = cls()
        counts.add(summary)
        return counts

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


SUITE_INTRO = (
    "Line, region and function coverage per workspace crate (`vendor/`, the `-sys` crates and their generated "
    "bindings excluded)."
)


def render(rows: Mapping[str, CrateCoverage], notes: Iterable[str] = (), intro: str = SUITE_INTRO) -> str:
    total = CrateCoverage()
    lines = [
        intro,
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
    """Runs one `cargo llvm-cov report` (the html); returns llvm-cov's mismatch warnings."""
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


def llvm_tool(name: str, runner: Runner = subprocess.run) -> str:
    """The toolchain's `llvm-tools` program `name` (`llvm-cov`, `llvm-profdata`) where cargo-llvm-cov finds it: the
    `LLVM_COV` / `LLVM_PROFDATA` override, else the `bin` directory beside the host's target libdir."""
    override = os.environ.get(name.upper().replace("-", "_"))
    if override:
        return override
    try:
        done = runner(["rustc", "--print", "target-libdir"], capture_output=True, text=True, check=False)
    except OSError as error:
        raise ReportError(f"cannot run rustc to find {name}: {error}") from None
    if done.returncode != 0 or not (done.stdout or "").strip():
        raise ReportError(f"rustc --print target-libdir exited {done.returncode}: cannot find {name}")
    suffix = ".exe" if sys.platform == "win32" else ""
    return os.fspath(Path(done.stdout.strip()).parent / "bin" / f"{name}{suffix}")


# llvm-cov's error for an object with no coverage map: a program with no instrumented code (a test target compiled
# empty on this OS, `#![cfg(target_os = "macos")]`), which has nothing to join.
NO_COVERAGE = "no coverage data found"


def _llvm(runner: Runner, command: list[str]) -> subprocess.CompletedProcess[str]:
    """Runs one llvm tool; refuses with its exit and the tail of its error output."""
    try:
        done = runner(command, capture_output=True, text=True, check=False)
    except OSError as error:
        raise ReportError(f"cannot run {Path(command[0]).name}: {error}") from None
    if done.returncode != 0:
        tail = "\n".join(_ANSI.sub("", done.stderr or "").strip().splitlines()[-5:])
        raise ReportError(f"{Path(command[0]).name} {' '.join(command[1:2])} exited {done.returncode}: {tail}")
    return done


# llvm-cov's region kind of plain code (`CounterMappingRegion::CodeRegion`), the only kind its region % counts.
CODE_REGION = 0


@dataclass
class FileJoin:
    """One source file's counts joined over the programs: the highest any program gave."""

    lines: dict[int, int] = field(default_factory=dict)
    # lcov's function records by name: (first line, count)
    functions: dict[str, tuple[int, int]] = field(default_factory=dict)
    # code regions by their span (start line and column, end line and column)
    regions: dict[tuple[int, int, int, int], int] = field(default_factory=dict)
    # whether a function, by its start (line, column), ran in one of its instantiations: llvm-cov's function %
    starts: dict[tuple[int, int], bool] = field(default_factory=dict)

    def drop(self, covers: Callable[[int], bool]) -> None:
        """Leaves out the lines, and the functions and regions starting on a line, that `covers` holds."""
        self.lines = {number: count for number, count in self.lines.items() if not covers(number)}
        self.functions = {name: seen for name, seen in self.functions.items() if not covers(seen[0])}
        self.regions = {span: count for span, count in self.regions.items() if not covers(span[0])}
        self.starts = {start: ran for start, ran in self.starts.items() if not covers(start[0])}

    def summary(self) -> dict[str, dict[str, Any]]:
        def counts(values: Iterable[int]) -> dict[str, Any]:
            listed = list(values)
            covered = sum(1 for value in listed if value)
            return {"count": len(listed), "covered": covered, "percent": 100.0 * covered / len(listed) if listed else 0}

        return {
            "lines": counts(self.lines.values()),
            "regions": counts(self.regions.values()),
            "functions": counts(int(ran) for ran in self.starts.values()),
        }


def join_lcov(files: dict[str, FileJoin], text: str) -> None:
    """Joins one program's `llvm-cov export -format=lcov` into `files`."""
    current: FileJoin | None = None
    starts: dict[str, int] = {}
    for line in text.splitlines():
        if line.startswith("SF:"):
            current = files.setdefault(line[3:], FileJoin())
            starts = {}
        elif current is None:
            continue
        elif line.startswith("DA:"):
            number, count = (int(part) for part in line[3:].split(",")[:2])
            current.lines[number] = max(current.lines.get(number, 0), count)
        elif line.startswith("FN:"):
            number, name = line[3:].split(",", 1)
            starts[name] = int(number)
        elif line.startswith("FNDA:"):
            count, name = line[5:].split(",", 1)
            start, seen = current.functions.get(name, (starts.get(name, 0), 0))
            current.functions[name] = (start, max(seen, int(count)))
        elif line == "end_of_record":
            current = None


def join_regions(files: dict[str, FileJoin], export: Mapping[str, Any]) -> None:
    """Joins one program's `llvm-cov export -format=text` functions into `files`: their code regions and starts."""
    for data in export.get("data", []):
        for function in data.get("functions", []):
            names = function.get("filenames") or []
            regions = function.get("regions") or []
            if not names or not regions:
                continue
            for start_line, start_col, end_line, end_col, count, file_id, *rest in regions:
                if not rest or rest[-1] != CODE_REGION or file_id >= len(names):
                    continue
                join = files.setdefault(names[file_id], FileJoin())
                span = (start_line, start_col, end_line, end_col)
                join.regions[span] = max(join.regions.get(span, 0), count)
            head = regions[0]
            join = files.setdefault(names[head[5]] if head[5] < len(names) else names[0], FileJoin())
            start = (head[0], head[1])
            join.starts[start] = join.starts.get(start, False) or int(function.get("count", 0)) > 0


def render_lcov(files: Mapping[str, FileJoin]) -> str:
    out: list[str] = []
    for name, join in sorted(files.items()):
        if not join.lines and not join.functions:
            continue
        functions = sorted(join.functions.items(), key=lambda item: (item[1][0], item[0]))
        out.append(f"SF:{name}")
        out += [f"FN:{start},{function}" for function, (start, _count) in functions]
        out += [f"FNDA:{count},{function}" for function, (_start, count) in functions]
        out += [f"FNF:{len(functions)}", f"FNH:{sum(1 for _name, (_start, count) in functions if count)}"]
        out += [f"DA:{number},{count}" for number, count in sorted(join.lines.items())]
        out += [f"LF:{len(join.lines)}", f"LH:{sum(1 for count in join.lines.values() if count)}", "end_of_record"]
    return "".join(f"{line}\n" for line in out)


def join_objects(
    profiles: Sequence[Path],
    objects: Sequence[Path],
    work: Path,
    runner: Runner = subprocess.run,
    exclude: str = EXCLUDE,
    regions: bool = True,
) -> tuple[dict[str, FileJoin], list[str]]:
    """The merged profiles (merged into `work`) read against each of `objects` on its own and joined per source file,
    the files `exclude` matches left out; with llvm-cov's warnings. Without `regions`, lines and functions alone (the
    lcov export, not the text one)."""
    profdata = work / "merged.profdata"
    inputs = work / "profiles.txt"
    inputs.write_text("".join(f"{os.fspath(path)}\n" for path in profiles), encoding="utf-8")
    profdata_tool = llvm_tool("llvm-profdata", runner)
    _llvm(runner, [profdata_tool, "merge", "-sparse", f"--input-files={inputs}", "-o", os.fspath(profdata)])
    llvm_cov = llvm_tool("llvm-cov", runner)
    files: dict[str, FileJoin] = {}
    warnings: list[str] = []
    for obj in objects:
        common = [f"-instr-profile={profdata}", f"-ignore-filename-regex={exclude}", os.fspath(obj)]
        try:
            lcov = _llvm(runner, [llvm_cov, "export", "-format=lcov", *common])
        except ReportError as error:
            if NO_COVERAGE in str(error):
                continue
            raise
        text = _llvm(runner, [llvm_cov, "export", "-format=text", *common]) if regions else None
        for done in (lcov,) if text is None else (lcov, text):
            notes = _ANSI.sub("", done.stderr or "").splitlines()
            warnings += [f"{obj.name}: {note.strip()}" for note in notes if MISMATCH.search(note)]
        join_lcov(files, lcov.stdout or "")
        if text is None:
            continue
        try:
            join_regions(files, json.loads(text.stdout or "{}"))
        except json.JSONDecodeError as error:
            raise ReportError(f"unreadable llvm-cov export of {obj.name}: {error}") from None
    # llvm-cov's -ignore-filename-regex leaves out an excluded file's lines, not the regions its functions list
    excluded = re.compile(exclude)
    return {name: join for name, join in files.items() if not excluded.search(name)}, warnings


def export_of(files: Mapping[str, FileJoin]) -> dict[str, Any]:
    """An `llvm-cov export` shaped summary of `files`: `files` with `filename` and `summary`."""
    listed = [{"filename": name, "summary": join.summary()} for name, join in sorted(files.items())]
    return {"data": [{"files": listed}]}


def joined_export(
    profiles: Sequence[Path], objects: Sequence[Path], out: Path, runner: Runner = subprocess.run
) -> tuple[dict[str, Any], str, list[str]]:
    """The merged profiles read against each of `objects` on its own and joined: an `llvm-cov export` shaped summary
    (`export_of`), the joined lcov and llvm-cov's warnings."""
    files, warnings = join_objects(profiles, objects, out, runner)
    return export_of(files), render_lcov(files), warnings


def report(
    profiles: Path, stage_bin: Path, release: Path, repo: Path, out: Path, runner: Runner = subprocess.run
) -> str:
    """Checks the inputs, writes the reports into `out` and returns the summary's markdown."""
    found = check_profiles(profiles)
    check_binaries(stage_bin, release)
    out.mkdir(parents=True, exist_ok=True)
    objects = [release / binary.name for binary in stage_binaries(stage_bin)]
    export, lcov, warnings = joined_export(found, objects, out, runner)
    (out / "summary.json").write_text(json.dumps(export), encoding="utf-8")
    (out / "lcov.info").write_text(lcov, encoding="utf-8")
    # the html reads every object at once, so its own warnings describe that first-record view, not the summary
    _cargo(runner, repo, "--html", "--output-dir", os.fspath(out))
    markdown = render(crate_rows(export, repo.resolve()), dict.fromkeys(warnings))
    (out / "summary.md").write_text(markdown, encoding="utf-8")
    return markdown


# The unit-test report's exclusions besides `EXCLUDE`: test files by their names (an integration test's `tests/`, a
# `tests.rs` or `*_tests.rs` module); test code elsewhere is found by its attributes (rust_test_code.py).
UNIT_EXCLUDE = EXCLUDE[:-1] + r"|[\\/]tests[\\/]|[\\/]tests\.rs$|_tests\.rs$)"
UNIT_INTRO = (
    "Line, region and function coverage of `cargo test` per workspace crate (test code, `vendor/` and the `-sys` "
    "crates excluded)."
)
UNIT_FILES_INTRO = (
    "Line, function and region coverage of `cargo test` per source file (test code, `vendor/` and the `-sys` crates "
    "excluded), and its uncovered lines (`lcov.info` has them by number)."
)


def built_programs(messages: str, repo: Path) -> list[Path]:
    """The programs of the workspace's crates a `cargo test --no-run --message-format=json` built, from its messages:
    the test binaries and the programs they start, not build scripts or a -sys crate's."""
    crates = repo.resolve() / "crates"
    found: dict[str, Path] = {}
    for line in messages.splitlines():
        if not line.startswith("{"):
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        executable = message.get("executable")
        if message.get("reason") != "compiler-artifact" or not executable:
            continue
        if "custom-build" in (message.get("target") or {}).get("kind", []):
            continue
        try:
            parts = Path(str(message.get("manifest_path", ""))).resolve().relative_to(crates).parts
        except ValueError:
            continue
        if parts and not parts[0].endswith("-sys"):
            found[executable] = Path(executable)
    return sorted(found.values())


def readable_profiles(profiles: Sequence[Path], runner: Runner = subprocess.run) -> tuple[list[Path], list[Path]]:
    """`profiles` split into those llvm-profdata reads and those it cannot: a test child killed mid-run (a timeout, a
    crash test) leaves a truncated profile that would fail the whole merge."""
    tool = llvm_tool("llvm-profdata", runner)
    kept: list[Path] = []
    dropped: list[Path] = []
    for profile in profiles:
        try:
            done = runner([tool, "show", os.fspath(profile)], capture_output=True, text=True, check=False)
        except OSError as error:
            raise ReportError(f"cannot run {Path(tool).name}: {error}") from None
        (kept if done.returncode == 0 else dropped).append(profile)
    return kept, dropped


def crate_sources(files: Iterable[str], repo: Path) -> list[Path]:
    """Every `.rs` file of the crates `files` belong to: where their test-only code is declared."""
    directories = set()
    for name in files:
        parts = Path(name).relative_to(repo).parts
        if len(parts) > 2 and parts[0] == "crates":
            directories.add(repo / "crates" / parts[1])
    return [path.resolve() for directory in sorted(directories) for path in sorted(directory.rglob("*.rs"))]


def strip_test_code(files: dict[str, FileJoin], test_code: TestCode) -> None:
    """Leaves the test-only code `test_code` names out of `files`: its files whole, its spans' lines, functions and
    regions in the others."""
    for name in list(files):
        path = Path(name).resolve()
        if test_code.whole(path):
            del files[name]
        elif path in test_code.spans:
            files[name].drop(test_code.spans[path].covers)


def render_files(files: Mapping[str, FileJoin], repo: Path) -> str:
    lines = [
        UNIT_FILES_INTRO,
        "",
        "| file | lines | functions | regions | uncovered lines |",
        "|---|---:|---:|---:|---:|",
    ]
    for name, join in sorted(files.items()):
        summary = join.summary()
        cells = [Counts.of(summary[metric]).cell() for metric in ("lines", "functions", "regions")]
        uncovered = summary["lines"]["count"] - summary["lines"]["covered"]
        lines.append(f"| {Path(name).relative_to(repo).as_posix()} | " + " | ".join(cells) + f" | {uncovered} |")
    return "\n".join(lines) + "\n"


def unit_report(
    messages: Path, profiles: Path, repo: Path, out: Path, crates: Sequence[str] = (), runner: Runner = subprocess.run
) -> str:
    """The unit-test report (`scripts/unit-coverage.sh`): this run's profiles in `profiles`, read against each program
    its build built (`messages`, the build's JSON messages) and joined, test code left out, only the files of
    `crates` (package names; every workspace crate when empty). Writes `out/lcov.info`, `out/summary.json`,
    `out/summary.md` (per crate) and `out/summary-files.md` (per file); returns the summary's markdown."""
    repo = repo.resolve()
    objects = built_programs(messages.read_text(encoding="utf-8"), repo)
    if not objects:
        raise ReportError(f"no program of the workspace's crates in the build's messages {messages}")
    kept, dropped = readable_profiles(check_profiles(profiles), runner)
    if not kept:
        raise ReportError(f"no readable profile in {profiles}: {len(dropped)} truncated")
    files, warnings = join_objects(kept, objects, profiles, runner, UNIT_EXCLUDE)
    names: dict[str, str] = {}
    files = {
        name: join
        for name, join in files.items()
        if (crate := crate_of(name, repo, names)) is not None and (not crates or crate in crates)
    }
    test_code = find_test_code(crate_sources(files, repo))
    strip_test_code(files, test_code)
    export = export_of(files)
    out.mkdir(parents=True, exist_ok=True)
    (out / "summary.json").write_text(json.dumps(export), encoding="utf-8")
    (out / "lcov.info").write_text(render_lcov(files), encoding="utf-8")
    (out / "summary-files.md").write_text(render_files(files, repo), encoding="utf-8")
    notes = [f"{len(dropped)} truncated profile(s) of killed test processes dropped"] if dropped else []
    notes += [
        f"unreadable source, its test code counted: {path.relative_to(repo).as_posix()}: {reason}"
        for path, reason in sorted(test_code.unread.items())
    ]
    rows = crate_rows(export, repo)
    markdown = render(rows, [*notes, *dict.fromkeys(warnings)], intro=UNIT_INTRO)
    (out / "summary.md").write_text(markdown, encoding="utf-8")
    return markdown


def unit_main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(prog="python -m agterm_conformance.code_coverage unit")
    for option in ("--messages", "--profiles", "--repo", "--out"):
        parser.add_argument(option, type=Path, required=True)
    parser.add_argument("--crate", action="append", default=[], help="a package to report (repeatable; default: all)")
    args = parser.parse_args(argv)
    try:
        markdown = unit_report(args.messages, args.profiles, args.repo, args.out, args.crate)
    except (ReportError, OSError) as error:
        print(f"code coverage: {error}", file=sys.stderr)
        return 2
    print(markdown, end="")
    return 0


# The e2e report (`code_coverage.py e2e`): one app on one platform.

# Each app's programs and modules per platform, relative to the repo.
APPS_FILE = Path("conformance/coverage/apps.toml")
E2E_TARGET = 90.0
_APP_KEYS = frozenset({"binaries", "modules", "exclude"})
_PROGRAM = re.compile(r"^[A-Za-z0-9_.+-]+$")
_DRIVE = re.compile(r"^[A-Za-z]:/")
# The word a marker's function is named with in the summary, by language.
_FUNCTION_WORD = {"rust": "fn", "kotlin": "fun", "swift": "func"}


@dataclass(frozen=True)
class AppBuild:
    """One app's code on one platform (apps.toml): the programs whose coverage maps count, the modules its non-Rust
    files group into (name and repo-relative path prefix, the longest prefix first), and the patterns of the files it
    leaves out."""

    binaries: tuple[str, ...]
    modules: tuple[tuple[str, str], ...] = ()
    exclude: tuple[str, ...] = ()


@dataclass(frozen=True)
class RustInput:
    """An instrumented run's profiles, the build whose programs' maps they are read against, and the stage the run
    ran (checked to be that build's programs byte for byte)."""

    profiles: Path
    release: Path
    stage_bin: Path | None = None


def load_apps(path: Path) -> dict[tuple[str, str], AppBuild]:
    """apps.toml: a `[app.platform]` table per app and platform with `binaries`, and optionally `modules` and
    `exclude`."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as error:
        raise ReportError(f"cannot read the apps file {path}: {error}") from None
    try:
        data = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise ReportError(f"{path}: not TOML: {error}") from None
    apps: dict[tuple[str, str], AppBuild] = {}
    for app, platforms in data.items():
        if not isinstance(platforms, dict):
            raise ReportError(f"{path}: [{app}] is not a table of platforms")
        for platform, entry in platforms.items():
            where = f"{path}: [{app}.{platform}]"
            if not isinstance(entry, dict):
                raise ReportError(f"{where}: not a table")
            unknown = sorted(set(entry) - _APP_KEYS)
            if unknown:
                raise ReportError(f"{where}: unknown key {', '.join(unknown)}")
            binaries = entry.get("binaries", [])
            if not isinstance(binaries, list) or not all(
                isinstance(name, str) and _PROGRAM.match(name) for name in binaries
            ):
                raise ReportError(f"{where}: binaries: not a list of names")
            modules = entry.get("modules", {})
            if not isinstance(modules, dict) or not all(
                isinstance(prefix, str) and prefix for prefix in modules.values()
            ):
                raise ReportError(f"{where}: modules: not a table of path prefixes")
            exclude = entry.get("exclude", [])
            if not isinstance(exclude, list) or not all(isinstance(pattern, str) for pattern in exclude):
                raise ReportError(f"{where}: exclude: not a list of regular expressions")
            for pattern in exclude:
                try:
                    re.compile(pattern)
                except re.error as error:
                    raise ReportError(f"{where}: exclude: not a regular expression: {pattern!r}: {error}") from None
            ordered = tuple(sorted(modules.items(), key=lambda item: (-len(item[1]), item[0])))
            apps[app, platform] = AppBuild(tuple(binaries), ordered, tuple(exclude))
    return apps


def app_objects(app: str, build: AppBuild, release: Path) -> list[Path]:
    """The app's programs in the build directory `release` (on Windows with `.exe`)."""
    objects = []
    for name in build.binaries:
        found = next((path for path in (release / name, release / f"{name}.exe") if path.is_file()), None)
        if found is None:
            raise ReportError(f"the {app} app's program {name} is not in {release}")
        objects.append(found)
    return objects


def check_app_stage(stage_bin: Path, objects: Sequence[Path]) -> None:
    """Each of the app's programs staged must be the build's own, whose coverage map the report reads."""
    for built in objects:
        staged = stage_bin / built.name
        if not staged.is_file() or not filecmp.cmp(staged, built, shallow=False):
            raise ReportError(f"binary mismatch: {staged} is not {built}, whose coverage map the report reads")


def repo_relative(name: str, repo: Path) -> str | None:
    """A coverage file name as a repo-relative POSIX path, None outside the repo (the toolchain's library, the
    registry's crates). Relative names (the e2e report's own lcov) are taken as relative to the repo."""
    posix = name.replace("\\", "/")
    if _DRIVE.match(posix) or posix.startswith("/"):
        try:
            return Path(os.path.normpath(name)).relative_to(repo).as_posix()
        except ValueError:
            return None
    return Path(os.path.normpath(posix)).as_posix()


def group_of(path: str, build: AppBuild, repo: Path, names: dict[str, str]) -> str | None:
    """The row a repo-relative file counts in: its module (apps.toml), else its workspace crate; None when the app
    leaves it out (`EXCLUDE` or the app's `exclude`, searched in the path with a leading `/`) or it is in neither."""
    rooted = "/" + path
    if re.search(EXCLUDE, rooted) or any(re.search(pattern, rooted) for pattern in build.exclude):
        return None
    for module, prefix in build.modules:
        if path.startswith(prefix):
            return module
    return crate_of(os.fspath(repo / path), repo, names)


def _absorb(into: FileJoin, other: FileJoin) -> None:
    """Joins `other` into `into`: a line or function counts as run when either ran it."""
    for number, count in other.lines.items():
        into.lines[number] = max(into.lines.get(number, 0), count)
    for name, (start, count) in other.functions.items():
        seen = into.functions.get(name, (start, 0))
        into.functions[name] = (seen[0], max(seen[1], count))


@dataclass
class E2ERow:
    count: int = 0
    covered: int = 0
    excluded: int = 0

    def add(self, other: E2ERow) -> None:
        self.count += other.count
        self.covered += other.covered
        self.excluded += other.excluded

    @property
    def lines(self) -> Counts:
        return Counts(self.count, self.covered)

    @property
    def share(self) -> Counts:
        """The excluded lines out of the measured and excluded ones."""
        return Counts(self.count + self.excluded, self.excluded)


@dataclass(frozen=True)
class Listed:
    """A marker as the summary lists it: the measured lines it excluded, and whether this build has its code."""

    marker: Marker
    lines: int
    in_build: bool


def find_app_markers(files: Iterable[str], repo: Path) -> list[Marker]:
    """The markers of the measured sources (`files`, repo-relative); every source and marker that cannot be read is
    named in one ReportError."""
    markers: list[Marker] = []
    errors: list[str] = []
    for path in sorted(files):
        language = language_of(path)
        if language is None:
            continue
        try:
            source = (repo / path).read_text(encoding="utf-8")
        except FileNotFoundError:
            errors.append(f"{path}: a measured source is not in the checkout {repo}")
            continue
        except (OSError, UnicodeDecodeError) as error:
            errors.append(f"{path}: cannot read the source: {error}")
            continue
        try:
            markers += find_markers(source, language, path)
        except MarkerError as error:
            errors.append(str(error))
    if errors:
        raise ReportError("\n".join(errors))
    return markers


def apply_markers(
    files: Mapping[str, tuple[str, FileJoin]], markers: Sequence[Marker]
) -> tuple[dict[str, FileJoin], dict[str, E2ERow], list[Listed]]:
    """Leaves the marked code out of `files` (repo-relative path → its row and join): the measured files, the rows
    and each marker as listed."""
    by_file: dict[str, list[Marker]] = {}
    for marker in markers:
        by_file.setdefault(marker.path, []).append(marker)
    measured: dict[str, FileJoin] = {}
    rows: dict[str, E2ERow] = {}
    listed: list[Listed] = []
    for path, (group, join) in sorted(files.items()):
        marks = by_file.get(path, [])
        for marker in marks:
            lines = sum(1 for number in join.lines if marker.covers(number))
            records = any(marker.covers(start) for start, _count in join.functions.values())
            listed.append(Listed(marker, lines, lines > 0 or records))

        def covers(number: int, marks: list[Marker] = marks) -> bool:
            return any(marker.covers(number) for marker in marks)

        kept = FileJoin(dict(join.lines), dict(join.functions))
        kept.drop(covers)
        row = rows.setdefault(group, E2ERow())
        row.add(E2ERow(len(kept.lines), sum(1 for count in kept.lines.values() if count),
                       len(join.lines) - len(kept.lines)))
        measured[path] = kept
    listed.sort(key=lambda item: (item.marker.path, item.marker.line))
    return measured, dict(sorted(rows.items())), listed


def _distance(percent: float | None) -> str:
    if percent is None:
        return "no line measured"
    if percent + 1e-9 >= E2E_TARGET:
        return f"at or above {E2E_TARGET:.0f}%"
    return f"{E2E_TARGET - percent:.2f} pt to {E2E_TARGET:.0f}%"


def _escape(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def render_e2e(
    app: str, platform: str, build: AppBuild, rows: Mapping[str, E2ERow], listed: Sequence[Listed],
    notes: Iterable[str] = (),
) -> str:
    total = E2ERow()
    for row in rows.values():
        total.add(row)
    programs = ", ".join(build.binaries) or "none"
    modules = f" and its modules {', '.join(name for name, _prefix in build.modules)}" if build.modules else ""
    percent = total.lines.percent
    shown = "n/a" if percent is None else f"{percent:.2f}% of lines ({total.covered}/{total.count})"
    share = total.share
    lines = [
        f"E2E line coverage of the `{app}` app on `{platform}`: the lines of its programs {programs}{modules} that its "
        "e2e tests ran, driving them from outside. Left out: `vendor/`, the `-sys` crates, the files apps.toml leaves "
        "out for the app, and the code the `// e2e-coverage: off` markers name (listed below).",
        "",
        f"**Total: {shown}, {_distance(percent)}. Excluded by markers: {share.percent or 0.0:.2f}% of lines "
        f"({share.covered}/{share.count}).**",
        "",
        "| crate | lines | excluded |",
        "|---|---:|---:|",
    ]
    for name, row in rows.items():
        lines.append(f"| {name} | {row.lines.cell()} | {row.share.cell()} |")
    lines.append(f"| total | {total.lines.cell()} | {total.share.cell()} |")
    lines += ["", "### Exclusion markers", ""]
    if not listed:
        lines.append("None.")
    else:
        lines += ["| file | line | excludes | lines | reason |", "|---|---:|---|---:|---|"]
        for item in listed:
            marker = item.marker
            if marker.scope == FILE:
                what = "the file"
            else:
                word = _FUNCTION_WORD.get(language_of(marker.path) or "", "function")
                what = f"{word} `{marker.name}`" + ("" if item.in_build else " (not in this build)")
            lines.append(f"| {marker.path} | {marker.line} | {what} | {item.lines} | {_escape(marker.reason)} |")
    lines += [f"\nllvm-cov: {note}" for note in notes]
    return "\n".join(lines) + "\n"


def e2e_report(
    app: str,
    platform: str,
    repo: Path,
    out: Path,
    rust: Sequence[RustInput] = (),
    lcov: Sequence[Path] = (),
    apps_file: Path | None = None,
    runner: Runner = subprocess.run,
) -> str:
    """The e2e report of `app` on `platform`: the profiles of `rust` read against the app's own programs, joined with
    the lcov files `lcov` (an adapter's output, or other jobs' e2e `lcov.info`), the app's files alone, its markers'
    code left out. Writes `out/lcov.info` (the app's data before the markers, the input of a later merge),
    `out/lcov-measured.info` (after them), `out/summary.json` and `out/summary.md`; returns the summary's markdown."""
    repo = repo.resolve()
    if not rust and not lcov:
        raise ReportError("no input: give --profiles with --release-dir, or --lcov")
    apps_file = repo / APPS_FILE if apps_file is None else apps_file
    builds = load_apps(apps_file)
    if (app, platform) not in builds:
        raise ReportError(f"no app `{app}` on `{platform}` in {apps_file}")
    build = builds[app, platform]
    texts = []
    for path in lcov:
        try:
            texts.append(path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError) as error:
            raise ReportError(f"cannot read the lcov file {path}: {error}") from None
    jobs = []
    for item in rust:
        found = check_profiles(item.profiles)
        objects = app_objects(app, build, item.release)
        if item.stage_bin is not None:
            check_app_stage(item.stage_bin, objects)
        jobs.append((found, objects))
    raw: dict[str, FileJoin] = {}
    warnings: list[str] = []
    with tempfile.TemporaryDirectory() as work:
        for number, (found, objects) in enumerate(jobs):
            directory = Path(work) / str(number)
            directory.mkdir()
            joined, notes = join_objects(found, objects, directory, runner, regions=False)
            for name, join in joined.items():
                _absorb(raw.setdefault(name, FileJoin()), join)
            warnings += notes
    for text in texts:
        join_lcov(raw, text)
    names: dict[str, str] = {}
    files: dict[str, tuple[str, FileJoin]] = {}
    for name, join in raw.items():
        path = repo_relative(name, repo)
        group = None if path is None else group_of(path, build, repo, names)
        if path is None or group is None or not (join.lines or join.functions):
            continue
        if path in files:
            _absorb(files[path][1], join)
        else:
            files[path] = (group, FileJoin(dict(join.lines), dict(join.functions)))
    if not files:
        raise ReportError(f"no line of the {app} app's code in the inputs")
    markers = find_app_markers(files, repo)
    measured, rows, listed = apply_markers(files, markers)
    markdown = render_e2e(app, platform, build, rows, listed, dict.fromkeys(warnings))
    total = E2ERow()
    for row in rows.values():
        total.add(row)
    summary = {
        "app": app,
        "platform": platform,
        "binaries": list(build.binaries),
        "total": {"lines": {"count": total.count, "covered": total.covered}},
        "excluded": {"lines": total.excluded, "share": total.share.percent or 0.0},
        "rows": {
            name: {"lines": {"count": row.count, "covered": row.covered}, "excluded": row.excluded}
            for name, row in rows.items()
        },
        "markers": [
            {"path": item.marker.path, "line": item.marker.line, "scope": item.marker.scope, "name": item.marker.name,
             "reason": item.marker.reason, "lines": item.lines}
            for item in listed
        ],
        "data": export_of(measured)["data"],
    }
    out.mkdir(parents=True, exist_ok=True)
    (out / "lcov.info").write_text(render_lcov({path: join for path, (_group, join) in files.items()}),
                                   encoding="utf-8")
    (out / "lcov-measured.info").write_text(render_lcov(measured), encoding="utf-8")
    (out / "summary.json").write_text(json.dumps(summary), encoding="utf-8")
    (out / "summary.md").write_text(markdown, encoding="utf-8")
    return markdown


def e2e_main(argv: Sequence[str]) -> int:
    parser = argparse.ArgumentParser(prog="python -m agterm_conformance.code_coverage e2e")
    parser.add_argument("--app", required=True, help="the app (a table of apps.toml)")
    parser.add_argument("--platform", required=True, help="its platform (a subtable of the app's)")
    for option in ("--repo", "--out"):
        parser.add_argument(option, type=Path, required=True)
    parser.add_argument("--apps", type=Path, help=f"the apps file (default: REPO/{APPS_FILE.as_posix()})")
    parser.add_argument("--profiles", type=Path, help="an instrumented run's profile directory")
    parser.add_argument("--release-dir", type=Path, help="the instrumented build's programs (with --profiles)")
    parser.add_argument("--stage-bin", type=Path, help="the stage the run ran, checked to be that build's")
    parser.add_argument("--lcov", type=Path, action="append", default=[],
                        help="an lcov file to join (repeatable: a line run in any counts as run)")
    args = parser.parse_args(argv)
    rust = []
    if args.profiles is not None or args.release_dir is not None or args.stage_bin is not None:
        if args.profiles is None or args.release_dir is None:
            print("code coverage: --profiles needs --release-dir (and --release-dir --profiles)", file=sys.stderr)
            return 2
        rust.append(RustInput(args.profiles, args.release_dir, args.stage_bin))
    try:
        markdown = e2e_report(args.app, args.platform, args.repo, args.out, rust, args.lcov, args.apps)
    except (ReportError, OSError) as error:
        print(f"code coverage: {error}", file=sys.stderr)
        return 2
    print(markdown, end="")
    return 0


def main(argv: Sequence[str]) -> int:
    if argv[:1] == ["e2e"]:
        return e2e_main(argv[1:])
    if argv[:1] == ["unit"]:
        return unit_main(argv[1:])
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
