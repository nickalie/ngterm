#!/usr/bin/env python3
"""Line coverage of the iOS app's Unit test plan (mobile/ios/Unit.xctestplan), for scripts/build-ios.sh.

The plan's codeCoverage names the targets measured: the NgTerm app target and the local packages' library targets,
never the test bundles, the fake core (NgtermCoreFakes) or the generated UniFFI bindings (NgtermCoreBindings). The
measurement is Xcode's own llvm-cov (`xcrun llvm-cov`) over the profiles the run left in its derived data
(mobile/ios/build/DerivedData/Build/ProfileData/<simulator>/Coverage.profdata, merged across the simulators into
mobile/ios/build/Unit.profdata) and each target's own objects: a package target's <target>.o in the build products,
the app target's object files. `xccov view --report` over the result bundle can't be the measure: it attributes the
packages' code to framework binaries the build never makes (their libraries link statically) and leaves whole targets
out of its report from one build to the next (NgtermPlatform, NgtermTerminal on Xcode 27.2), and it reports one
simulator's run, not both. Each target's objects hold every function of its sources, run or not, so code no test
reaches counts at 0%.

- summarize: write mobile/ios/build/Unit.coverage.json and Unit.coverage.txt (and Unit.profdata), right after the
  Unit plan's run (the UI plan's run replaces the derived data's profiles). Each file counts toward the target its
  source lives in: mobile/ios/NgTerm/ is the NgTerm target, mobile/ios/Packages/<package>/Sources/<target>/ is
  <target>; what the build generates (under mobile/ios/build/, a resource bundle's accessor) is left out, and any
  other file in a target's objects fails the summary. The JSON is {"targets": [{"name", "covered", "executable",
  "percent", "files": [{"path", "covered", "executable", "percent"}]}]}, paths relative to the repository root,
  percents rounded down to two decimals; the text is a table of the targets, then every file, least covered first.
- check: fail when a target's line percent is below its floor in mobile/ios/coverage-floors.txt (`<target> <whole
  percent>` lines, `#` comments), when a measured target has no floor, or when a floor names a target the summary
  lacks. Floors only rise.
- uncovered FILE...: the lines of each source FILE (a path, absolute or relative to the current directory) that no
  test of the last Unit run executed, as line ranges, from `llvm-cov show` over Unit.profdata. It needs the
  derived data of that run (scripts/build-ios.sh test or coverage), and the Xcode that ran it (eval the exports of
  `scripts/setup-ios.sh --check` first, as build-ios.sh does).

Usage: scripts/ios-coverage.py summarize|check
       scripts/ios-coverage.py uncovered FILE...
       scripts/ios-coverage.py --self-test
"""

import json
import math
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IOS = ROOT / "mobile" / "ios"
BUILD = IOS / "build"
DERIVED_DATA = BUILD / "DerivedData"
PLAN = IOS / "Unit.xctestplan"
FLOORS = IOS / "coverage-floors.txt"
PROFDATA = BUILD / "Unit.profdata"
SUMMARY = BUILD / "Unit.coverage"
APP_TARGET = "NgTerm"
APP_CONTAINER = "container:NgTerm.xcodeproj"
CONFIGURATION = "Debug-iphonesimulator"

USAGE = """usage: scripts/ios-coverage.py summarize|check
       scripts/ios-coverage.py uncovered FILE...
       scripts/ios-coverage.py --self-test"""


class CoverageError(Exception):
    pass


def percent(covered, executable):
    """Line percent rounded down to two decimals; a target or file with no executable line is 100."""
    if executable == 0:
        return 100.0
    return math.floor(covered * 10000 / executable) / 100


def owning_target(path):
    """The target whose sources hold `path` (relative to the repository root), or None."""
    parts = Path(path).parts
    if parts[:3] == ("mobile", "ios", APP_TARGET) and len(parts) > 3:
        return APP_TARGET
    if parts[:3] == ("mobile", "ios", "Packages") and len(parts) > 6 and parts[4] == "Sources":
        return parts[5]
    return None


def is_generated(path):
    """Whether `path` (relative to the repository root) is a source the build generated."""
    return Path(path).parts[:3] == ("mobile", "ios", "build")


def relative_path(path, root):
    """`path` relative to `root`, the repository root (both resolved, as the compiler records them)."""
    try:
        return Path(path).resolve().relative_to(root).as_posix()
    except ValueError:
        return Path(path).as_posix()


def plan_targets(plan):
    """The targets the plan's codeCoverage names: [(name, is the app target)]."""
    coverage = plan.get("defaultOptions", {}).get("codeCoverage")
    if not isinstance(coverage, dict) or not coverage.get("targets"):
        raise CoverageError("Unit.xctestplan names no codeCoverage targets")
    return [(t["name"], t["containerPath"] == APP_CONTAINER) for t in coverage["targets"]]


