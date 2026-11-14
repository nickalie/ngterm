#!/usr/bin/env python3
"""Join the reports of a run's shards into the report of the whole run, and judge the ratchet on it.

    merge_shards.py --out DIR [--list RATCHET] [--title NAME] [--coverage APP PLATFORM] SHARD_DIR...

Each SHARD_DIR is one shard's report directory (`run-linux.sh --shard I/N --out SHARD_DIR`, or the Windows runner's).
The shards must be one split, whole: the same N, every index of it once, the same collection size, file-to-shard
table and mode in each header (agterm_conformance/shards.py), the same `--must-pass` capabilities, and no test id in
two of them. A shard missing, come without a nodeid report (its run wrote none) or from another split exits 2: a test
no shard ran never reads as a holding run.

Into DIR it writes what a whole run's runner writes:

- `nodeids.jsonl`: the shards' test records under one run header (the shards' own mode, `"shards": N`), their
  collection errors once each, their `--must-pass` problems, and the problems only the whole run can have: a
  capability no shard's test requires, or none passed for (from the shards' counts).
- `junit.xml`: the shards' test suites, each named after its shard; `pytest.log`: their logs, one after another.
- `traces/` and `instances/`: the shards' joined. Everything else of a shard (its own `report.md`, `coverage/`,
  `lost-profiles.txt`) goes to `shards/I/`.
- with `--coverage APP PLATFORM`, `coverage/`: the app's e2e report joined from the shards' `coverage/lcov.info`
  (agterm_conformance/code_coverage.py e2e: a line run in any shard counts as run), which `check_coverage.py` then
  holds against the floor.
- `report.md`: the counts, each shard's share, the failures, the ratchet verdict, the coverage and the ui ids a
  passing test exercised.

With `--list` it judges the merged report with `check_ratchet.py` and exits with its status (0 holds, 1 broken);
2 for an unusable shard set, or when `--coverage` yields no report.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import xml.etree.ElementTree as ElementTree
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

TOOLS = Path(__file__).resolve().parent
CONFORMANCE = TOOLS.parent
sys.path.insert(0, str(CONFORMANCE))

from agterm_conformance import shards as split  # noqa: E402 - after the path above

NODEIDS, JUNIT, LOG, REPORT = "nodeids.jsonl", "junit.xml", "pytest.log", "report.md"
# a shard's directories whose entries are named after their tests: joined as they are
JOINED = ("traces", "instances")
HEADER_KEYS = ("count", "collected", "split")
PASSED = frozenset({"passed", "xpassed"})
FAILED = frozenset({"failed", "error"})


class MergeError(Exception):
    """A shard set that is not one whole split; the message names the shard and what is wrong with it."""


@dataclass
class ShardReport:
    directory: Path
    header: dict[str, Any]
    tests: list[dict[str, Any]] = field(default_factory=list)
    collection_errors: list[dict[str, Any]] = field(default_factory=list)
    problems: list[str] = field(default_factory=list)
    capabilities: dict[str, dict[str, int]] = field(default_factory=dict)

    @property
    def index(self) -> int:
        return int(self.header["shard"]["index"])


def read_shard(directory: Path) -> ShardReport:
    path = directory / NODEIDS
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeDecodeError) as error:
        raise MergeError(f"{directory}: no nodeid report (the shard's run wrote none): {error}") from None
    try:
        records = [json.loads(line) for line in lines if line]
    except json.JSONDecodeError as error:
        raise MergeError(f"{path}: not JSON lines: {error}") from None
    if not records or not isinstance(records[0], dict) or records[0].get("kind") != "run":
        raise MergeError(f"{path}: no run header")
    header = records[0]
    shard = header.get("shard")
    if not isinstance(shard, dict) or not all(isinstance(shard.get(key), int) for key in ("index", "count", "collected")):
        raise MergeError(f"{path}: not a shard's report (its run header has no `shard`)")
    report = ShardReport(directory, header)
    for record in records[1:]:
        kind = record.get("kind") if isinstance(record, dict) else None
        if kind == "test":
            report.tests.append(record)
        elif kind == "collection-error":
            report.collection_errors.append(record)
        elif kind == "must-pass":
            report.problems.append(str(record.get("problem")))
        elif kind == "must-pass-capability":
            report.capabilities[str(record.get("capability"))] = {
                "required": int(record.get("required", 0)), "passed": int(record.get("passed", 0))
            }
        else:
            raise MergeError(f"{path}: unknown record kind {kind!r}")
    return report


def check_split(reports: list[ShardReport]) -> None:
    """Refuses shards that are not every part of one split, once each."""
    first = reports[0]
    count = first.header["shard"]["count"]
    for report in reports[1:]:
        for key in HEADER_KEYS:
            if report.header["shard"].get(key) != first.header["shard"].get(key):
                raise MergeError(
                    f"{report.directory} and {first.directory} are not shards of one split: their `{key}` differ "
                    f"({report.header['shard'].get(key)!r}, {first.header['shard'].get(key)!r})"
                )
        for key in ("mode", "partial"):
            if report.header.get(key) != first.header.get(key):
                raise MergeError(f"{report.directory} and {first.directory} ran in different modes (`{key}`)")
        if sorted(report.capabilities) != sorted(first.capabilities):
            raise MergeError(f"{report.directory} and {first.directory} ran with different --must-pass capabilities")
    by_index: dict[int, ShardReport] = {}
    for report in reports:
        if report.index in by_index:
            raise MergeError(f"{report.directory} and {by_index[report.index].directory} are both shard {report.index}")
        by_index[report.index] = report
    missing = [index for index in range(1, count + 1) if index not in by_index]
    if missing or len(by_index) != count:
        raise MergeError(f"shard {', '.join(f'{index}/{count}' for index in missing)}: no report")
    seen: dict[str, int] = {}
    for report in reports:
        for test in report.tests:
            nodeid = test.get("nodeid")
            if nodeid in seen:
                raise MergeError(f"{nodeid} is in shard {seen[nodeid]} and in shard {report.index}")
            seen[nodeid] = report.index


def merged_records(reports: list[ShardReport]) -> list[dict[str, Any]]:
    first = reports[0]
    run = {key: value for key, value in first.header.items() if key != "shard"}
    run.update(shards=first.header["shard"]["count"], collected=first.header["shard"]["collected"])
    records: list[dict[str, Any]] = [run]
    for report in reports:
        records += report.tests
    errors: dict[tuple[Any, Any], dict[str, Any]] = {}
    for report in reports:
        for error in report.collection_errors:
            # every shard collects the whole suite, so each meets the same errors
            errors.setdefault((error.get("nodeid"), error.get("message")), error)
    records += errors.values()
    counts = {
        capability: {key: sum(report.capabilities[capability][key] for report in reports) for key in ("required", "passed")}
        for capability in first.capabilities
    }
    problems = split.capability_problems(counts)
    for report in reports:
        problems += [problem for problem in report.problems if problem not in problems]
    records += [{"kind": "must-pass", "problem": problem} for problem in problems]
    return records


def merge_junit(reports: list[ShardReport], out: Path) -> None:
    root = ElementTree.Element("testsuites")
    for report in reports:
        path = report.directory / JUNIT
        if not path.is_file():
            continue
        try:
            parsed = ElementTree.parse(path).getroot()
        except ElementTree.ParseError as error:
            raise MergeError(f"{path}: not XML: {error}") from None
        for suite in [parsed] if parsed.tag == "testsuite" else parsed.iter("testsuite"):
            suite.set("name", f"{suite.get('name', 'pytest')} shard {report.index}/{report.header['shard']['count']}")
            root.append(suite)
    ElementTree.ElementTree(root).write(out, encoding="utf-8", xml_declaration=True)


def copy_files(reports: list[ShardReport], out: Path) -> None:
    count = reports[0].header["shard"]["count"]
    with open(out / LOG, "w", encoding="utf-8") as log:
        for report in reports:
            log.write(f"===== shard {report.index}/{count} =====\n")
            path = report.directory / LOG
            if path.is_file():
                log.write(path.read_text(encoding="utf-8", errors="replace").rstrip("\n") + "\n")
    for report in reports:
        for entry in sorted(report.directory.iterdir()):
            if entry.name in (NODEIDS, JUNIT, LOG):
                continue
            if entry.name in JOINED and entry.is_dir():
                shutil.copytree(entry, out / entry.name, dirs_exist_ok=True)
                continue
            target = out / "shards" / str(report.index) / entry.name
            target.parent.mkdir(parents=True, exist_ok=True)
            if entry.is_dir():
                shutil.copytree(entry, target)
            else:
                shutil.copy2(entry, target)


def counts_line(tests: list[dict[str, Any]]) -> str:
    """The plugin's summary line (`plugin.summary_line`) from the records, without its reruns."""
    outcomes = [test.get("outcome") for test in tests]
    passed = sum(1 for outcome in outcomes if outcome in PASSED)
    failed = sum(1 for outcome in outcomes if outcome in FAILED)
    xfailed = outcomes.count("xfailed")
    skips = {"unsupported": 0, "not-yet-at-level": 0, "other": 0}
    for test in tests:
        if test.get("outcome") == "skipped":
            skips[test.get("skip") if test.get("skip") in skips else "other"] += 1
    parts = [f"{passed} pass", f"{failed} fail", *(f"{number} skip({category})" for category, number in skips.items())]
    if xfailed:
        parts.append(f"{xfailed} xfailed")
    if not_run := outcomes.count("not-run"):
        parts.append(f"{not_run} not run")
    owed = passed + failed + xfailed + skips["not-yet-at-level"]
    parts.append(f"parity {100 * passed / owed:.1f}%" if owed else "parity n/a")
    return ", ".join(parts)


