"""The per-app e2e coverage report (`code_coverage.py e2e`): the app's own programs, lcov shards merged, the exclusion
markers applied and listed, and the inputs it refuses."""

import json
import re
import tomllib
from pathlib import Path

import pytest
from agterm_conformance import code_coverage
from agterm_conformance.code_coverage import ReportError
from test_code_coverage import FakeTools, lcov_record

REPO = Path(__file__).resolve().parents[2]

APPS = """
[desktop.linux]
binaries = ["ngterm", "ngtermctl", "ngterm-session"]

[headless.linux]
binaries = ["ngterm-headless", "ngtermctl", "ngterm-session"]

[android.emulator]
binaries = ["libngterm_mobile.so"]
modules = { android-app = "mobile/android/app/src/main/" }
exclude = ["/generated/"]
"""

CORE = """pub fn used() -> u8 {
    1
}
// e2e-coverage: off: runs only when the OS kills the process
#[cold]
pub fn on_kill() {
    let _ = 1;
}
pub fn unused() {}
"""


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    root = tmp_path / "repo"
    for directory in ("ngterm-core", "ngterm-ctl", "ngterm-headless"):
        (root / "crates" / directory / "src").mkdir(parents=True)
        (root / "crates" / directory / "Cargo.toml").write_text(f'[package]\nname = "{directory}"\n')
    (root / "crates/ngterm-core/src/lib.rs").write_text(CORE)
    (root / "crates/ngterm-ctl/src/main.rs").write_text("fn main() {\n}\n")
    (root / "crates/ngterm-headless/src/main.rs").write_text("fn main() {\n}\n")
    (root / "conformance/coverage").mkdir(parents=True)
    (root / "conformance/coverage/apps.toml").write_text(APPS)
    return root


def core_record(repo: Path, hits: dict[int, int]) -> str:
    lines = {1: 0, 2: 0, 3: 0, 6: 0, 7: 0, 8: 0, 9: 0} | hits
    functions = {"used": (1, lines[1]), "on_kill": (6, lines[6]), "unused": (9, lines[9])}
    return lcov_record(str(repo / "crates/ngterm-core/src/lib.rs"), functions, lines)


def main_record(repo: Path, crate: str, hit: int) -> str:
    return lcov_record(str(repo / f"crates/{crate}/src/main.rs"), {"main": (1, hit)}, {1: hit, 2: hit})


@pytest.fixture
def rust_inputs(tmp_path: Path, repo: Path, monkeypatch: pytest.MonkeyPatch) -> tuple[Path, Path, Path]:
    for override in ("LLVM_COV", "LLVM_PROFDATA"):
        monkeypatch.delenv(override, raising=False)
    profiles, stage, release = tmp_path / "profiles", tmp_path / "stage/bin", tmp_path / "release"
    for directory in (profiles, stage, release):
        directory.mkdir(parents=True)
    (profiles / "conformance-1_0.profraw").write_bytes(b"raw")
    for name in ("ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"):
        (stage / name).write_bytes(name.encode())
        (release / name).write_bytes(name.encode())
    return profiles, stage, release


def exports(repo: Path) -> dict[str, tuple[str, str]]:
    """Each program's lcov: the headless one ran `used`, ngtermctl its main, the desktop app all of ngterm-core."""
    return {
        "ngterm": (core_record(repo, {1: 9, 2: 9, 3: 9, 6: 9, 7: 9, 8: 9, 9: 9}), "{}"),
        "ngterm-headless": (core_record(repo, {1: 1, 2: 1, 3: 1}) + main_record(repo, "ngterm-headless", 1), "{}"),
        "ngtermctl": (main_record(repo, "ngterm-ctl", 2), "{}"),
        "ngterm-session": ("", "{}"),
    }


def run_e2e(repo: Path, out: Path, app: str = "headless", platform: str = "linux", *, profiles=None, lcov=(),
            tools=None) -> str:
    rust = []
    if profiles is not None:
        profile_dir, stage, release = profiles
        rust = [code_coverage.RustInput(profile_dir, release, stage)]
    return code_coverage.e2e_report(app, platform, repo, out, rust=rust, lcov=list(lcov),
                                    runner=tools or FakeTools(exports(repo)))


