import json
import re
import subprocess
from pathlib import Path
from typing import Any

import pytest
from agterm_conformance import code_coverage
from agterm_conformance.code_coverage import ReportError


def summary(lines: tuple[int, int], regions: tuple[int, int] = (0, 0), functions: tuple[int, int] = (0, 0)) -> dict:
    def counts(pair: tuple[int, int]) -> dict[str, Any]:
        return {"count": pair[0], "covered": pair[1], "percent": 100.0 * pair[1] / pair[0] if pair[0] else 0}

    return {"lines": counts(lines), "regions": counts(regions), "functions": counts(functions)}


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    root = tmp_path / "repo"
    for directory, name in (("ngterm-core", "ngterm-core"), ("ngterm-gpui", "ngterm-gpui"), ("ngterm-vt-sys", "x-sys")):
        (root / "crates" / directory).mkdir(parents=True)
        (root / "crates" / directory / "Cargo.toml").write_text(f'[package]\nname = "{name}"\n')
    return root


def export(repo: Path, *files: tuple[str, dict]) -> dict[str, Any]:
    return {"data": [{"files": [{"filename": str(repo / name), "summary": data} for name, data in files]}]}


def test_rows_sum_files_per_crate_and_leave_out_vendor_and_sys(repo: Path) -> None:
    rows = code_coverage.crate_rows(
        export(
            repo,
            ("crates/ngterm-gpui/src/main.rs", summary((10, 5), (20, 4), (2, 1))),
            ("crates/ngterm-core/src/lib.rs", summary((100, 50), (200, 100), (10, 5))),
            ("crates/ngterm-core/src/model.rs", summary((100, 25), (0, 0), (10, 0))),
            ("crates/ngterm-vt-sys/src/bindings.rs", summary((1000, 0))),
            ("vendor/gpui-pre/src/lib.rs", summary((1000, 0))),
        ),
        repo,
    )
    assert list(rows) == ["ngterm-core", "ngterm-gpui"]
    core = rows["ngterm-core"].metrics
    assert (core["lines"].count, core["lines"].covered) == (200, 75)
    assert (core["regions"].count, core["regions"].covered) == (200, 100)
    assert (core["functions"].count, core["functions"].covered) == (20, 5)


def test_a_file_outside_the_repo_is_left_out(repo: Path) -> None:
    rows = code_coverage.crate_rows(
        {"data": [{"files": [{"filename": "/rustc/abc/library/core/src/lib.rs", "summary": summary((5, 5))}]}]}, repo
    )
    assert rows == {}


def test_a_crate_is_named_by_its_package(repo: Path) -> None:
    (repo / "crates" / "ngterm-ctl").mkdir()
    (repo / "crates" / "ngterm-ctl" / "Cargo.toml").write_text('[package]\nname = "ngterm-ctl"\n[[bin]]\nname = "x"\n')
    (repo / "crates" / "odd").mkdir()  # no manifest: its directory's name
    rows = code_coverage.crate_rows(
        export(repo, ("crates/ngterm-ctl/src/main.rs", summary((4, 1))), ("crates/odd/src/lib.rs", summary((2, 2)))),
        repo,
    )
    assert list(rows) == ["ngterm-ctl", "odd"]


def test_the_summary_has_a_row_per_crate_and_a_total(repo: Path) -> None:
    rows = code_coverage.crate_rows(
        export(
            repo,
            ("crates/ngterm-core/src/lib.rs", summary((200, 50), (100, 25), (10, 5))),
            ("crates/ngterm-gpui/src/main.rs", summary((50, 50), (0, 0), (2, 2))),
        ),
        repo,
    )
    lines = code_coverage.render(rows).splitlines()
    assert "| crate | lines | regions | functions |" in lines
    assert "| ngterm-core | 25.00% (50/200) | 25.00% (25/100) | 50.00% (5/10) |" in lines
    assert "| ngterm-gpui | 100.00% (50/50) | n/a | 100.00% (2/2) |" in lines
    assert lines[-1] == "| total | 40.00% (100/250) | 25.00% (25/100) | 58.33% (7/12) |"
    assert "excluded" in lines[0]


