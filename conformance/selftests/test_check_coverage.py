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


def check(
    capsys: pytest.CaptureFixture[str], summary: Path, floor: Path, *extra: str, batch: list[Path] | None = None
) -> tuple[int, str, str]:
    """Runs the tool on `batch` (default: the one summary, given twice under --add, a batch of two identical runs)."""
    summaries = batch if batch is not None else [summary] * (cc.MIN_ADD_SUMMARIES if "--add" in extra else 1)
    code = cc.main([arg for path in summaries for arg in ("--summary", str(path))] + ["--floor", str(floor), *extra])
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


def test_add_refuses_a_single_summary(files, capsys):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (6123, 10000)}, "ngterm-core 60.00\n")
    code, _, err = check(capsys, summary, floor, "--add", batch=[summary])
    assert code == 2
    assert "--add needs at least 2 --summary" in err
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 60.00\n"


def batch_of(tmp_path: Path, *runs: dict[str, tuple[int, int] | None]) -> list[Path]:
    paths = []
    for number, rows in enumerate(runs):
        path = tmp_path / f"summary-{number}.md"
        path.write_text(summary_text(rows), encoding="utf-8")
        paths.append(path)
    return paths


def test_a_repeated_summary_judges_each_crate_by_its_lowest(files, capsys, tmp_path):
    summary, floor = files
    write(summary, floor, {}, "ngterm-gpui 64.37\n")
    batch = batch_of(tmp_path, {"ngterm-gpui": (6438, 10000)}, {"ngterm-gpui": (6380, 10000)})
    code, out, _ = check(capsys, summary, floor, batch=batch)
    assert code == 1
    assert "BROKEN ngterm-gpui: 63.80%, floor 64.37% (0.57 pt below" in out


def test_add_raises_to_the_batch_lowest(files, capsys, tmp_path):
    summary, floor = files
    write(summary, floor, {}, "ngterm-core 60.00\nngterm-gpui 63.00\n")
    batch = batch_of(tmp_path, {"ngterm-core": (6200, 10000), "ngterm-gpui": (6438, 10000)},
                     {"ngterm-core": (6100, 10000), "ngterm-gpui": (6402, 10000)},
                     {"ngterm-core": (6300, 10000), "ngterm-gpui": (6380, 10000)})
    assert check(capsys, summary, floor, "--add", batch=batch)[0] == 0
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-core 61.00\nngterm-gpui 63.80\n"


def test_add_over_a_batch_keeps_a_partly_measured_crate_and_adds_only_one_measured_everywhere(files, capsys, tmp_path):
    summary, floor = files
    write(summary, floor, {}, "ngterm-core 60.00\nngterm-old 40.00\nngterm-wpe 50.00\n")
    batch = batch_of(tmp_path, {"ngterm-core": (7000, 10000), "ngterm-wpe": (9000, 10000), "ngterm-a": (1, 2),
                                "ngterm-b": (1, 4)},
                     {"ngterm-core": (7000, 10000), "ngterm-b": (1, 2)})
    assert check(capsys, summary, floor, "--add", batch=batch)[0] == 0
    # wpe: measured by one summary only, floor kept; old: by none, dropped; a: not everywhere, not added
    assert floor.read_text(encoding="utf-8") == HEADER + "ngterm-b 25.00\nngterm-core 70.00\nngterm-wpe 50.00\n"


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
        (
            "| crate | lines | regions | functions |\n| ngterm-core | 10.00% (1/10) |\n| total | 10.00% (1/10) |\n",
            "no separator row",
        ),
        ("| crate | lines | regions | functions |", "no separator row"),
        (
            "| crate | lines | regions | functions |\n|---|---:|---:|---:|\n"
            "| ngterm-core | 50.00% (1/2) | 50.00% (1/2) | 50.00% (1/2) |",
            "no total row",
        ),
        (
            "| crate | lines | regions | functions |\n|---|---:|---:|---:|\n| ngterm-core | 50.00% (1/2) |\n",
            "not a crate row",
        ),
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