def test_only_the_apps_own_programs_are_read(tmp_path, repo, rust_inputs):
    tools = FakeTools(exports(repo))
    markdown = run_e2e(repo, tmp_path / "out", profiles=rust_inputs, tools=tools)
    exported = [Path(command[-1]).name for command in tools.calls if Path(command[0]).stem == "llvm-cov"]
    # the desktop app's ngterm, which ran every line of ngterm-core, is not read; lcov only, no region export
    assert sorted(exported) == ["ngterm-headless", "ngterm-session", "ngtermctl"]
    assert all("-format=lcov" in command for command in tools.calls if Path(command[0]).stem == "llvm-cov")
    # ngterm-core: 3 of its 4 measured lines (on_kill's 3 lines are excluded by its marker)
    assert "| ngterm-core | 75.00% (3/4) | 42.86% (3/7) |" in markdown
    assert "| ngterm-ctl | 100.00% (2/2) | 0.00% (0/2) |" in markdown
    assert "| ngterm-headless | 100.00% (2/2) | 0.00% (0/2) |" in markdown
    assert "| total | 87.50% (7/8) | 27.27% (3/11) |" in markdown


def test_the_summary_gives_the_total_its_distance_to_90_and_the_excluded_share(tmp_path, repo, rust_inputs):
    markdown = run_e2e(repo, tmp_path / "out", profiles=rust_inputs)
    assert markdown.startswith("E2E line coverage of the `headless` app on `linux`")
    assert "ngterm-headless, ngtermctl, ngterm-session" in markdown.splitlines()[0]
    assert ("**Total: 87.50% of lines (7/8), 2.50 pt to 90%. Excluded by markers: 27.27% of lines (3/11).**"
            in markdown)


def test_the_marker_list_names_each_markers_file_line_scope_and_reason(tmp_path, repo, rust_inputs):
    markdown = run_e2e(repo, tmp_path / "out", profiles=rust_inputs)
    assert "### Exclusion markers" in markdown
    assert "| file | line | excludes | lines | reason |" in markdown
    assert "| crates/ngterm-core/src/lib.rs | 4 | fn `on_kill` | 3 | runs only when the OS kills the process |" in (
        markdown)


def test_the_outputs_keep_the_apps_data_and_the_measured_data_apart(tmp_path, repo, rust_inputs):
    out = tmp_path / "out"
    markdown = run_e2e(repo, out, profiles=rust_inputs)
    assert (out / "summary.md").read_text() == markdown
    # lcov.info: the app's data before the markers, with paths relative to the repo (what a later merge takes)
    full = (out / "lcov.info").read_text()
    assert "SF:crates/ngterm-core/src/lib.rs\n" in full and "DA:7,0\n" in full and str(repo) not in full
    measured = (out / "lcov-measured.info").read_text()
    assert "DA:7," not in measured and "on_kill" not in measured and "DA:9,0\n" in measured
    summary = json.loads((out / "summary.json").read_text())
    assert (summary["app"], summary["platform"]) == ("headless", "linux")
    assert summary["total"]["lines"] == {"count": 8, "covered": 7}
    assert summary["excluded"] == {"lines": 3, "share": pytest.approx(300 / 11)}
    assert summary["rows"]["ngterm-core"] == {"lines": {"count": 4, "covered": 3}, "excluded": 3}
    assert summary["markers"] == [{
        "path": "crates/ngterm-core/src/lib.rs", "line": 4, "scope": "function", "name": "on_kill",
        "reason": "runs only when the OS kills the process", "lines": 3,
    }]


def test_two_lcov_shards_merge_a_line_hit_in_either_counting(tmp_path, repo):
    shards = []
    for number, hits in enumerate(({1: 1, 2: 1}, {3: 4, 9: 1})):
        shard = tmp_path / f"shard-{number}.info"
        # the shards' lcov.info have relative paths, as the report writes them
        shard.write_text(core_record(repo, hits).replace(str(repo) + "/", ""))
        shards.append(shard)
    markdown = run_e2e(repo, tmp_path / "out", lcov=shards)
    assert "| ngterm-core | 100.00% (4/4) | 42.86% (3/7) |" in markdown
    assert "DA:3,4\n" in (tmp_path / "out/lcov.info").read_text()


def test_a_report_merged_from_reports_keeps_the_excluded_share(tmp_path, repo, rust_inputs):
    first = tmp_path / "first"
    run_e2e(repo, first, profiles=rust_inputs)
    markdown = run_e2e(repo, tmp_path / "merged", lcov=[first / "lcov.info", first / "lcov.info"])
    assert "Excluded by markers: 27.27% of lines (3/11)" in markdown


def test_a_file_marker_excludes_the_whole_file(tmp_path, repo):
    source = repo / "crates/ngterm-ctl/src/main.rs"
    source.write_text("// e2e-coverage: off-file: the macOS app delegate's callbacks\nfn main() {\n}\n")
    shard = tmp_path / "shard.info"
    shard.write_text(main_record(repo, "ngterm-ctl", 0) + core_record(repo, {1: 1}))
    markdown = run_e2e(repo, tmp_path / "out", lcov=[shard])
    assert "| crates/ngterm-ctl/src/main.rs | 1 | the file | 2 | the macOS app delegate's callbacks |" in markdown
    # a crate with every line excluded keeps its row, with no lines measured
    assert "| ngterm-ctl | n/a | 100.00% (2/2) |" in markdown