def target_objects(name, is_app, derived_data):
    """The object files that hold target `name`'s code in the derived data's Debug simulator build."""
    if is_app:
        objects = derived_data / "Build/Intermediates.noindex" / f"{name}.build" / CONFIGURATION / f"{name}.build"
        found = sorted(objects.glob("Objects-normal/arm64/*.o"))
    else:
        found = [derived_data / "Build/Products" / CONFIGURATION / f"{name}.o"]
        found = [f for f in found if f.is_file()]
    if not found:
        raise CoverageError(f"no objects of {name} in {derived_data}: did the Unit plan's build run?")
    return found


def llvm_cov(*args):
    result = subprocess.run(["xcrun", "llvm-cov", *args], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise CoverageError(f"llvm-cov {args[0]} failed:\n{result.stderr.strip()}")
    return result.stdout


def object_args(objects):
    """llvm-cov's object arguments: the first object, then `-object` before each other."""
    args = [str(objects[0])]
    for extra in objects[1:]:
        args += ["-object", str(extra)]
    return args


def merge_profiles(derived_data, out):
    profiles = sorted((derived_data / "Build/ProfileData").glob("*/Coverage.profdata"))
    if not profiles:
        raise CoverageError(f"no Coverage.profdata under {derived_data}/Build/ProfileData: did the Unit plan run?")
    result = subprocess.run(
        ["xcrun", "llvm-profdata", "merge", "-sparse", *map(str, profiles), "-o", str(out)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise CoverageError(f"llvm-profdata merge failed:\n{result.stderr.strip()}")
    return len(profiles)


def target_summary(name, exported_files, root):
    """Target `name`'s summary from the files of `llvm-cov export -summary-only` over its objects."""
    files = []
    for entry in exported_files:
        path = relative_path(entry["filename"], root)
        if is_generated(path):
            continue
        owner = owning_target(path)
        if owner != name:
            raise CoverageError(f"{name}'s objects measure {path}, which isn't one of its sources")
        lines = entry["summary"]["lines"]
        covered, executable = lines["covered"], lines["count"]
        files.append(
            {"path": path, "covered": covered, "executable": executable, "percent": percent(covered, executable)}
        )
    files.sort(key=lambda f: f["path"])
    covered = sum(f["covered"] for f in files)
    executable = sum(f["executable"] for f in files)
    return {"name": name, "covered": covered, "executable": executable, "percent": percent(covered, executable),
            "files": files}


def summary_text(summary):
    """The summary as text: the targets, then the files least covered first."""
    lines = ["iOS Unit plan line coverage", "", f"{'target':<24} {'lines':>8} {'covered':>8} {'%':>7}"]
    for target in summary["targets"]:
        lines.append(f"{target['name']:<24} {target['executable']:>8} {target['covered']:>8} {target['percent']:>7.2f}")
    files = [f for target in summary["targets"] for f in target["files"]]
    files.sort(key=lambda f: (f["percent"], -f["executable"], f["path"]))
    lines += ["", f"{'%':>7} {'covered':>8} {'lines':>8}  file (least covered first)"]
    for f in files:
        lines.append(f"{f['percent']:>7.2f} {f['covered']:>8} {f['executable']:>8}  {f['path']}")
    return "\n".join(lines) + "\n"


def summarize():
    root = ROOT.resolve()
    targets = plan_targets(json.loads(PLAN.read_text()))
    count = merge_profiles(DERIVED_DATA, PROFDATA)
    summary = []
    for name, is_app in sorted(targets):
        objects = target_objects(name, is_app, DERIVED_DATA)
        exported = json.loads(llvm_cov("export", "-summary-only", "-instr-profile", str(PROFDATA),
                                       *object_args(objects)))
        summary.append(target_summary(name, exported["data"][0]["files"], root))
    summary = {"targets": summary}
    Path(f"{SUMMARY}.json").write_text(json.dumps(summary, indent=2) + "\n")
    text = summary_text(summary)
    Path(f"{SUMMARY}.txt").write_text(text)
    print("\n".join(text.splitlines()[: 3 + len(summary["targets"])]))
    print(f"ios-coverage: the profiles of {count} simulators, summarized in {SUMMARY}.txt and .json")


def read_floors(text):
    """The floors of a floor file: {target: whole percent}."""
    floors = {}
    for number, raw in enumerate(text.splitlines(), 1):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        match = re.fullmatch(r"(\S+)\s+(\d{1,3})", line)
        if not match or int(match[2]) > 100:
            raise CoverageError(f"floor file line {number}: expected `<target> <whole percent>`, got {raw!r}")
        if match[1] in floors:
            raise CoverageError(f"floor file line {number}: {match[1]} has a floor already")
        floors[match[1]] = int(match[2])
    return floors


def check_floors(summary, floors):
    """The report lines and the failures of a summary against its floors."""
    measured = {target["name"]: target["percent"] for target in summary["targets"]}
    lines = [f"{'target':<24} {'%':>7} {'floor':>6}"]
    failures = []
    for name in sorted(set(measured) | set(floors)):
        value, floor = measured.get(name), floors.get(name)
        shown_value = "-" if value is None else f"{value:.2f}"
        shown_floor = "-" if floor is None else str(floor)
        lines.append(f"{name:<24} {shown_value:>7} {shown_floor:>6}")
        if value is None:
            failures.append(f"{name} has a floor but no coverage: drop or rename its floor")
        elif floor is None:
            failures.append(f"{name} has no floor: add `{name} {math.floor(value)}`")
        elif value < floor:
            failures.append(f"{name} covers {value:.2f}% of its lines, below its floor of {floor}%")
    return lines, failures


def check(summary_path, floors_path):
    try:
        summary = json.loads(Path(summary_path).read_text())
        floors = read_floors(Path(floors_path).read_text())
    except OSError as error:
        raise CoverageError(str(error)) from None
    lines, failures = check_floors(summary, floors)
    print("\n".join(lines))
    if failures:
        raise CoverageError(f"coverage below the floors ({floors_path}):\n  " + "\n  ".join(failures))
    print("ios-coverage: every target is at or above its floor")


def unexecuted_ranges(show_text):
    """The unexecuted lines of `llvm-cov show` output (`  12|      0|code`), as ranges ("12", "40-44")."""
    lines = []
    for raw in show_text.splitlines():
        match = re.match(r"\s*(\d+)\|\s*(\S*)\|", raw)
        if match and match[2] == "0":
            lines.append(int(match[1]))
    ranges = []
    for line in lines:
        if ranges and ranges[-1][1] == line - 1:
            ranges[-1][1] = line
        else:
            ranges.append([line, line])
    return [str(a) if a == b else f"{a}-{b}" for a, b in ranges]


def uncovered(sources):
    if not PROFDATA.is_file():
        raise CoverageError(f"no {PROFDATA}: run scripts/build-ios.sh coverage first")
    root = ROOT.resolve()
    targets = dict(plan_targets(json.loads(PLAN.read_text())))
    for source in sources:
        path = Path(source).resolve()
        name = owning_target(relative_path(path, root))
        if name not in targets:
            raise CoverageError(f"{source} is in none of the measured targets ({', '.join(sorted(targets))})")
        objects = target_objects(name, targets[name], DERIVED_DATA)
        shown = llvm_cov("show", "-instr-profile", str(PROFDATA), *object_args(objects), "-sources", str(path))
        ranges = unexecuted_ranges(shown)
        print(f"{source}: " + (", ".join(ranges) if ranges else "every executable line ran"))


def expect_error(call, fragment):
    try:
        call()
    except CoverageError as error:
        assert fragment in str(error), (fragment, str(error))
    else:
        raise AssertionError(f"no error with {fragment!r}")


def self_test():
    assert percent(1, 3) == 33.33
    assert percent(2, 3) == 66.66
    assert percent(0, 0) == 100.0
    assert owning_target("mobile/ios/NgTerm/App.swift") == "NgTerm"
    assert owning_target("mobile/ios/Packages/NgtermCore/Sources/NgtermCore/Core.swift") == "NgtermCore"
    assert owning_target("mobile/ios/Packages/NgtermCore/Sources/NgtermCore/Sub/Deep.swift") == "NgtermCore"
    assert owning_target("mobile/ios/Packages/NgtermCore/Generated/NgtermCoreBindings/NgtermCoreBindings.swift") is None
    assert owning_target("mobile/ios/Packages/NgtermCore/Tests/NgtermCoreTests/CoreTests.swift") is None
    assert owning_target("mobile/ios/NgTermTests/KeyTests.swift") is None
    assert is_generated("mobile/ios/build/DerivedData/Build/Intermediates.noindex/x/resource_bundle_accessor.swift")

    plan = {
        "defaultOptions": {
            "codeCoverage": {
                "targets": [
                    {"containerPath": "container:NgTerm.xcodeproj", "identifier": "4E", "name": "NgTerm"},
                    {"containerPath": "container:Packages/NgtermCore", "identifier": "NgtermCore",
                     "name": "NgtermCore"},
                ]
            }
        }
    }
    assert plan_targets(plan) == [("NgTerm", True), ("NgtermCore", False)]
    expect_error(lambda: plan_targets({"defaultOptions": {}}), "no codeCoverage targets")

    with tempfile.TemporaryDirectory() as tmp:
        derived = Path(tmp)
        expect_error(lambda: target_objects("NgtermCore", False, derived), "no objects of NgtermCore")
        products = derived / "Build/Products" / CONFIGURATION
        products.mkdir(parents=True)
        (products / "NgtermCore.o").write_bytes(b"")
        assert target_objects("NgtermCore", False, derived) == [products / "NgtermCore.o"]
        app = derived / "Build/Intermediates.noindex/NgTerm.build" / CONFIGURATION / "NgTerm.build/Objects-normal/arm64"
        app.mkdir(parents=True)
        for name in ("B.o", "A.o", "A.swiftdeps"):
            (app / name).write_bytes(b"")
        assert [o.name for o in target_objects("NgTerm", True, derived)] == ["A.o", "B.o"]
        expect_error(lambda: merge_profiles(derived, derived / "out.profdata"), "no Coverage.profdata")
    assert object_args([Path("a.o"), Path("b.o"), Path("c.o")]) == ["a.o", "-object", "b.o", "-object", "c.o"]

    root = Path("/repo")

    def exported(path, covered, count):
        return {"filename": f"/repo/{path}", "summary": {"lines": {"covered": covered, "count": count}}}

    core = target_summary(
        "NgtermCore",
        [
            exported("mobile/ios/Packages/NgtermCore/Sources/NgtermCore/Idle.swift", 0, 6),
            exported("mobile/ios/Packages/NgtermCore/Sources/NgtermCore/Core.swift", 3, 4),
            exported("mobile/ios/build/DerivedData/Build/Intermediates.noindex/x/resource_bundle_accessor.swift", 0, 9),
        ],
        root,
    )
    assert (core["covered"], core["executable"], core["percent"]) == (3, 10, 30.0), core
    assert [f["path"].rsplit("/", 1)[1] for f in core["files"]] == ["Core.swift", "Idle.swift"]
    expect_error(
        lambda: target_summary("NgtermCore", [exported("mobile/ios/Packages/NgtermCore/Tests/T/T.swift", 1, 1)], root),
        "Tests/T/T.swift, which isn't one of its sources",
    )
    expect_error(
        lambda: target_summary("NgtermCore", [exported("mobile/ios/Packages/NgtermDesign/Sources/NgtermDesign/D.swift",
                                                       1, 1)], root),
        "which isn't one of its sources",
    )
    app_summary = target_summary("NgTerm", [exported("mobile/ios/NgTerm/App.swift", 5, 10)], root)
    summary = {"targets": [app_summary, core]}
    text = summary_text(summary)
    assert text.index("Idle.swift") < text.index("App.swift") < text.index("/Core.swift")

    floors = read_floors("# comment\n\nNgTerm 50  # the app\nNgtermCore 30\n")
    assert floors == {"NgTerm": 50, "NgtermCore": 30}
    for bad in ("NgTerm 50.5", "NgTerm", "NgTerm 101", "NgTerm 1\nNgTerm 2"):
        expect_error(lambda bad=bad: read_floors(bad), "floor file line")
    _, failures = check_floors(summary, floors)
    assert failures == [], failures
    _, failures = check_floors(summary, {"NgTerm": 51, "NgtermCore": 30, "Gone": 1})
    assert len(failures) == 2 and "Gone" in failures[0] and "below its floor of 51%" in failures[1], failures
    _, failures = check_floors(summary, {"NgTerm": 50})
    assert failures == ["NgtermCore has no floor: add `NgtermCore 30`"], failures

    show = "/x.swift:\n    1|       |import A\n    2|      3|  a()\n    3|      0|  b()\n    4|      0|  c()\n" \
        "    5|  1.20k|  d()\n    6|      0|  e()\n    7|       |}\n    8|      0|  f()\n"
    assert unexecuted_ranges(show) == ["3-4", "6", "8"]

    with tempfile.TemporaryDirectory() as tmp:
        summary_path, floors_path = Path(tmp, "Unit.coverage.json"), Path(tmp, "floors.txt")
        summary_path.write_text(json.dumps(summary))
        floors_path.write_text("NgTerm 50\nNgtermCore 31\n")
        expect_error(lambda: check(summary_path, floors_path), "NgtermCore covers 30.00%")
        expect_error(lambda: check(Path(tmp, "missing.json"), floors_path), "missing.json")
    print("ios-coverage self-test: ok")


def main(argv):
    try:
        if argv == ["--self-test"]:
            self_test()
        elif argv == ["summarize"]:
            summarize()
        elif argv == ["check"]:
            check(f"{SUMMARY}.json", FLOORS)
        elif len(argv) >= 2 and argv[0] == "uncovered":
            uncovered(argv[1:])
        else:
            raise SystemExit(USAGE)
    except CoverageError as error:
        raise SystemExit(f"ios-coverage: {error}") from None


if __name__ == "__main__":
    main(sys.argv[1:])