@pytest.mark.parametrize(
    "name, metrics",
    [
        ("conformance/coverage-floor-linux.txt", ("lines",)),
        ("conformance/coverage-floor-windows.txt", ("lines",)),
        ("scripts/unit-coverage-floor-linux.txt", ("lines", "functions", "regions")),
    ],
)
def test_the_checked_in_floor_files_parse(name, metrics):
    path = REPO / name
    floors = cc.read_floors(path)
    assert floors.header, f"{path} has no header comment"
    assert floors.floors, f"{path} has no floor"
    assert floors.metrics == metrics


def test_runs_as_a_script(files):
    summary, floor = files
    write(summary, floor, {"ngterm-core": (1, 2)}, "ngterm-core 50.00\n")
    done = subprocess.run([sys.executable, str(TOOL), "--summary", str(summary), "--floor", str(floor)],
                          capture_output=True, text=True, check=False)
    assert done.returncode == 0, done.stderr
    assert "coverage floors hold" in done.stdout


# The three-column form (scripts/unit-coverage.sh's floors): `crate lines% functions% regions%`.
THREE = "# unit coverage floors\n# crate lines% functions% regions%\n"


def three_summary(rows: dict[str, dict[str, tuple[int, int]] | None]) -> str:
    """A summary.md with each metric's own (covered, count), or None for an n/a crate."""
    from agterm_conformance import code_coverage

    crates = {}
    for name, metrics in rows.items():
        row = code_coverage.CrateCoverage()
        for metric, (covered, count) in (metrics or {}).items():
            row.metrics[metric] = code_coverage.Counts(count=count, covered=covered)
        crates[name] = row
    return code_coverage.render(crates)


def measured(lines: int, functions: int, regions: int) -> dict[str, tuple[int, int]]:
    """Per-metric counts out of 10000, so each % is the number over 100."""
    return {"lines": (lines, 10000), "functions": (functions, 10000), "regions": (regions, 10000)}


def write_three(summary: Path, floor: Path, rows: dict[str, dict[str, tuple[int, int]] | None], floors: str) -> None:
    summary.write_text(three_summary(rows), encoding="utf-8")
    floor.write_text(THREE + floors, encoding="utf-8")