def test_a_marked_function_this_build_has_no_record_of_is_listed_as_such(tmp_path, repo):
    shard = tmp_path / "shard.info"
    # a platform whose build compiles `on_kill` out: no record of lines 6-8
    shard.write_text(lcov_record(str(repo / "crates/ngterm-core/src/lib.rs"), {"used": (1, 1)}, {1: 1, 2: 1, 3: 1}))
    markdown = run_e2e(repo, tmp_path / "out", lcov=[shard])
    assert "| crates/ngterm-core/src/lib.rs | 4 | fn `on_kill` (not in this build) | 0 |" in markdown
    assert "Excluded by markers: 0.00% of lines (0/3)" in markdown


def test_modules_group_by_path_and_the_apps_exclusions_leave_files_out(tmp_path, repo):
    kotlin = repo / "mobile/android/app/src/main/java/A.kt"
    kotlin.parent.mkdir(parents=True)
    kotlin.write_text("class A {\n    fun a() {\n    }\n}\n")
    shard = tmp_path / "kotlin.info"
    shard.write_text(
        lcov_record("mobile/android/app/src/main/java/A.kt", {"a": (2, 1)}, {2: 1, 3: 0})
        + lcov_record("mobile/android/app/src/main/generated/B.kt", {"b": (1, 0)}, {1: 0})
        + lcov_record("/rustc/abc/library/core/src/lib.rs", {"c": (1, 0)}, {1: 0})
    )
    markdown = run_e2e(repo, tmp_path / "out", "android", "emulator", lcov=[shard])
    assert "| android-app | 50.00% (1/2) | 0.00% (0/2) |" in markdown
    assert "generated" not in (tmp_path / "out/lcov.info").read_text()
    assert "rustc" not in (tmp_path / "out/lcov.info").read_text()


def test_the_distance_to_90_reads_at_or_above_once_there(tmp_path, repo):
    shard = tmp_path / "shard.info"
    shard.write_text(main_record(repo, "ngterm-ctl", 1))
    assert "100.00% of lines (2/2), at or above 90%." in run_e2e(repo, tmp_path / "out", lcov=[shard])


@pytest.mark.parametrize(
    "source, message",
    [
        ("// e2e-coverage: off:\nfn main() {\n}\n", "crates/ngterm-ctl/src/main.rs:1: a marker with no reason"),
        ("// e2e-coverage: off: why\nstruct A;\nfn main() {\n}\n",
         "crates/ngterm-ctl/src/main.rs:1: no function directly below the marker"),
    ],
)
def test_a_bad_marker_exits_2_naming_it(tmp_path, repo, capsys, source, message):
    (repo / "crates/ngterm-ctl/src/main.rs").write_text(source)
    shard = tmp_path / "shard.info"
    shard.write_text(main_record(repo, "ngterm-ctl", 1))
    status = code_coverage.main(["e2e", "--app", "headless", "--platform", "linux", "--repo", str(repo),
                                 "--out", str(tmp_path / "out"), "--lcov", str(shard)])
    assert status == 2
    assert message in capsys.readouterr().err
    assert not (tmp_path / "out").exists()


def missing_cases(tmp_path: Path, repo: Path) -> dict[str, tuple[list[str], str]]:
    shard = tmp_path / "shard.info"
    shard.write_text(main_record(repo, "ngterm-ctl", 1))
    release = tmp_path / "release"
    release.mkdir(exist_ok=True)
    (release / "ngterm-headless").write_bytes(b"x")
    profiles = tmp_path / "profiles"
    profiles.mkdir(exist_ok=True)
    return {
        "no input": ([], "no input: give --profiles with --release-dir, or --lcov"),
        "missing lcov": (["--lcov", str(tmp_path / "gone.info")], "cannot read the lcov file"),
        "unknown app": (["--lcov", str(shard), "--app", "tui"], "no app `tui` on `linux` in"),
        "no profiles": (["--profiles", str(profiles), "--release-dir", str(release)], "no profiles in"),
        "profiles without a build": (["--profiles", str(profiles)], "--profiles needs --release-dir"),
        "missing program": (
            ["--profiles", str(shard.parent), "--release-dir", str(release)],
            "the headless app's program ngtermctl is not in",
        ),
        "missing apps file": (["--lcov", str(shard), "--repo", str(tmp_path)], "cannot read the apps file"),
        "nothing measured": (
            ["--lcov", str(tmp_path / "empty.info")],
            "no line of the headless app's code in the inputs",
        ),
    }