def test_no_profiles_is_refused(tmp_path: Path) -> None:
    (tmp_path / "lost-profiles.txt").write_text("1 ngterm: killed\n")
    with pytest.raises(ReportError, match="no profiles in"):
        code_coverage.check_profiles(tmp_path)
    with pytest.raises(ReportError, match="no profiles in"):
        code_coverage.check_profiles(tmp_path / "missing")


def stage_and_release(tmp_path: Path, staged: bytes, built: bytes | None) -> tuple[Path, Path]:
    stage, release = tmp_path / "stage" / "bin", tmp_path / "release"
    stage.mkdir(parents=True)
    release.mkdir()
    (stage / "ngterm").write_bytes(staged)
    (stage / "helper.txt").write_text("not a program of the build")
    if built is not None:
        (release / "ngterm").write_bytes(built)
    return stage, release


@pytest.mark.parametrize("built", [None, b"another build"], ids=["not built", "another build"])
def test_a_stage_that_is_not_the_instrumented_build_is_a_mismatch(tmp_path: Path, built: bytes | None) -> None:
    stage, release = stage_and_release(tmp_path, b"the staged build", built)
    with pytest.raises(ReportError, match=r"binary mismatch: .*stage/bin/ngterm is not .*release/ngterm"):
        code_coverage.check_binaries(stage, release)


def test_the_instrumented_builds_own_stage_passes(tmp_path: Path) -> None:
    stage, release = stage_and_release(tmp_path, b"the build", b"the build")
    code_coverage.check_binaries(stage, release)


def test_a_stage_without_the_builds_programs_is_a_mismatch(tmp_path: Path) -> None:
    (tmp_path / "bin").mkdir()
    with pytest.raises(ReportError, match="binary mismatch: no program of the build"):
        code_coverage.check_binaries(tmp_path / "bin", tmp_path)



def lcov_record(source: str, functions: dict[str, tuple[int, int]], lines: dict[int, int]) -> str:
    out = [f"SF:{source}"]
    out += [f"FN:{start},{name}" for name, (start, _count) in functions.items()]
    out += [f"FNDA:{count},{name}" for name, (_start, count) in functions.items()]
    out += [f"DA:{number},{count}" for number, count in lines.items()]
    return "\n".join([*out, "end_of_record"]) + "\n"


def regions_export(source: str, functions: list[tuple[str, int, list[tuple[int, int, int, int, int]]]]) -> str:
    """`llvm-cov export -format=text`'s functions: (name, count, code regions as start and end line/col and count)."""
    return json.dumps({"data": [{"functions": [
        {"name": name, "count": count, "filenames": [source],
         "regions": [[*span, region_count, 0, 0, code_coverage.CODE_REGION] for *span, region_count in regions]}
        for name, count, regions in functions
    ]}]})