def shard_line(report: ShardReport, prefix: str) -> str | None:
    """A line of the shard's own report.md (`- build: ...`), None without one."""
    path = report.directory / REPORT
    if not path.is_file():
        return None
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return next((line for line in lines if line.startswith(prefix)), None)


def failure_summary(report: ShardReport) -> list[str]:
    """pytest's short summary of the shard's failures, as the runner's report shows it."""
    path = report.directory / LOG
    if not path.is_file():
        return []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    start = next((number for number, line in enumerate(lines) if "short test summary info" in line), None)
    return [] if start is None else lines[start + 1:]


def join_coverage(reports: list[ShardReport], app: str, platform: str, repo: Path, out: Path) -> str | None:
    """The app's e2e report joined from the shards' lcov files into `out/coverage`; the reason when there is none."""
    lcovs = [report.directory / "coverage" / "lcov.info" for report in reports]
    missing = [str(path) for path in lcovs if not path.is_file()]
    if missing:
        return f"no coverage report from a shard: {', '.join(missing)}"
    command = [sys.executable, "-m", "agterm_conformance.code_coverage", "e2e", "--app", app, "--platform", platform,
               "--repo", str(repo), "--out", str(out / "coverage")]
    for path in lcovs:
        command += ["--lcov", str(path)]
    done = subprocess.run(command, cwd=CONFORMANCE, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        return (done.stderr or done.stdout).strip().removeprefix("code coverage: ") or f"exit {done.returncode}"
    lost = [line for report in reports if (report.directory / "lost-profiles.txt").is_file()
            for line in (report.directory / "lost-profiles.txt").read_text(encoding="utf-8").splitlines()]
    if lost:
        (out / "lost-profiles.txt").write_text("".join(f"{line}\n" for line in lost), encoding="utf-8")
    return None


def catalog_line(out: Path, header: dict[str, Any]) -> str:
    command = [sys.executable, "-m", "agterm_conformance.coverage", str(out / NODEIDS)]
    # the header names the files on the shards' machine: this checkout's stand in where those are not here
    if not all(isinstance(header.get(key), str) and Path(header[key]).is_file() for key in ("catalog", "ui_ids")):
        command += ["--catalog", str(CONFORMANCE / "catalog.yaml"), "--ui-ids", str(CONFORMANCE / "ui-ids.yaml")]
    done = subprocess.run(command, cwd=CONFORMANCE, capture_output=True, text=True, check=False)
    return (done.stdout + done.stderr).strip()


def render_report(
    title: str, reports: list[ShardReport], records: list[dict[str, Any]], ratchet_log: str | None, status: int,
    coverage: tuple[str | None, Path] | None, catalog: str,
) -> str:
    count = reports[0].header["shard"]["count"]
    tests = [record for record in records if record.get("kind") == "test"]
    lines = [f"## {title} conformance", ""]
    for prefix in ("- build:", "- pytest args:"):
        if (line := shard_line(reports[0], prefix)) is not None:
            lines.append(line)
    lines += [
        f"- mode: {records[0].get('mode')}, {count} shards (by test file; each shard's own report is in `shards/`)",
        f"- tests: {len(tests)} of {records[0].get('collected')} collected",
        f"- exit status: {status}",
        "",
        "```",
        counts_line(tests),
        "```",
        "",
        "| shard | tests | counts | test seconds | suite time |",
        "|---|---:|---|---:|---|",
    ]
    for report in reports:
        seconds = sum(float(test.get("duration") or 0.0) for test in report.tests)
        suite = (shard_line(report, "- suite time:") or "- suite time: ?").removeprefix("- suite time: ")
        lines.append(f"| {report.index}/{count} | {len(report.tests)} | {counts_line(report.tests)} | {seconds:.0f} | {suite} |")
    failures = [(report, failure_summary(report)) for report in reports]
    if any(test.get("outcome") in FAILED for test in tests):
        lines += ["", "### failures", ""]
        for report, summary in failures:
            if any(test.get("outcome") in FAILED for test in report.tests):
                lines += [f"shard {report.index}/{count} (`shards/{report.index}/{REPORT}`, traces in `traces/`):", "", "```"]
                lines += summary or [f"{test.get('outcome')}: {test.get('nodeid')}" for test in report.tests
                                     if test.get("outcome") in FAILED]
                lines += ["```", ""]
    if ratchet_log is not None:
        lines += ["", "### ratchet", "", "```", ratchet_log, "```"]
    if coverage is not None:
        error, directory = coverage
        lines += ["", "### code coverage", ""]
        if error is None:
            lines += [(directory / "summary.md").read_text(encoding="utf-8").rstrip("\n")]
        else:
            lines.append(f"no report: {error}")
    lines += ["", "### catalog coverage", "", catalog]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("shards", nargs="+", type=Path, metavar="SHARD_DIR", help="a shard's report directory")
    parser.add_argument("--out", type=Path, required=True, help="the merged report directory (made empty first)")
    parser.add_argument("--list", type=Path, default=None, help="the ratchet list to judge the merged report by")
    parser.add_argument("--title", default="rust", help="the report's title (default: rust)")
    parser.add_argument("--coverage", nargs=2, metavar=("APP", "PLATFORM"), default=None,
                        help="join the shards' e2e coverage of APP on PLATFORM into OUT/coverage")
    parser.add_argument("--repo", type=Path, default=CONFORMANCE.parent, help="the checkout (default: this one)")
    args = parser.parse_args(argv)
    try:
        reports = sorted((read_shard(directory) for directory in args.shards), key=lambda report: report.index)
        check_split(reports)
        records = merged_records(reports)
        if args.out.exists():
            shutil.rmtree(args.out)
        args.out.mkdir(parents=True)
        (args.out / NODEIDS).write_text("".join(json.dumps(record) + "\n" for record in records), encoding="utf-8")
        merge_junit(reports, args.out / JUNIT)
        copy_files(reports, args.out)
    except MergeError as error:
        print(f"merge_shards: {error}", file=sys.stderr)
        return 2
    status = 0
    ratchet_log = None
    if args.list is not None:
        done = subprocess.run(
            [sys.executable, str(TOOLS / "check_ratchet.py"), "--report", str(args.out / NODEIDS), "--list", str(args.list)],
            capture_output=True, text=True, check=False,
        )
        ratchet_log = (done.stdout + done.stderr).rstrip("\n")
        status = done.returncode
        print(ratchet_log)
    coverage = None
    if args.coverage is not None:
        error = join_coverage(reports, args.coverage[0], args.coverage[1], args.repo, args.out)
        coverage = (error, args.out / "coverage")
        if error is not None:
            status = 2
            print(f"code coverage: {error}", file=sys.stderr)
    catalog = catalog_line(args.out, records[0])
    report = render_report(args.title, reports, records, ratchet_log, status, coverage, catalog)
    (args.out / REPORT).write_text(report, encoding="utf-8")
    print(f"report: {args.out / REPORT}")
    return status


if __name__ == "__main__":
    sys.exit(main())
