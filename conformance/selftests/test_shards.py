"""`--shard I/N` (agterm_conformance/shards.py), `tools/merge_shards.py` and `tools/shard_durations.py`."""

import json
import subprocess
import sys
import xml.etree.ElementTree as ElementTree
from pathlib import Path
from typing import Any

import pytest
from agterm_conformance import shards

TOOLS = Path(__file__).resolve().parent.parent / "tools"
MERGE = TOOLS / "merge_shards.py"
DURATIONS = TOOLS / "shard_durations.py"

FILES = {
    "test_a.py": "def test_a1():\n    pass\n\ndef test_a2():\n    pass\n",
    "test_b.py": "def test_b1():\n    pass\n",
    "test_c.py": "def test_c1():\n    pass\n\ndef test_c2():\n    assert False\n",
    "test_d.py": "def test_d1():\n    pass\n",
}
ALL = ["test_a.py::test_a1", "test_a.py::test_a2", "test_b.py::test_b1", "test_c.py::test_c1", "test_c.py::test_c2",
       "test_d.py::test_d1"]


def read(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]


def ids(records: list[dict[str, Any]]) -> list[str]:
    return [record["nodeid"] for record in records if record["kind"] == "test"]


def run_shards(pytester: pytest.Pytester, count: int, *args: str) -> list[Path]:
    """One run per shard, each with its report directory `shard-I` (nodeids.jsonl, junit.xml)."""
    directories = []
    for index in range(1, count + 1):
        directory = pytester.path / f"shard-{index}"
        pytester.runpytest(
            "--shard", f"{index}/{count}", "--nodeid-report", str(directory / "nodeids.jsonl"),
            "--junitxml", str(directory / "junit.xml"), "--trace-mode", "off", *args,
        )
        directories.append(directory)
    return directories


def merge(*args: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(MERGE), *map(str, args)], capture_output=True, text=True, check=False)


def ratchet(path: Path, *listed: str) -> Path:
    path.write_text("# listed\n" + "".join(f"{nodeid}\n" for nodeid in sorted(listed)), encoding="utf-8")
    return path


# -- the split


@pytest.mark.parametrize("text", ["0/2", "3/2", "1", "1/0", "a/b", "1/2/3", "-1/2", ""])
def test_a_bad_shard_is_a_usage_error(text: str) -> None:
    with pytest.raises(pytest.UsageError, match="--shard"):
        shards.parse(text)


def test_the_heaviest_files_go_to_the_lightest_shard() -> None:
    tests = {"a": 1, "b": 1, "c": 1, "d": 1}
    assert shards.assign(tests, {"a": 10, "b": 6, "c": 5, "d": 1}, 2) == {"a": 1, "b": 2, "c": 2, "d": 1}


def test_equal_files_split_by_path_whatever_their_order() -> None:
    assert shards.assign({"b": 1, "a": 1, "c": 1}, {}, 3) == shards.assign({"c": 1, "a": 1, "b": 1}, {}, 3) == {
        "a": 1, "b": 2, "c": 3,
    }


def test_a_file_without_a_weight_weighs_its_tests_at_the_recorded_mean() -> None:
    # 10 recorded seconds over 5 tests: 2 s a test, so the new file's 4 tests weigh 8
    weights = shards.file_weights({"old.py": 5, "new.py": 4}, {"old.py": 10.0, "gone.py": 99.0})
    assert weights == {"old.py": 10.0, "new.py": 8.0}
    assert shards.file_weights({"x.py": 3}, {}) == {"x.py": 3.0}


def test_the_digest_names_the_whole_table() -> None:
    one = shards.split_digest({"a": 1, "b": 2})
    assert one == shards.split_digest({"b": 2, "a": 1})
    assert one != shards.split_digest({"a": 2, "b": 1})


@pytest.mark.parametrize("text", ["fast a.py\n", "1.5\n", "-1 a.py\n", "nan a.py\n", "1 a.py\n2 a.py\n"])
def test_bad_weights_are_refused(text: str) -> None:
    with pytest.raises(ValueError, match="weights:"):
        shards.parse_weights(text, "weights")