@pytest.mark.parametrize("case", ["no input", "missing lcov", "unknown app", "no profiles", "profiles without a build",
                                  "missing program", "missing apps file", "nothing measured"])
def test_a_missing_input_exits_2(tmp_path, repo, capsys, monkeypatch, case):
    for override in ("LLVM_COV", "LLVM_PROFDATA"):
        monkeypatch.delenv(override, raising=False)
    (tmp_path / "empty.info").write_text("")
    (tmp_path / "x.profraw").write_bytes(b"raw")
    args, message = missing_cases(tmp_path, repo)[case]
    base = {"--app": "headless", "--platform": "linux", "--repo": str(repo), "--out": str(tmp_path / "out")}
    for option in ("--app", "--repo"):
        if option in args:
            base[option] = args.pop(args.index(option) + 1)
            args.remove(option)
    argv = ["e2e", *[part for pair in base.items() for part in pair], *args]
    assert code_coverage.main(argv) == 2
    assert message in capsys.readouterr().err


def test_a_measured_source_missing_from_the_checkout_exits_2(tmp_path, repo):
    shard = tmp_path / "shard.info"
    shard.write_text(lcov_record("crates/ngterm-core/src/gone.rs", {"a": (1, 1)}, {1: 1}))
    with pytest.raises(ReportError, match="crates/ngterm-core/src/gone.rs: a measured source is not in the checkout"):
        run_e2e(repo, tmp_path / "out", lcov=[shard])


def test_a_stage_that_is_not_the_build_is_refused(tmp_path, repo, rust_inputs):
    profiles, stage, release = rust_inputs
    (stage / "ngtermctl").write_bytes(b"another build")
    with pytest.raises(ReportError, match="binary mismatch: .*ngtermctl"):
        run_e2e(repo, tmp_path / "out", profiles=rust_inputs)


def test_a_windows_program_is_found_by_its_exe(tmp_path, repo, rust_inputs):
    profiles, stage, release = rust_inputs
    for name in ("ngtermctl", "ngterm-session", "ngterm-headless"):
        (release / name).rename(release / f"{name}.exe")
        (stage / name).rename(stage / f"{name}.exe")
    tools = FakeTools({f"{name}.exe": export for name, export in exports(repo).items()})
    assert "| total | 87.50% (7/8) |" in run_e2e(repo, tmp_path / "out", profiles=rust_inputs, tools=tools)


@pytest.mark.parametrize(
    "apps, message",
    [
        ("[headless.linux]\nbinaries = [1]\n", "binaries: not a list of names"),
        ("[headless.linux]\nbinary = []\n", "unknown key binary"),
        ("[headless.linux]\nmodules = { a = 1 }\n", "modules: not a table of path prefixes"),
        ("[headless.linux]\nexclude = ['(']\n", "exclude: not a regular expression"),
        ("[headless]\nlinux = 1\n", "not a table"),
        ("[headless.linux\n", "not TOML"),
    ],
)
def test_a_malformed_apps_file_is_refused(tmp_path, repo, apps, message):
    (repo / "conformance/coverage/apps.toml").write_text(apps)
    shard = tmp_path / "shard.info"
    shard.write_text(main_record(repo, "ngterm-ctl", 1))
    with pytest.raises(ReportError, match=re.escape(message)):
        run_e2e(repo, tmp_path / "out", lcov=[shard])


def test_the_checked_in_apps_file_names_programs_the_workspace_builds():
    apps = code_coverage.load_apps(REPO / code_coverage.APPS_FILE)
    programs = set()
    for manifest in (REPO / "crates").glob("*/Cargo.toml"):
        package = tomllib.loads(manifest.read_text())
        programs |= {target["name"] for target in package.get("bin", [])}
        programs |= {path.stem for path in (manifest.parent / "src/bin").glob("*.rs")}
    assert {"ngterm", "ngtermctl", "ngterm-session", "ngterm-headless", "ngterm-web"} <= programs
    for (app, platform), build in apps.items():
        assert build.binaries, f"{app}.{platform} names no program"
        assert set(build.binaries) <= programs, f"{app}.{platform}: {set(build.binaries) - programs}"
    assert apps["desktop", "linux"].binaries == ("ngterm", "ngtermctl", "ngterm-session", "ngterm-web")
    assert "ngterm-headless" not in apps["desktop", "linux"].binaries
    assert apps["headless", "linux"].binaries == ("ngterm-headless", "ngtermctl", "ngterm-session")