def test_three_columns_hold_within_half_a_point_each(files, capsys):
    summary, floor = files
    write_three(summary, floor, {"ngterm-core": measured(5950, 7000, 8000)}, "ngterm-core 60.00 70.00 80.20\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 0
    assert "coverage floors hold: 1 crate(s) judged (0 new, 0 gone)" in out


def test_three_columns_break_on_any_metric_naming_it(files, capsys):
    summary, floor = files
    write_three(summary, floor, {"ngterm-core": measured(6000, 6949, 5000), "ngterm-term": measured(1, 1, 1)},
                "ngterm-core 60.00 70.00 50.51\nngterm-term 0.00 0.00 0.00\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 1
    assert "BROKEN ngterm-core functions: 69.49%, floor 70.00% (0.51 pt below, more than 0.5 pt)" in out
    assert "BROKEN ngterm-core regions: 50.00%, floor 50.51% (0.51 pt below" in out
    assert "ngterm-core lines" not in out
    # two metrics below, one crate
    assert "coverage floors broken: 1 crate(s) of 2 below their floor" in out


def test_three_columns_add_raises_each_metric_on_its_own_and_keeps_the_form(files, capsys):
    summary, floor = files
    write_three(summary, floor, {"ngterm-core": measured(6123, 6990, 8456), "ngterm-new": measured(1, 2, 3)},
                "ngterm-core 60.00 70.00 80.00\nngterm-old 1.00 2.00 3.00\n")
    code, out, _ = check(capsys, summary, floor, "--add")
    assert code == 0
    assert "new, no floor yet: ngterm-new: lines 0.01%, functions 0.02%, regions 0.03%" in out
    assert "gone from the run, floor dropped by --add: ngterm-old (floor lines 1.00%, functions 2.00%" in out
    # functions 69.90 is within the tolerance of its 70.00 floor: kept, not lowered
    assert floor.read_text(encoding="utf-8") == THREE + "ngterm-core 61.23 70.00 84.56\nngterm-new 0.01 0.02 0.03\n"


def test_three_columns_add_raises_to_the_batch_lowest_per_metric(files, capsys, tmp_path):
    summary, floor = files
    write_three(summary, floor, {}, "ngterm-gpui 60.00 60.00 60.00\n")
    batch = []
    for number, counts in enumerate([(6438, 6100, 6250), (6380, 6200, 6240)]):
        path = tmp_path / f"three-{number}.md"
        path.write_text(three_summary({"ngterm-gpui": measured(*counts)}), encoding="utf-8")
        batch.append(path)
    assert check(capsys, summary, floor, "--add", batch=batch)[0] == 0
    assert floor.read_text(encoding="utf-8") == THREE + "ngterm-gpui 63.80 61.00 62.40\n"


def test_three_columns_skip_a_crate_with_no_lines(files, capsys):
    summary, floor = files
    write_three(summary, floor, {"ngterm-core": measured(1, 2, 3), "ngterm-proto": None},
                "ngterm-core 0.00 0.00 0.00\n")
    code, out, _ = check(capsys, summary, floor, "--add")
    assert code == 0
    assert "ngterm-proto" not in out


@pytest.mark.parametrize(
    "floors, message",
    [
        ("ngterm-core 60 70 80\nngterm-term 60\n", "1 value(s), where the file's floors have 3"),
        ("ngterm-core 60\nngterm-term 60 70 80\n", "3 value(s), where the file's floors have 1"),
        ("ngterm-core 60 70\n", "not `crate line%` or `crate lines% functions% regions%`"),
        ("ngterm-core 60 70 80 90\n", "not `crate line%` or `crate lines% functions% regions%`"),
        ("ngterm-core 60 seventy 80\n", "not a percentage"),
        ("ngterm-core 60 70 101\n", "out of 0..100"),
    ],
)
def test_a_malformed_three_column_floor_file_exits_2(files, capsys, floors, message):
    summary, floor = files
    write_three(summary, floor, {"ngterm-core": measured(1, 2, 3)}, floors)
    code, _, err = check(capsys, summary, floor)
    assert code == 2
    assert message in err and str(floor) in err


# The e2e report's summaries (code_coverage.py e2e): line % alone, a `total` floor and `--min`.
E2E = "# e2e floors\n# total, then crate line%\n"


def e2e_summary(rows: dict[str, tuple[int, int]], excluded: int = 0) -> str:
    """An e2e summary.md as code_coverage.render_e2e writes it: rows of (covered, count), each with `excluded` lines."""
    from agterm_conformance import code_coverage

    build = code_coverage.AppBuild(("ngterm",))
    table = {name: code_coverage.E2ERow(count, covered, excluded) for name, (covered, count) in rows.items()}
    return code_coverage.render_e2e("desktop", "linux", build, table, [])


def write_e2e(summary: Path, floor: Path, rows: dict[str, tuple[int, int]], floors: str) -> None:
    summary.write_text(e2e_summary(rows, excluded=7), encoding="utf-8")
    floor.write_text(E2E + floors, encoding="utf-8")


def test_an_e2e_summary_is_judged_by_its_line_column_alone(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (5000, 10000), "ngterm-gpui": (10, 100)}, "ngterm-core 60.00\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 1
    assert "BROKEN ngterm-core: 50.00%, floor 60.00% (10.00 pt below, more than 0.5 pt)" in out


def test_the_total_floor_is_judged_like_a_crate(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (5000, 10000), "ngterm-gpui": (5000, 10000)},
              "total 51.00\nngterm-core 50.00\n")
    code, out, _ = check(capsys, summary, floor)
    assert code == 1
    assert "BROKEN total: 50.00%, floor 51.00% (1.00 pt below, more than 0.5 pt)" in out
    assert "coverage floors broken: 1 crate(s) of 2 below their floor" in out
    assert "new, no floor yet: ngterm-gpui: 50.00%" in out and "new, no floor yet: total" not in out


def test_add_raises_the_total_floor_and_keeps_it_first(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (6000, 10000), "ngterm-app": (7000, 10000)}, "total 10.00\n")
    assert check(capsys, summary, floor, "--add")[0] == 0
    assert floor.read_text(encoding="utf-8") == E2E + "total 65.00\nngterm-app 70.00\nngterm-core 60.00\n"