def test_weights_keep_a_path_with_spaces() -> None:
    assert shards.parse_weights("# header\n\n1.5 tests/a b.py\n", "weights") == {"tests/a b.py": 1.5}


# -- the option


def test_every_test_runs_in_exactly_one_shard_with_its_file(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    reports = [read(directory / "nodeids.jsonl") for directory in run_shards(pytester, 3)]
    ran = [nodeid for records in reports for nodeid in ids(records)]
    assert sorted(ran) == ALL
    for records in reports:
        files = {shards.file_of(nodeid) for nodeid in ids(records)}
        assert all(shards.file_of(nodeid) not in files for other in reports if other is not records for nodeid in ids(other))
    headers = [records[0] for records in reports]
    assert [header["shard"]["index"] for header in headers] == [1, 2, 3]
    assert {(h["shard"]["count"], h["shard"]["collected"], h["shard"]["split"]) for h in headers} == {
        (3, 6, headers[0]["shard"]["split"])
    }
    # the other shards run the rest: a shard alone is still a full run's part, not a narrowed run
    assert {(header["partial"], header["mode"]) for header in headers} == {(False, "full")}


def test_a_shard_keeps_its_files_tests_in_their_order(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(test_ordered="".join(f"def test_{number}():\n    pass\n\n" for number in (3, 1, 2)))
    (report,) = [read(directory / "nodeids.jsonl") for directory in run_shards(pytester, 1)]
    assert ids(report) == ["test_ordered.py::test_3", "test_ordered.py::test_1", "test_ordered.py::test_2"]


def test_the_weights_decide_the_split(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    (pytester.path / "weights.txt").write_text("9 test_d.py\n1 test_a.py\n1 test_b.py\n1 test_c.py\n")
    first, second = (read(d / "nodeids.jsonl") for d in run_shards(pytester, 2, "--shard-weights", "weights.txt"))
    assert ids(first) == ["test_d.py::test_d1"]
    assert sorted(ids(second)) == ALL[:5]


def test_a_narrower_selection_inside_a_shard_is_partial(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    reports = [read(directory / "nodeids.jsonl") for directory in run_shards(pytester, 2, "-k", "1")]
    assert {(records[0]["partial"], records[0]["mode"]) for records in reports} == {(True, "partial")}
    assert sorted(nodeid for records in reports for nodeid in ids(records)) == [n for n in ALL if n.endswith("1")]
    # the split is of the whole collection, whatever -k then leaves of it
    assert {records[0]["shard"]["collected"] for records in reports} == {6}


def test_a_ratchet_only_extra_may_be_in_another_shard(pytester: pytest.Pytester) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    ratchet(pytester.path / "list.txt", "test_a.py::test_a1")
    reports = [read(directory / "nodeids.jsonl")
               for directory in run_shards(pytester, 2, "--ratchet-only", "list.txt", "--also", "test_d.py")]
    assert sorted(nodeid for records in reports for nodeid in ids(records)) == ["test_a.py::test_a1", "test_d.py::test_d1"]
    assert {(records[0]["partial"], records[0]["mode"]) for records in reports} == {(True, "ratchet-only")}


def test_the_records_carry_their_duration(pytester: pytest.Pytester) -> None:
    pytester.makepyfile("import time\n\ndef test_slow():\n    time.sleep(0.05)\n\ndef test_fast():\n    pass\n")
    pytester.runpytest("--nodeid-report", "nodeids.jsonl", "--trace-mode", "off")
    slow, fast = read(pytester.path / "nodeids.jsonl")[1:]
    assert slow["duration"] >= 0.05 > fast["duration"] >= 0


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (("--shard", "4/3"), "*--shard: '4/3' is not I/N*"),
        (("--shard-weights", "w.txt"), "*--shard-weights needs --shard*"),
        (("--shard", "1/2", "--shard-weights", "missing.txt"), "*--shard-weights:*missing.txt*"),
        (("--shard", "1/2", "--coverage-gate"), "*--shard cannot be combined with --coverage-gate*"),
    ],
)
def test_shard_usage_errors(pytester: pytest.Pytester, args: tuple[str, ...], message: str) -> None:
    result = pytester.runpytest(*args)
    assert result.ret == pytest.ExitCode.USAGE_ERROR
    result.stderr.fnmatch_lines([message])


MUST_PASS = {
    "test_live": """
import pytest

@pytest.mark.requires("live-sessions")
def test_live():
    {live}
""",
    "test_plain": "def test_plain():\n    pass\n",
}


def must_pass_shards(pytester: pytest.Pytester, live: str = "pass") -> list[Path]:
    pytester.makepyfile(**{name: body.replace("{live}", live) for name, body in MUST_PASS.items()})
    return run_shards(pytester, 2, "--must-pass", "live-sessions")


def test_a_shard_without_a_must_pass_test_is_no_problem(pytester: pytest.Pytester) -> None:
    """Whether a test requires the capability at all is the whole run's to say: the shard counts, the merge judges."""
    reports = [read(directory / "nodeids.jsonl") for directory in must_pass_shards(pytester)]
    assert not [record for records in reports for record in records if record["kind"] == "must-pass"]
    counts = [record for records in reports for record in records if record["kind"] == "must-pass-capability"]
    assert sorted((count["required"], count["passed"]) for count in counts) == [(0, 0), (1, 1)]


def test_a_shards_own_must_pass_test_that_fails_is_its_problem(pytester: pytest.Pytester) -> None:
    reports = [read(directory / "nodeids.jsonl") for directory in must_pass_shards(pytester, "assert False")]
    problems = [record["problem"] for records in reports for record in records if record["kind"] == "must-pass"]
    assert problems == ["test_live.py::test_live"]


# -- the merge


def test_the_merged_report_is_the_whole_runs(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    directories = run_shards(pytester, 3)
    (directories[0] / "traces" / "one").mkdir(parents=True)
    (directories[1] / "traces" / "two").mkdir(parents=True)
    (directories[0] / "report.md").write_text("## rust-linux conformance\n\n- build: abc1234, `/stage`\n- suite time: 7s\n")
    (directories[0] / "pytest.log").write_text("first log\n")
    out = tmp_path / "merged"
    listed = ratchet(tmp_path / "list.txt", *[nodeid for nodeid in ALL if nodeid != "test_c.py::test_c2"])
    done = merge("--out", out, "--list", listed, "--title", "rust-linux", *reversed(directories))
    assert done.returncode == 0, done.stdout + done.stderr
    records = read(out / "nodeids.jsonl")
    assert sorted(ids(records)) == ALL
    assert {key: records[0][key] for key in ("partial", "mode", "shards", "collected")} == {
        "partial": False, "mode": "full", "shards": 3, "collected": 6,
    }
    assert "shard" not in records[0]
    suites = ElementTree.parse(out / "junit.xml").getroot().findall("testsuite")
    assert sorted(suite.get("name") for suite in suites) == [f"pytest shard {index}/3" for index in (1, 2, 3)]
    assert sum(len(suite.findall("testcase")) for suite in suites) == 6
    assert (out / "traces" / "one").is_dir() and (out / "traces" / "two").is_dir()
    assert (out / "shards" / "1" / "report.md").is_file()
    assert "===== shard 1/3 =====\nfirst log\n===== shard 2/3 =====\n" in (out / "pytest.log").read_text()
    report = (out / "report.md").read_text()
    assert "## rust-linux conformance" in report and "- build: abc1234, `/stage`" in report
    assert "- tests: 6 of 6 collected" in report
    assert "5 pass, 1 fail, 0 skip(unsupported), 0 skip(not-yet-at-level), 0 skip(other), parity 83.3%" in report
    assert "| 1/3 |" in report and "| 7s |" in report
    assert "### failures" in report and "test_c.py::test_c2" in report
    assert "ratchet holds: 5 listed id(s), full run" in report and "alone" not in report


def test_a_red_listed_id_breaks_the_merged_ratchet(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    done = merge("--out", tmp_path / "merged", "--list", ratchet(tmp_path / "list.txt", *ALL), *run_shards(pytester, 2))
    assert done.returncode == 1
    assert "BROKEN failed: test_c.py::test_c2" in done.stdout


def test_a_missing_shard_is_no_holding_run(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    first, second, third = run_shards(pytester, 3)
    done = merge("--out", tmp_path / "merged", "--list", ratchet(tmp_path / "list.txt", ALL[0]), first, third)
    assert done.returncode == 2
    assert "shard 2/3: no report" in done.stderr
    (second / "nodeids.jsonl").unlink()
    done = merge("--out", tmp_path / "merged", first, second, third)
    assert done.returncode == 2
    assert "no nodeid report (the shard's run wrote none)" in done.stderr


def test_shards_of_two_splits_do_not_merge(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    first, second = run_shards(pytester, 2)
    records = read(second / "nodeids.jsonl")
    records[0]["shard"]["split"] = "0" * 64
    (second / "nodeids.jsonl").write_text("".join(json.dumps(record) + "\n" for record in records))
    done = merge("--out", tmp_path / "merged", first, second)
    assert done.returncode == 2
    assert "are not shards of one split: their `split` differ" in done.stderr


def test_a_shard_given_twice_or_a_test_in_two_shards_is_refused(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(**{name.removesuffix(".py"): body for name, body in FILES.items()})
    first, second = run_shards(pytester, 2)
    done = merge("--out", tmp_path / "merged", first, first)
    assert done.returncode == 2 and "are both shard 1" in done.stderr
    records = read(second / "nodeids.jsonl")
    records.append(read(first / "nodeids.jsonl")[1])
    (second / "nodeids.jsonl").write_text("".join(json.dumps(record) + "\n" for record in records))
    done = merge("--out", tmp_path / "merged", first, second)
    assert done.returncode == 2 and "is in shard 1 and in shard 2" in done.stderr


def test_a_whole_runs_report_is_not_a_shard(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile("def test_one():\n    pass\n")
    pytester.runpytest("--nodeid-report", "whole/nodeids.jsonl", "--trace-mode", "off")
    done = merge("--out", tmp_path / "merged", pytester.path / "whole")
    assert done.returncode == 2 and "not a shard's report" in done.stderr


def test_the_merge_judges_the_capabilities_of_the_whole_run(pytester: pytest.Pytester, tmp_path: Path) -> None:
    listed = ratchet(tmp_path / "list.txt", "test_plain.py::test_plain")
    done = merge("--out", tmp_path / "merged", "--list", listed, *must_pass_shards(pytester))
    assert done.returncode == 0, done.stdout + done.stderr
    assert not [record for record in read(tmp_path / "merged" / "nodeids.jsonl") if record["kind"] == "must-pass"]


def test_a_capability_no_shard_passed_breaks_the_merged_run(pytester: pytest.Pytester, tmp_path: Path) -> None:
    skip = "pytest.skip('unsupported: swift-macos lacks pages-survive (upstream ends it)')"
    pytester.makepyfile(
        test_live=f"import pytest\n\n@pytest.mark.requires('live-sessions', 'pages-survive')\ndef test_live():\n    {skip}\n",
        test_plain="def test_plain():\n    pass\n",
    )
    directories = run_shards(pytester, 2, "--must-pass", "live-sessions")
    listed = ratchet(tmp_path / "list.txt", "test_plain.py::test_plain")
    done = merge("--out", tmp_path / "merged", "--list", listed, *directories)
    assert done.returncode == 1
    assert "BROKEN must-pass: no test requiring live-sessions passed" in done.stdout


def test_a_capability_no_shard_collected_breaks_the_merged_run(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(test_one="def test_one():\n    pass\n", test_two="def test_two():\n    pass\n")
    directories = run_shards(pytester, 2, "--must-pass", "live-sessions")
    listed = ratchet(tmp_path / "list.txt", "test_one.py::test_one")
    done = merge("--out", tmp_path / "merged", "--list", listed, *directories)
    assert done.returncode == 1
    assert "BROKEN must-pass: no collected test requires live-sessions" in done.stdout


def test_a_collection_error_every_shard_met_is_reported_once(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(test_one="def test_one():\n    pass\n", test_broken="import no_such_module\n")
    directories = run_shards(pytester, 2)
    done = merge("--out", tmp_path / "merged", "--list", ratchet(tmp_path / "list.txt", "test_one.py::test_one"),
                 *directories)
    errors = [r for r in read(tmp_path / "merged" / "nodeids.jsonl") if r["kind"] == "collection-error"]
    assert done.returncode == 1 and len(errors) == 1
    assert done.stdout.count("BROKEN collection error: test_broken.py") == 1


def test_the_merge_joins_the_shards_coverage(pytester: pytest.Pytester, tmp_path: Path) -> None:
    pytester.makepyfile(test_one="def test_one():\n    pass\n", test_two="def test_two():\n    pass\n")
    directories = run_shards(pytester, 2)
    repo = tmp_path / "repo"
    source = repo / "crates" / "ngterm-core" / "src" / "lib.rs"
    source.parent.mkdir(parents=True)
    source.write_text("fn one() {}\nfn two() {}\n")
    (source.parents[1] / "Cargo.toml").write_text('[package]\nname = "ngterm-core"\n')
    (repo / "conformance" / "coverage").mkdir(parents=True)
    (repo / "conformance" / "coverage" / "apps.toml").write_text('[desktop.linux]\nbinaries = ["ngterm"]\n')
    for directory, ran in zip(directories, ((1, 0), (0, 1)), strict=True):
        (directory / "coverage").mkdir()
        (directory / "coverage" / "lcov.info").write_text(
            f"SF:crates/ngterm-core/src/lib.rs\nDA:1,{ran[0]}\nDA:2,{ran[1]}\nend_of_record\n"
        )
        (directory / "lost-profiles.txt").write_text(f"{ran[0]} ngterm: killed\n")
    out = tmp_path / "merged"
    done = merge("--out", out, "--coverage", "desktop", "linux", "--repo", repo, *directories)
    assert done.returncode == 0, done.stdout + done.stderr
    # each shard ran one of the two lines: the whole run ran both
    assert "**Total: 100.00% of lines (2/2)" in (out / "coverage" / "summary.md").read_text()
    assert "### code coverage" in (out / "report.md").read_text()
    assert (out / "lost-profiles.txt").read_text() == "1 ngterm: killed\n0 ngterm: killed\n"
    (directories[1] / "coverage" / "lcov.info").unlink()
    done = merge("--out", out, "--coverage", "desktop", "linux", "--repo", repo, *directories)
    assert done.returncode == 2 and "no coverage report from a shard" in done.stderr


# -- the weights


def test_the_weights_are_each_files_seconds_averaged_over_the_reports(tmp_path: Path) -> None:
    def report(name: str, **seconds: float) -> Path:
        lines = [{"kind": "run", "partial": False}]
        lines += [{"kind": "test", "nodeid": f"tests/{file}.py::test", "duration": value} for file, value in seconds.items()]
        lines.append({"kind": "test", "nodeid": "tests/a.py::test_more", "duration": 1.0})
        path = tmp_path / name
        path.write_text("".join(json.dumps(line) + "\n" for line in lines))
        return path

    out = tmp_path / "weights.txt"
    done = subprocess.run(
        [sys.executable, str(DURATIONS), str(report("one", a=2.0, b=4.0)), str(report("two", a=4.0)), "--write", str(out)],
        capture_output=True, text=True, check=False,
    )
    assert done.returncode == 0, done.stderr
    assert shards.parse_weights(out.read_text(), "weights") == {"tests/a.py": 4.0, "tests/b.py": 4.0}


def test_a_report_without_durations_gives_no_weights(tmp_path: Path) -> None:
    path = tmp_path / "nodeids.jsonl"
    path.write_text('{"kind": "run", "partial": false}\n{"kind": "test", "nodeid": "tests/a.py::test"}\n')
    done = subprocess.run([sys.executable, str(DURATIONS), str(path)], capture_output=True, text=True, check=False)
    assert done.returncode == 2 and "no test record with a duration" in done.stderr


@pytest.mark.parametrize("platform", ["linux", "windows"])
def test_the_checked_in_weights_parse_and_name_test_files(platform: str) -> None:
    path = TOOLS.parent / f"shard-weights-{platform}.txt"
    weights = shards.parse_weights(path.read_text(encoding="utf-8"), str(path))
    assert weights and all(file.startswith("tests/") and file.endswith(".py") for file in weights)
