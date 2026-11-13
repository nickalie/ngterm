"""scripts/test-mobile-e2e.sh, the headless app's e2e runner: its plain run and its `--coverage` run (the instrumented
build and stage, the pooled profiles the tests' processes get, the headless app's other e2e on the same build, the
report and the exit codes). Each case runs the real script over a copy of it, its library and the pins, with stand-in
cargo, stage.sh, uname and Python; nothing is built and no test or profile is real."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = "scripts/test-mobile-e2e.sh"
WRAPPER = "/stand-in/cargo-llvm-cov"

# Logs each call with the environment the script gave it; `llvm-cov show-env` prints an instrumenting environment.
CARGO = f'''
import json, os, sys
args = sys.argv[1:]
keys = ("RUSTC_WRAPPER", "CARGO_TARGET_DIR", "LLVM_PROFILE_FILE", "E2E_BIN_DIR", "MOBILE_E2E_STAGE",
        "NGTERM_CALLER")
env = {{key: os.environ[key] for key in keys if key in os.environ}}
pool = os.environ.get("LLVM_PROFILE_FILE")
if pool:
    env["pool"] = sorted(os.listdir(os.path.dirname(pool)))
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps(["cargo", args, env]) + "\\n")
if args[:2] == ["llvm-cov", "show-env"]:
    print("export RUSTC_WRAPPER={WRAPPER}")
elif args[0] == "build" and "RUSTC_WRAPPER" in os.environ:
    sys.exit(int(os.environ.get("BUILD_STATUS", "0")))
elif args[0] == "test":
    if "LIVE_DIR" in os.environ:
        with open(os.path.join(os.environ["MOBILE_E2E_WORK"], "live-dir"), "w") as live:
            live.write(os.environ["LIVE_DIR"])
    failing = os.environ.get("FAILING_TEST")
    sys.exit(101 if failing and failing in args else 0)
'''

# The staged ngterm-session, as the exit trap's reap runs it (env -i: the log's path is written in): logs its call and
# its profile path, and lists one session.
SESSION = '''
import json, os, sys
with open({log!r}, "a") as log:
    log.write(json.dumps(["ngterm-session", sys.argv[1:], {{"LLVM_PROFILE_FILE": os.environ.get("LLVM_PROFILE_FILE")}}]) + "\\n")
if sys.argv[1:] == ["list", "--short"]:
    print("left")
'''

PYTHON = '''
import json, os, sys
keys = ("RUSTC_WRAPPER", "CARGO_TARGET_DIR", "PYTHONPATH")
env = {key: os.environ[key] for key in keys if key in os.environ}
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps([os.path.basename(sys.argv[0]), sys.argv[1:], env]) + "\\n")
if sys.argv[1] == "-c":
    sys.exit(1 if os.path.basename(sys.argv[0]) in os.environ.get("WITHOUT_TOMLLIB", "").split() else 0)
sys.exit(int(os.environ.get("REPORT_STATUS", "0")))
'''

# scripts/stage.sh: the destination's bin/, and the call logged with the build it stages from
STAGE = '''#!/usr/bin/env bash
mkdir -p "$1/bin"
[[ -z "${STAGE_SESSION:-}" ]] || cp "$STAGE_SESSION" "$1/bin/ngterm-session"
printf '["stage", ["%s", "%s", "%s"], {"CARGO_TARGET_DIR": "%s"}]\\n' "$1" "$2" "$3" "$CARGO_TARGET_DIR" >>"$TOOL_LOG"
'''


class Checkout:
    """The script, its library and the pins as a repository of their own, with stand-in tools first on PATH."""

    def __init__(self, root: Path) -> None:
        self.root = root
        for name in (SCRIPT, "scripts/lib/native.sh", "build/pins.env"):
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO / name, destination)
        stage = root / "scripts/stage.sh"
        stage.write_text(STAGE)
        stage.chmod(0o755)
        tests = root / "crates/ngterm-headless/tests"
        tests.mkdir(parents=True)
        for name in ("lifecycle.rs", "agent_hooks.rs"):
            (tests / name).write_text("")
        self.target = root / "target"
        self.cov = self.target / "llvm-cov-target"
        self.bin = root / "bin"
        self.bin.mkdir()
        self.log = root / "calls.jsonl"
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("NGTERM_", "AGTERM_", "LLVM_", "CARGO_LLVM_COV", "__CARGO_LLVM_COV"))
            and key not in ("PYTHON", "CARGO_TARGET_DIR", "RUSTC_WRAPPER", "MOBILE_E2E_WORK_ROOT", "CI")
        }
        self.env.update(
            PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            TOOL_LOG=str(self.log),
            NGTERM_CALLER="the caller's",
        )
        self.tool("uname", 'echo Linux\n', shell=True)
        self.tool("cargo", CARGO)
        for name in ("python3", "python", "python3.13", "python3.12", "python3.11"):
            self.tool(name, PYTHON)

    def tool(self, name: str, body: str, shell: bool = False) -> Path:
        path = self.bin / name
        path.write_text(("#!/usr/bin/env bash\n" if shell else f"#!{sys.executable}\n") + body)
        path.chmod(0o755)
        return path

    def run(self, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(self.root / SCRIPT), *args],
            env={**self.env, **env},
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    def calls(self) -> list[list]:
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text().splitlines()]

    def cargo(self, command: str) -> list[tuple[list[str], dict]]:
        return [(args, env) for tool, args, env in self.calls() if tool == "cargo" and args[0] == command]

    def report(self) -> list[tuple[list[str], dict]]:
        return [(args, env) for tool, args, env in self.calls() if tool.startswith("python") and args[0] == "-m"]


@pytest.fixture
def checkout(tmp_path: Path) -> Checkout:
    return Checkout(tmp_path / "repo")


def test_a_plain_run_builds_stages_and_runs_the_mobile_e2e_alone(checkout: Checkout) -> None:
    result = checkout.run()
    assert result.returncode == 0, result.stderr
    assert result.stderr == "→ mobile e2e passed\n"
    [(build, build_env)] = checkout.cargo("build")
    assert build == ["build", "-p", "ngterm-headless", "-p", "ngterm-ctl", "-p", "ngterm-session"]
    assert "RUSTC_WRAPPER" not in build_env
    [stage] = [args for tool, args, env in checkout.calls() if tool == "stage"]
    assert stage[1:] == ["--profile", "debug"]
    assert [env for tool, args, env in checkout.calls() if tool == "stage"] == [{"CARGO_TARGET_DIR": str(checkout.target)}]
    [(test, test_env)] = checkout.cargo("test")
    assert test == ["test", "-p", "ngterm-mobile", "--test", "headless_e2e", "--", "--ignored", "--nocapture"]
    assert test_env["MOBILE_E2E_STAGE"] == f"{stage[0]}"
    assert not {"LLVM_PROFILE_FILE", "E2E_BIN_DIR", "NGTERM_CALLER"} & set(test_env)
    assert checkout.report() == []
    assert not (checkout.root / "target/llvm-cov-target").exists()


def test_a_coverage_run_measures_the_instrumented_stage_under_every_headless_e2e(checkout: Checkout) -> None:
    pool = checkout.cov / "headless-e2e"
    pool.mkdir(parents=True)
    (pool / "headless-stale.profraw").write_text("an earlier run's")
    result = checkout.run("--coverage", "report")
    assert result.returncode == 0, result.stderr
    built = checkout.cov / "debug"

    [(build, build_env)] = checkout.cargo("build")
    assert build == ["build", "-p", "ngterm-headless", "-p", "ngterm-ctl", "-p", "ngterm-session"]
    assert build_env == {"RUSTC_WRAPPER": WRAPPER, "CARGO_TARGET_DIR": str(checkout.cov), "NGTERM_CALLER": "the caller's"}
    [stage] = [(args, env) for tool, args, env in checkout.calls() if tool == "stage"]
    assert stage[1] == {"CARGO_TARGET_DIR": str(checkout.cov)}

    tests = checkout.cargo("test")
    assert [args for args, _ in tests] == [
        ["test", "-p", "ngterm-mobile", "--test", "headless_e2e", "--", "--ignored", "--nocapture"],
        # every target runs even when an earlier one fails
        ["test", "-p", "ngterm-headless", "--no-fail-fast", "--test", "agent_hooks", "--test", "lifecycle"],
        ["test", "-p", "ngterm-session", "--test", "e2e"],
    ]
    for _, env in tests:
        # built as usual: the tests' own processes write no profile, only the stage's programs they start do
        assert "RUSTC_WRAPPER" not in env and "CARGO_TARGET_DIR" not in env and "NGTERM_CALLER" not in env
        assert env["LLVM_PROFILE_FILE"] == f"{pool}/headless-%4m.profraw"
        assert env["E2E_BIN_DIR"] == str(built)
        assert env["MOBILE_E2E_STAGE"] == stage[0][0]
        assert env["pool"] == []

    [(report, report_env)] = checkout.report()
    assert report == [
        "-m", "agterm_conformance.code_coverage", "e2e", "--app", "headless", "--platform", "linux",
        "--profiles", str(pool), "--release-dir", str(built), "--stage-bin", f"{stage[0][0]}/bin",
        "--repo", str(checkout.root), "--out", str(checkout.root / "report"),
    ]
    assert report_env["RUSTC_WRAPPER"] == WRAPPER
    assert report_env["PYTHONPATH"] == str(checkout.root / "conformance")
    assert result.stderr.endswith(f"→ headless e2e coverage: {checkout.root / 'report'}/summary.md\n")


def test_a_release_coverage_run_builds_tests_and_reads_the_release_build(checkout: Checkout) -> None:
    result = checkout.run("--profile", "release", "--coverage", "report")
    assert result.returncode == 0, result.stderr
    built = checkout.cov / "release"
    assert all("--release" in args for args, _ in checkout.cargo("build") + checkout.cargo("test"))
    assert {env["E2E_BIN_DIR"] for _, env in checkout.cargo("test")} == {str(built)}
    [(report, _)] = checkout.report()
    assert report[report.index("--release-dir") + 1] == str(built)


def test_a_failed_plain_run_is_exit_one(checkout: Checkout) -> None:
    result = checkout.run(FAILING_TEST="headless_e2e")
    assert result.returncode == 1
    assert "mobile e2e failed; its scratch directory is kept" in result.stderr


@pytest.mark.parametrize("coverage", [False, True])
def test_the_exit_trap_reaps_the_private_sessions_into_the_pool_under_coverage(checkout: Checkout, coverage: bool) -> None:
    session = checkout.root / "session-stand-in"
    session.write_text(f"#!{sys.executable}\n" + SESSION.format(log=str(checkout.log)))
    session.chmod(0o755)
    args = ("--coverage", "report") if coverage else ()
    result = checkout.run(*args, STAGE_SESSION=str(session), LIVE_DIR="the namespace")
    assert result.returncode == 0, result.stderr
    reaped = [(args, env) for tool, args, env in checkout.calls() if tool == "ngterm-session"]
    assert [args for args, _ in reaped] == [["list", "--short"], ["kill", "--force", "left"]]
    profile = f"{checkout.cov}/headless-e2e/headless-%4m.profraw" if coverage else None
    assert {env["LLVM_PROFILE_FILE"] for _, env in reaped} == {profile}


@pytest.mark.parametrize("failing", ["headless_e2e", "agent_hooks", "lifecycle", "e2e"])
def test_a_failed_test_still_runs_the_rest_and_reports(checkout: Checkout, failing: str) -> None:
    result = checkout.run("--coverage", "report", FAILING_TEST=failing)
    assert result.returncode == 1
    assert len(checkout.cargo("test")) == 3
    assert len(checkout.report()) == 1
    assert "mobile e2e: a test failed (the coverage report is written)\n" in result.stderr


def test_no_report_is_exit_two(checkout: Checkout) -> None:
    result = checkout.run("--coverage", "report", REPORT_STATUS="2")
    assert result.returncode == 2
    assert "mobile e2e: no coverage report could be made from the run's profiles\n" in result.stderr


def test_a_failed_instrumented_build_runs_nothing(checkout: Checkout) -> None:
    result = checkout.run("--coverage", "report", BUILD_STATUS="101")
    assert result.returncode == 2
    assert result.stderr.endswith("mobile e2e: the instrumented build failed\n")
    assert checkout.cargo("test") == [] and checkout.report() == []


def test_a_python_without_tomllib_is_refused_before_cargo(checkout: Checkout) -> None:
    result = checkout.run(
        "--coverage", "report", WITHOUT_TOMLLIB="python3 python python3.13 python3.12 python3.11"
    )
    assert result.returncode == 2
    assert result.stderr == "mobile e2e: Python 3.11+ with tomllib is required for --coverage (set PYTHON)\n"
    assert checkout.cargo("build") == []


def test_the_python_override_makes_the_report(checkout: Checkout) -> None:
    override = checkout.tool("chosen-python", PYTHON)
    checkout.tool("python3", "exit 49\n", shell=True)
    result = checkout.run("--coverage", "report", PYTHON=str(override))
    assert result.returncode == 0, result.stderr
    assert [tool for tool, args, _ in checkout.calls() if tool.endswith("python") and args[0] == "-m"] == [
        "chosen-python"
    ]


@pytest.mark.parametrize("args", [("--coverage",), ("--coverage", ""), ("--profile", "fast"), ("--other",)])
def test_bad_arguments_are_a_usage_error(checkout: Checkout, args: tuple[str, ...]) -> None:
    result = checkout.run(*args)
    assert result.returncode == 1
    assert result.stderr == "usage: scripts/test-mobile-e2e.sh [--profile debug|release] [--coverage DIR]\n"
    assert checkout.calls() == []