def test_add_never_adds_a_total_to_a_file_without_one(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (6000, 10000)}, "ngterm-core 50.00\n")
    assert check(capsys, summary, floor, "--add")[0] == 0
    assert floor.read_text(encoding="utf-8") == E2E + "ngterm-core 60.00\n"


def test_a_total_floor_below_the_first_line_is_refused(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (1, 2)}, "ngterm-core 50.00\ntotal 50.00\n")
    code, _, err = check(capsys, summary, floor)
    assert code == 2
    assert "the total floor goes first" in err and str(floor) in err


def test_min_fails_a_total_under_it_naming_both_numbers(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (8520, 10000)}, "total 85.00\n")
    code, out, _ = check(capsys, summary, floor, "--min", "90")
    assert code == 1
    assert "BROKEN total: 85.20% of lines, under --min 90.00% (4.80 pt below)" in out
    assert "coverage under the minimum: total 85.20% of lines, --min 90.00%" in out


def test_min_holds_at_the_minimum_and_judges_the_batchs_lowest_total(files, capsys, tmp_path):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (9000, 10000)}, "total 89.00\n")
    assert check(capsys, summary, floor, "--min", "90")[0] == 0
    batch = []
    for number, covered in enumerate((9100, 8999)):
        path = tmp_path / f"e2e-{number}.md"
        path.write_text(e2e_summary({"ngterm-core": (covered, 10000)}), encoding="utf-8")
        batch.append(path)
    code, out, _ = check(capsys, summary, floor, "--min", "90", batch=batch)
    assert code == 1
    assert "BROKEN total: 89.99% of lines, under --min 90.00% (0.01 pt below)" in out


def test_min_skips_add(files, capsys):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (8000, 10000)}, "total 70.00\n")
    code, _, err = check(capsys, summary, floor, "--min", "90", "--add")
    assert code == 1
    assert "--add skipped" in err
    assert floor.read_text(encoding="utf-8") == E2E + "total 70.00\n"


@pytest.mark.parametrize("value", ["101", "-1", "nan", "ninety"])
def test_a_min_out_of_range_exits_2(files, capsys, value):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (1, 2)}, "")
    code, _, err = check(capsys, summary, floor, "--min", value)
    assert code == 2
    assert "--min" in err


def test_the_total_and_its_distance_to_90_are_always_printed(files, capsys, tmp_path, monkeypatch):
    summary, floor = files
    write_e2e(summary, floor, {"ngterm-core": (6120, 10000)}, "")
    job = tmp_path / "step-summary.md"
    monkeypatch.setenv("GITHUB_STEP_SUMMARY", str(job))
    code, out, _ = check(capsys, summary, floor)
    assert code == 0
    assert "total: 61.20% of lines, 28.80 pt to 90%" in out
    assert "total: 61.20% of lines, 28.80 pt to 90%" in job.read_text(encoding="utf-8")


def test_floors_of_a_metric_the_summary_lacks_exit_2(files, capsys):
    summary, floor = files
    summary.write_text(e2e_summary({"ngterm-core": (1, 2)}), encoding="utf-8")
    floor.write_text(THREE + "ngterm-core 1.00 2.00 3.00\n", encoding="utf-8")
    code, _, err = check(capsys, summary, floor)
    assert code == 2
    assert "no functions or regions column, which the floors need" in err


def test_an_e2e_summary_with_no_total_numbers_cannot_take_min(files, capsys):
    summary, floor = files
    summary.write_text(e2e_summary({"ngterm-core": (0, 0)}), encoding="utf-8")
    floor.write_text(E2E, encoding="utf-8")
    code, _, err = check(capsys, summary, floor, "--min", "90")
    assert code == 2
    assert "no total line % to hold against --min" in err