class FakeTools:
    """rustc, llvm-profdata, llvm-cov export (each object's `lcov` and `text` from `exports`) and `cargo llvm-cov
    report --html`, as the real ones answer."""

    def __init__(
        self,
        exports: dict[str, tuple[str, str]],
        stderr: str = "",
        failing: str = "",
        truncated: frozenset[str] = frozenset(),
    ) -> None:
        self.exports, self.stderr, self.failing, self.truncated = exports, stderr, failing, truncated
        self.calls: list[list[str]] = []

    def __call__(self, command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
        self.calls.append(command)
        tool = Path(command[0]).stem
        if tool == self.failing:
            return subprocess.CompletedProcess(command, 1, "", "error: no input files\n")
        if tool == "rustc":
            return subprocess.CompletedProcess(command, 0, "/toolchain/lib/rustlib/host/lib\n", "")
        if tool == "llvm-profdata" and command[1] == "show":
            truncated = Path(command[2]).name in self.truncated
            return subprocess.CompletedProcess(command, 1 if truncated else 0, "", "malformed" if truncated else "")
        if tool == "llvm-cov":
            if Path(command[-1]).name not in self.exports:
                error = f"error: failed to load coverage: '{command[-1]}': no coverage data found\n"
                error += "error: could not load coverage information\n"
                return subprocess.CompletedProcess(command, 1, "", error)
            lcov, text = self.exports[Path(command[-1]).name]
            return subprocess.CompletedProcess(command, 0, lcov if "-format=lcov" in command else text, self.stderr)
        if "--html" in command:
            html = Path(command[command.index("--output-dir") + 1]) / "html"
            html.mkdir(parents=True, exist_ok=True)
            (html / "index.html").write_text("<html></html>")
        return subprocess.CompletedProcess(command, 0, "", "")


@pytest.fixture
def inputs(tmp_path: Path) -> tuple[Path, Path, Path]:
    profiles = tmp_path / "profiles"
    profiles.mkdir()
    (profiles / "conformance-1_0.profraw").write_bytes(b"raw")
    stage, release = stage_and_release(tmp_path, b"the build", b"the build")
    (stage / "ngtermctl").write_bytes(b"the cli")
    (release / "ngtermctl").write_bytes(b"the cli")
    return profiles, stage, release


def exports(repo: Path) -> dict[str, tuple[str, str]]:
    """`Request::new` in both programs: the app holds its unused record, the cli the run one."""
    source = str(repo / "crates/ngterm-core/src/lib.rs")
    unused = lcov_record(source, {"new": (1, 0), "main": (5, 1)}, {1: 0, 2: 0, 5: 1, 6: 1})
    run = lcov_record(source, {"new": (1, 3)}, {1: 3, 2: 3, 5: 0})
    return {
        "ngterm": (unused, regions_export(source, [("new", 0, [(1, 1, 2, 2, 0)]), ("main", 1, [(5, 1, 6, 2, 1)])])),
        "ngtermctl": (run, regions_export(source, [("new", 3, [(1, 1, 2, 2, 3)])])),
    }


def test_the_report_joins_each_programs_counts(
    tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    for override in ("LLVM_COV", "LLVM_PROFDATA"):
        monkeypatch.delenv(override, raising=False)
    tools = FakeTools(exports(repo))
    out = tmp_path / "out"
    markdown = code_coverage.report(*inputs, repo, out, runner=tools)
    # each line, region and function counts as run when either program ran it: nothing of `new` is lost to the app's
    # unused record, whichever object comes first
    assert "| ngterm-core | 100.00% (4/4) | 100.00% (2/2) | 100.00% (2/2) |" in markdown
    assert (out / "summary.md").read_text() == markdown
    lcov = (out / "lcov.info").read_text()
    assert "FNDA:3,new\n" in lcov and "DA:1,3\nDA:2,3\nDA:5,1\nDA:6,1\n" in lcov and "LH:4\n" in lcov
    assert (out / "html" / "index.html").is_file()
    exported = [command for command in tools.calls if Path(command[0]).stem == "llvm-cov"]
    assert [Path(command[-1]).name for command in exported] == ["ngterm", "ngterm", "ngtermctl", "ngtermctl"]
    for command in exported:
        assert f"-ignore-filename-regex={code_coverage.EXCLUDE}" in command
        assert command[0] == str(Path("/toolchain/lib/rustlib/host/bin/llvm-cov" +
                                      (".exe" if code_coverage.sys.platform == "win32" else "")))
    merge = next(command for command in tools.calls if Path(command[0]).stem == "llvm-profdata")
    assert merge[1:3] == ["merge", "-sparse"]
    assert (out / "profiles.txt").read_text() == f"{inputs[0] / 'conformance-1_0.profraw'}\n"


def test_an_llvm_tool_override_is_taken(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("LLVM_COV", "/opt/llvm/bin/llvm-cov")
    assert code_coverage.llvm_tool("llvm-cov", runner=FakeTools({})) == "/opt/llvm/bin/llvm-cov"


def test_join_regions_counts_code_regions_only_and_functions_by_their_start(repo: Path) -> None:
    files: dict[str, code_coverage.FileJoin] = {}
    source = str(repo / "crates/ngterm-core/src/lib.rs")
    export = json.loads(regions_export(source, [("a::<u8>", 0, [(1, 1, 3, 2, 0)]), ("a::<u16>", 2, [(1, 1, 3, 2, 2)])]))
    export["data"][0]["functions"][0]["regions"].append([2, 1, 2, 9, 0, 0, 0, 2])  # a skipped region
    code_coverage.join_regions(files, export)
    # two instantiations of one function: one function, run; one code region, run
    assert files[source].summary() == {"lines": {"count": 0, "covered": 0, "percent": 0},
                                       "regions": {"count": 1, "covered": 1, "percent": 100.0},
                                       "functions": {"count": 1, "covered": 1, "percent": 100.0}}

def test_the_exclusions_match_vendor_and_sys_crates_on_both_separators() -> None:
    for path in ("/r/vendor/gpui/src/a.rs", r"D:\r\vendor\gpui\a.rs", "/r/crates/ngterm-wpe-sys/src/lib.rs",
                 r"D:\r\crates\ngterm-ghostty-vt-sys\src\bindings.rs"):
        assert re.search(code_coverage.EXCLUDE, path), path
    for path in ("/r/crates/ngterm-core/src/lib.rs", "/r/crates/ngterm-gpui/src/sys.rs"):
        assert not re.search(code_coverage.EXCLUDE, path), path



def test_llvm_covs_mismatch_warning_is_noted_once_per_program_in_the_summary(
    tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path]
) -> None:
    # the staged build's own binaries share functions built differently: llvm-cov warns about a few of them
    tools = FakeTools({"ngterm": ("", "{}"), "ngtermctl": ("", "{}")},
                      stderr="\x1b[0;31mwarning\x1b[0m: 4 functions have mismatched data\n")
    markdown = code_coverage.report(*inputs, repo, tmp_path / "out", runner=tools)
    assert markdown.endswith(
        "| total | n/a | n/a | n/a |\n\nllvm-cov: ngterm: warning: 4 functions have mismatched data\n"
        "\nllvm-cov: ngtermctl: warning: 4 functions have mismatched data\n"
    )


@pytest.mark.parametrize("tool", ["llvm-profdata", "llvm-cov", "cargo"])
def test_a_failing_tool_names_its_exit_and_output(
    tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path], tool: str
) -> None:
    tools = FakeTools({"ngterm": ("", "{}"), "ngtermctl": ("", "{}")}, failing=tool)
    with pytest.raises(ReportError, match="exited 1: error: no input files"):
        code_coverage.report(*inputs, repo, tmp_path / "out", runner=tools)

def test_main_exits_2_with_the_reason(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "profiles").mkdir()
    status = code_coverage.main(
        ["--profiles", str(tmp_path / "profiles"), "--stage-bin", str(tmp_path), "--release-dir", str(tmp_path),
         "--repo", str(tmp_path), "--out", str(tmp_path / "out")]
    )
    assert status == 2
    assert capsys.readouterr().err.startswith(f"code coverage: no profiles in {tmp_path / 'profiles'}")
    assert not (tmp_path / "out").exists()


# The unit-test report (scripts/unit-coverage.sh).


def artifact(manifest: Path | str, kind: str, executable: str | None) -> str:
    return json.dumps({"reason": "compiler-artifact", "manifest_path": str(manifest), "target": {"kind": [kind]},
                       "executable": executable})


def build_messages(repo: Path) -> str:
    """A `cargo test --no-run --message-format=json` of the fixture: the programs, and what is not one."""
    crates = repo / "crates"
    return "\n".join([
        artifact(crates / "ngterm-core/Cargo.toml", "lib", "/t/debug/deps/ngterm_core-1a"),
        artifact(crates / "ngterm-gpui/Cargo.toml", "bin", "/t/debug/ngterm"),
        artifact(crates / "ngterm-core/Cargo.toml", "test", "/t/debug/deps/macos_only-5e"),  # empty here
        artifact(crates / "ngterm-gpui/Cargo.toml", "custom-build", "/t/debug/build/ngterm-gpui-2b/build-script-build"),
        artifact(crates / "ngterm-vt-sys/Cargo.toml", "lib", "/t/debug/deps/vt_sys-3c"),
        artifact(repo / "vendor/gpui/Cargo.toml", "lib", "/t/debug/deps/gpui-4d"),
        artifact(crates / "ngterm-core/Cargo.toml", "lib", None),  # the rlib
        json.dumps({"reason": "compiler-message", "message": {"rendered": "warning"}}),
        "{not json",
        json.dumps({"reason": "build-finished", "success": True}),
    ]) + "\n"


def test_the_built_programs_are_the_workspace_crates_test_binaries_and_programs(repo: Path) -> None:
    assert code_coverage.built_programs(build_messages(repo), repo) == [
        Path("/t/debug/deps/macos_only-5e"), Path("/t/debug/deps/ngterm_core-1a"), Path("/t/debug/ngterm")
    ]


# ngterm-core's lib.rs: two production functions, an inline test module and a test-only module file
CORE_LIB = """pub fn used() -> u8 {
    1
}
pub fn unused() {}
#[cfg(test)]
mod tests {
    #[test]
    fn runs() {
        assert_eq!(super::used(), 1);
    }
}
#[cfg(test)]
mod helpers;
"""


def unit_exports(repo: Path) -> dict[str, tuple[str, str]]:
    lib, helpers = str(repo / "crates/ngterm-core/src/lib.rs"), str(repo / "crates/ngterm-core/src/helpers.rs")
    main = str(repo / "crates/ngterm-gpui/src/main.rs")
    core_lcov = lcov_record(
        lib, {"used": (1, 1), "unused": (4, 0), "runs": (8, 1)}, {1: 1, 2: 1, 3: 1, 4: 0, 8: 1, 9: 1, 10: 1}
    ) + lcov_record(helpers, {"help": (1, 1)}, {1: 1})
    core_text = json.loads(regions_export(lib, [("used", 1, [(1, 1, 3, 2, 1)]), ("unused", 0, [(4, 1, 4, 19, 0)]),
                                                ("runs", 1, [(8, 1, 10, 6, 1)])]))
    helpers_text = json.loads(regions_export(helpers, [("help", 1, [(1, 1, 1, 16, 1)])]))
    core_text["data"][0]["functions"] += helpers_text["data"][0]["functions"]
    # an integration test's own function: llvm-cov's -ignore-filename-regex leaves its regions in the text export
    test_file = str(repo / "crates/ngterm-core/tests/api.rs")
    core_text["data"][0]["functions"] += json.loads(regions_export(test_file, [("api", 1, [(1, 1, 2, 2, 1)])]))[
        "data"][0]["functions"]
    return {
        "ngterm_core-1a": (core_lcov, json.dumps(core_text)),
        "ngterm": (
            lcov_record(main, {"main": (1, 1)}, {1: 1, 2: 0}), regions_export(main, [("main", 1, [(1, 1, 2, 2, 1)])])
        ),
    }


@pytest.fixture
def unit_inputs(tmp_path: Path, repo: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path]:
    for override in ("LLVM_COV", "LLVM_PROFDATA"):
        monkeypatch.delenv(override, raising=False)
    (repo / "crates/ngterm-core/src").mkdir(parents=True)
    (repo / "crates/ngterm-core/src/lib.rs").write_text(CORE_LIB)
    (repo / "crates/ngterm-core/src/helpers.rs").write_text("pub fn help() {}\n")
    (repo / "crates/ngterm-gpui/src").mkdir(parents=True)
    (repo / "crates/ngterm-gpui/src/main.rs").write_text("fn main() {\n}\n")
    messages = tmp_path / "build.jsonl"
    messages.write_text(build_messages(repo))
    profiles = tmp_path / "llvm-cov-target"
    profiles.mkdir()
    for name in ("ws-1.profraw", "ws-2.profraw"):
        (profiles / name).write_bytes(b"raw")
    return messages, profiles


def test_the_unit_report_leaves_test_code_out_of_every_metric(
    tmp_path: Path, repo: Path, unit_inputs: tuple[Path, Path]
) -> None:
    messages, profiles = unit_inputs
    tools = FakeTools(unit_exports(repo), truncated=frozenset({"ws-2.profraw"}))
    out = tmp_path / "out"
    markdown = code_coverage.unit_report(messages, profiles, repo, out, runner=tools)
    # lib.rs keeps lines 1-4, `used` and `unused` and their regions; the test module and helpers.rs are gone
    assert markdown.startswith(code_coverage.UNIT_INTRO + "\n")
    assert "| ngterm-core | 75.00% (3/4) | 50.00% (1/2) | 50.00% (1/2) |" in markdown
    assert "| ngterm-gpui | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) |" in markdown
    assert "| total | 66.67% (4/6) | 66.67% (2/3) | 66.67% (2/3) |" in markdown
    assert "llvm-cov: 1 truncated profile(s) of killed test processes dropped" in markdown
    assert (out / "summary.md").read_text() == markdown
    lcov = (out / "lcov.info").read_text()
    assert "helpers.rs" not in lcov and "runs" not in lcov and "DA:8," not in lcov
    assert "tests/api.rs" not in (out / "summary-files.md").read_text()
    assert "FN:4,unused\n" in lcov and "DA:4,0\nLF:4\nLH:3\n" in lcov
    files = (out / "summary-files.md").read_text().splitlines()
    assert files[2:] == [
        "| file | lines | functions | regions | uncovered lines |",
        "|---|---:|---:|---:|---:|",
        "| crates/ngterm-core/src/lib.rs | 75.00% (3/4) | 50.00% (1/2) | 50.00% (1/2) | 1 |",
        "| crates/ngterm-gpui/src/main.rs | 50.00% (1/2) | 100.00% (1/1) | 100.00% (1/1) | 1 |",
    ]
    summary = json.loads((out / "summary.json").read_text())["data"][0]["files"]
    assert [entry["summary"]["lines"] for entry in summary][0] == {"count": 4, "covered": 3, "percent": 75.0}
    # the build's programs only, each exported on its own, against the readable profiles alone; one with no
    # coverage map (a test compiled empty on this OS) has nothing to join
    exported = [Path(command[-1]).name for command in tools.calls if Path(command[0]).stem == "llvm-cov"]
    assert exported == ["macos_only-5e", "ngterm_core-1a", "ngterm_core-1a", "ngterm", "ngterm"]
    assert (profiles / "profiles.txt").read_text() == f"{profiles / 'ws-1.profraw'}\n"
    for command in tools.calls:
        if Path(command[0]).stem == "llvm-cov":
            assert f"-ignore-filename-regex={code_coverage.UNIT_EXCLUDE}" in command


def test_the_unit_report_of_selected_crates_lists_their_files_alone(
    tmp_path: Path, repo: Path, unit_inputs: tuple[Path, Path]
) -> None:
    out = tmp_path / "out"
    tools = FakeTools(unit_exports(repo))
    markdown = code_coverage.unit_report(*unit_inputs, repo, out, ["ngterm-gpui"], runner=tools)
    assert "| ngterm-gpui |" in markdown and "ngterm-core" not in markdown
    assert "ngterm-core" not in (out / "lcov.info").read_text()
    assert "ngterm-core" not in (out / "summary-files.md").read_text()


@pytest.mark.parametrize(
    "path", ["/r/crates/a/tests/cli.rs", "/r/crates/a/src/tests.rs", "/r/crates/a/src/model_tests.rs",
             "/r/vendor/gpui/src/a.rs", "/r/crates/ngterm-wpe-sys/src/lib.rs"]
)
def test_the_unit_exclusions_add_test_files_by_name(path: str) -> None:
    assert re.search(code_coverage.UNIT_EXCLUDE, path)
    assert not re.search(code_coverage.UNIT_EXCLUDE, "/r/crates/a/src/contests.rs")


def test_unit_main_exits_2_when_the_build_built_no_program(
    tmp_path: Path, repo: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    messages = tmp_path / "build.jsonl"
    messages.write_text(json.dumps({"reason": "build-finished", "success": True}) + "\n")
    status = code_coverage.main(["unit", "--messages", str(messages), "--profiles", str(tmp_path), "--repo",
                                 str(repo), "--out", str(tmp_path / "out"), "--crate", "ngterm-core"])
    assert status == 2
    assert capsys.readouterr().err.startswith("code coverage: no program of the workspace's crates")
    assert not (tmp_path / "out").exists()
