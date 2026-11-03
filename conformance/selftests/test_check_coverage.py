import importlib.util
import subprocess
import sys
from pathlib import Path
from types import ModuleType

import pytest

TOOLS = Path(__file__).resolve().parent.parent / "tools"
TOOL = TOOLS / "check_coverage.py"
REPO = TOOLS.parent.parent
HEADER = "# coverage floors\n# crate line%\n"


def _load(name: str) -> ModuleType:
    spec = importlib.util.spec_from_file_location(name, TOOLS / f"{name}.py")
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module  # dataclasses resolve their module by name
    spec.loader.exec_module(module)
    return module


cc = _load("check_coverage")


def summary_text(rows: dict[str, tuple[int, int] | None]) -> str:
    """A summary.md as agterm_conformance.code_coverage.render writes it: rows of (covered, count) or None for n/a."""
    from agterm_conformance import code_coverage

    crates = {}
    for name, counts in rows.items():
        row = code_coverage.CrateCoverage()
        if counts is not None:
            for metric in code_coverage.METRICS:
                row.metrics[metric] = code_coverage.Counts(count=counts[1], covered=counts[0])
        crates[name] = row
    return code_coverage.render(crates, ["2 functions have mismatched data"])


@pytest.fixture
def files(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path]:
    monkeypatch.delenv("GITHUB_STEP_SUMMARY", raising=False)
    return tmp_path / "summary.md", tmp_path / "coverage-floor-linux.txt"


def write(summary: Path, floor: Path, rows: dict[str, tuple[int, int] | None], floors: str) -> None:
    summary.write_text(summary_text(rows), encoding="utf-8")
    floor.write_text(HEADER + floors, encoding="utf-8")


def check(capsys: pytest.CaptureFixture[str], summary: Path, floor: Path, *extra: str) -> tuple[int, str, str]:
    code = cc.main(["--summary", str(summary), "--floor", str(floor), *extra])
    out, err = capsys.readouterr()
    return code, out, err


def test_a_drop_within_half_a_point_holds(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (5960, 10000), "ngterm-term": (5000, 10000)}, "ngterm-core 60.00\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 0
    assert "coverage floors hold: 1 crate(s) judged (1 new, 0 gone)" in out


def test_a_drop_of_exactly_half_a_point_holds(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (5950, 10000)}, "ngterm-core 60.00\n")
    assert check(capsys, summary, floor)[0] == 0


def test_a_larger_drop_fails_naming_the_crate_and_both_numbers(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (5949, 10000), "ngterm-term": (5000, 10000)},
          "ngterm-core 60.00\nngterm-term 50.00\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 1
    assert "BROKEN ngterm-core: 59.49%, floor 60.00% (0.51 pt below, more than 0.5 pt)" in out
    assert "ngterm-term" not in out.split("BROKEN")[1].splitlines()[0]
    assert "coverage floors broken: 1 crate(s) of 2 below their floor" in out


def test_add_raises_and_never_lowers(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (6123, 10000), "ngterm-term": (4980, 10000)},
          "ngterm-core 60.00\nngterm-term 50.00\n")
    code, _, _ = check(capsys, summary, floor, "--add")
    assert code == 0
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 61.23\nngterm-term 50.00\n"


def test_add_rounds_a_raised_floor_down(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (2, 3)}, "")
    assert check(capsys, summary, floor, "--add")[0] == 0
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 66.66\n"


def test_add_adds_a_new_crate_and_drops_a_removed_one(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-app": (1, 4), "ngterm-core": (6000, 10000)},
          "ngterm-core 60.00\nngterm-old 40.00\n")
    code, out, _ = check(capsys, summary, floor, "--add")
    assert code == 0
    assert "new, no floor yet: ngterm-app: 25.00%" in out
    assert "gone from the run, floor dropped by --add: ngterm-old (floor 40.00%)" in out
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-app 25.00\nngterm-core 60.00\n"


def test_add_seeds_an_empty_floor_file(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 2), "ngterm-ctl": (3, 4)}, "")
    assert check(capsys, summary, floor, "--add")[0] == 0
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 50.00\nngterm-ctl 75.00\n"


def test_add_writes_nothing_when_the_floors_are_broken(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 10), "ngterm-new": (1, 2)}, "ngterm-core 60.00\n")
    code, _, err = check(capsys, summary, floor, "--add")
    assert code == 1
    assert "--add skipped" in err
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 60.00\n"


def test_a_crate_with_no_lines_is_neither_judged_nor_added(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 2), "ngterm-proto": None}, "")
    code, out, _ = check(capsys, summary, floor, "--add")
    assert code == 0
    assert "ngterm-proto" not in out
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 50.00\n"


@pytest.mark.parametrize(
    "floors, message",
    [
        ("ngterm-core\n", "not `crate line%`"),
        ("ngterm-core 60.00 extra\n", "not `crate line%`"),
        ("ngterm-core sixty\n", "not a percentage"),
        ("ngterm-core 101\n", "out of 0..100"),
        ("ngterm-core nan\n", "out of 0..100"),
        ("ngterm-core 60\nngterm-core 61\n", "a second floor"),
        ("ngterm-term 60\nngterm-core 61\n", "out of order"),
        ("ngterm-core 60\n# late\n", "a comment after the first floor"),
        ("ngterm-core 60\r\n", "a carriage return"),
        ("\n", "not `crate line%`"),
    ],
)
def test_a_malformed_floor_file_exits_2(files, capsys, floors, message):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 2)}, floors)
    code, _, err = check(capsys, summary, floor, "--add")
    assert code == 2
    assert message in err and str(floor) in err


def test_a_missing_floor_file_exits_2(files, capsys):
    summary, floor = files
    summary.write_text(summary_text({"ngterm-core": (1, 2)}), encoding="utf-8")
    code, _, err = check(capsys, summary, floor)
    assert code == 2
    assert "cannot read the floor file" in err


@pytest.mark.parametrize(
    "text, message",
    [
        ("no report\n", "no per-crate table"),
        ("| crate | lines | regions | functions |\n|---|---:|---:|---:|\n| ngterm-core | 50.00% (1/2) |", "no total row"),
        ("| crate | lines | regions | functions |\n|---|---:|---:|---:|\n| ngterm-core | half |\n", "not a crate row"),
    ],
)
def test_a_malformed_summary_exits_2(files, capsys, text, message):
    summary, floor = files
    summary.write_text(text, encoding="utf-8")
    floor.write_text(HEADER, encoding="utf-8")
    code, _, err = check(capsys, summary, floor)
    assert code == 2
    assert message in err


def test_the_verdict_goes_to_the_job_summary(files, capsys, tmp_path, monkeypatch):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 10)}, "ngterm-core 60.00\n")
    job = tmp_path / "step-summary.md"
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(job))
    assert check(capsys, summary, floor)[0] == 1
    text = job.read_text(encoding="utf-8")
    assert "### Coverage floors" in text
    assert "ngterm-core: 10.00%, floor 60.00%" in text


@pytest.mark.parametrize("name", ["coverage-floor-linux.txt", "coverage-floor-windows.txt"])
def test_the_checked_in_floor_files_parse(name):
    path = REPO / "conformance" / name
    floors = cc.read_floors(path)
    assert floors.header, f"{path} has no header comment"


def test_runs_as_a_script(files):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 2)}, "ngterm-core 50.00\n")
    done = subprocess.run([sys.executable, str(TOOL), "--summary", str(summary), "--floor", str(floor)],
                          capture_output=True, text=True, check=False)
    assert done.returncode == 0, done.stderr
    assert "coverage floors hold" in done.stdout
