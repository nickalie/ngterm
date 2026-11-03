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


class FakeCargo:
    """`cargo llvm-cov report`: writes what each format asks for, as the real one does."""

    def __init__(self, export: dict[str, Any], stderr: str = "", returncode: int = 0) -> None:
        self.export, self.stderr, self.returncode = export, stderr, returncode
        self.calls: list[list[str]] = []

    def __call__(self, command: list[str], **kwargs: Any) -> subprocess.CompletedProcess[str]:
        self.calls.append(command)
        if self.returncode == 0:
            if "--output-path" in command:
                target = Path(command[command.index("--output-path") + 1])
                target.write_text(json.dumps(self.export) if "--json" in command else "TN:\n")
            if "--html" in command:
                html = Path(command[command.index("--output-dir") + 1]) / "html"
                html.mkdir(parents=True, exist_ok=True)
                (html / "index.html").write_text("<html></html>")
        return subprocess.CompletedProcess(command, self.returncode, "", self.stderr)


@pytest.fixture
def inputs(tmp_path: Path) -> tuple[Path, Path, Path]:
    profiles = tmp_path / "profiles"
    profiles.mkdir()
    (profiles / "conformance-1_0.profraw").write_bytes(b"raw")
    stage, release = stage_and_release(tmp_path, b"the build", b"the build")
    return profiles, stage, release


def test_the_report_writes_lcov_html_and_the_summary(
    tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path]
) -> None:
    cargo = FakeCargo(export(repo, ("crates/ngterm-core/src/lib.rs", summary((4, 1)))))
    out = tmp_path / "out"
    markdown = code_coverage.report(*inputs, repo, out, runner=cargo)
    assert "| ngterm-core | 25.00% (1/4) |" in markdown
    assert (out / "summary.md").read_text() == markdown
    assert (out / "lcov.info").is_file() and (out / "html" / "index.html").is_file()
    for command in cargo.calls:
        assert command[:6] == ["cargo", "llvm-cov", "report", "--release", "--color", "never"]
        assert command[command.index("--ignore-filename-regex") + 1] == code_coverage.EXCLUDE


def test_the_exclusions_match_vendor_and_sys_crates_on_both_separators() -> None:
    for path in ("/r/vendor/gpui/src/a.rs", r"D:\r\vendor\gpui\a.rs", "/r/crates/ngterm-wpe-sys/src/lib.rs",
                 r"D:\r\crates\ngterm-ghostty-vt-sys\src\bindings.rs"):
        assert re.search(code_coverage.EXCLUDE, path), path
    for path in ("/r/crates/ngterm-core/src/lib.rs", "/r/crates/ngterm-gpui/src/sys.rs"):
        assert not re.search(code_coverage.EXCLUDE, path), path


def test_llvm_covs_mismatch_warning_is_noted_once_in_the_summary(
    tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path]
) -> None:
    # the staged build's own binaries share functions built differently: llvm-cov warns about a few of them
    cargo = FakeCargo(export(repo), stderr="\x1b[0;31mwarning\x1b[0m: 4 functions have mismatched data\n")
    markdown = code_coverage.report(*inputs, repo, tmp_path / "out", runner=cargo)
    assert markdown.endswith("| total | n/a | n/a | n/a |\n\nllvm-cov: warning: 4 functions have mismatched data\n")


def test_a_failing_cargo_names_its_exit_and_output(tmp_path: Path, repo: Path, inputs: tuple[Path, Path, Path]) -> None:
    cargo = FakeCargo({}, stderr="error: no input files\n", returncode=1)
    with pytest.raises(ReportError, match="exited 1: error: no input files"):
        code_coverage.report(*inputs, repo, tmp_path / "out", runner=cargo)


def test_main_exits_2_with_the_reason(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    (tmp_path / "profiles").mkdir()
    status = code_coverage.main(
        ["--profiles", str(tmp_path / "profiles"), "--stage-bin", str(tmp_path), "--release-dir", str(tmp_path),
         "--repo", str(tmp_path), "--out", str(tmp_path / "out")]
    )
    assert status == 2
    assert capsys.readouterr().err.startswith(f"code coverage: no profiles in {tmp_path / 'profiles'}")
    assert not (tmp_path / "out").exists()
