"""scripts/unit-coverage.sh, the unit-test coverage gate's runner: its crate selection per host, the Python it picks,
where its test arguments go and its exit codes. Each case runs the real script over a copy of the scripts and pins,
with stand-in cargo, uname and Python on PATH; nothing is built and no coverage profile is read."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = "scripts/unit-coverage.sh"

CARGO = '''
import json, os, sys
args = sys.argv[1:]
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps(["cargo", *args]) + "\\n")
if args[0] == "metadata":
    print(json.dumps({"target_directory": os.environ["TEST_TARGET"]}))
elif args[:2] == ["llvm-cov", "show-env"]:
    pass
elif "--no-run" in args:
    sys.exit(int(os.environ.get("BUILD_STATUS", "0")))
else:
    sys.exit(int(os.environ.get("TEST_STATUS", "0")))
'''

PYTHON = '''
import json, os, sys
with open(os.environ["TOOL_LOG"], "a") as log:
    log.write(json.dumps([os.path.basename(sys.argv[0]), *sys.argv[1:]]) + "\\n")
if os.path.basename(sys.argv[0]) in os.environ.get("WITHOUT_TOMLLIB", "").split():
    if sys.argv[1] == "-m" or "tomllib" in sys.argv[2]:
        raise ModuleNotFoundError("No module named 'tomllib'")
if sys.argv[1] == "-c":
    exec(sys.argv[2])
else:
    sys.exit(int(os.environ.get("REPORT_STATUS", "0")))
'''


class Checkout:
    """The script, its library and the pins as a repository of their own, with stand-in tools first on PATH."""

    def __init__(self, root: Path) -> None:
        self.root = root
        for name in (SCRIPT, "scripts/lib/native.sh", "build/pins.env"):
            destination = root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(REPO / name, destination)
        for name in ("ngterm-proto", "ngterm-wpe", "ngterm-wpe-sys", "ngterm-ghostty-vt-sys"):
            (root / "crates" / name).mkdir(parents=True)
        self.bin = root / "bin"
        self.bin.mkdir()
        self.log = root / "calls.jsonl"
        self.env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("NGTERM_", "AGTERM_")) and key != "PYTHON"
        }
        self.env.update(
            PATH=f"{self.bin}{os.pathsep}{os.environ['PATH']}",
            TOOL_LOG=str(self.log),
            TEST_TARGET=str(root / "target"),
            TEST_OS="Linux",
        )
        self.tool("uname", 'printf "%s\\n" "$TEST_OS"\n', shell=True)
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
            ["bash", str(self.root / SCRIPT), str(self.root / "out"), *args],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )

    def calls(self, tool: str) -> list[list[str]]:
        if not self.log.exists():
            return []
        return [call[1:] for call in map(json.loads, self.log.read_text().splitlines()) if call[0] == tool]


@pytest.fixture
def checkout(tmp_path: Path) -> Checkout:
    return Checkout(tmp_path / "repo")


@pytest.mark.parametrize("host", ["MINGW64_NT-10.0", "Darwin"])
def test_default_selection_leaves_wpe_out_on_windows_and_macos(checkout: Checkout, host: str) -> None:
    result = checkout.run(TEST_OS=host)
    assert result.returncode == 0, result.stderr
    for call in checkout.calls("cargo")[-2:]:
        assert "--workspace" in call
        assert ["--exclude", "ngterm-wpe"] in [call[i : i + 2] for i in range(len(call))]


def test_linux_default_selection_and_output_stay_the_same(checkout: Checkout) -> None:
    result = checkout.run()
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert checkout.calls("cargo")[-2:] == [
        ["test", "--tests", "--no-run", "--message-format=json", "--workspace",
         "--exclude", "ngterm-ghostty-vt-sys", "--exclude", "ngterm-wpe-sys"],
        ["test", "--tests", "--no-fail-fast", "--workspace",
         "--exclude", "ngterm-ghostty-vt-sys", "--exclude", "ngterm-wpe-sys"],
    ]


def test_a_store_alias_falls_back_to_python_for_metadata_and_report(checkout: Checkout) -> None:
    checkout.tool("python3", 'echo "Python was not found" >&2\nexit 49\n', shell=True)
    result = checkout.run("-p", "ngterm-proto")
    assert (result.returncode, result.stderr) == (0, "")
    calls = checkout.calls("python")
    assert any(call[:1] == ["-c"] and "target_directory" in call[1] for call in calls)
    assert any(call[:2] == ["-m", "agterm_conformance.code_coverage"] for call in calls)


def test_python_override_is_used_for_both_calls(checkout: Checkout) -> None:
    override = checkout.bin / "chosen python"
    shutil.copy2(checkout.bin / "python", override)
    checkout.tool("python3", "exit 49\n", shell=True)
    result = checkout.run("-p", "ngterm-proto", PYTHON=str(override))
    assert result.returncode == 0, result.stderr
    assert any(call[:1] == ["-c"] for call in checkout.calls(override.name))
    assert any(call[:1] == ["-m"] for call in checkout.calls(override.name))


@pytest.mark.parametrize("fallback", ["python", "python3.13", "python3.12", "python3.11"])
def test_python_without_tomllib_uses_the_first_capable_fallback(checkout: Checkout, fallback: str) -> None:
    candidates = ["python3", "python", "python3.13", "python3.12", "python3.11"]
    rejected = candidates[:candidates.index(fallback)]
    result = checkout.run("-p", "ngterm-proto", TEST_OS="Darwin", WITHOUT_TOMLLIB=" ".join(rejected))
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    calls = list(map(json.loads, checkout.log.read_text().splitlines()))
    probes = [[name, "-c", "import tomllib"] for name in [*rejected, fallback]]
    assert calls[:len(probes)] == probes
    assert any("target_directory" in call[1] for call in checkout.calls(fallback) if call[0] == "-c")
    assert checkout.calls(fallback)[-1][:2] == ["-m", "agterm_conformance.code_coverage"]


def test_python_override_without_tomllib_falls_back(checkout: Checkout) -> None:
    override = checkout.tool("old-python", PYTHON)
    result = checkout.run("-p", "ngterm-proto", PYTHON=str(override), WITHOUT_TOMLLIB=override.name)
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    assert checkout.calls(override.name) == [["-c", "import tomllib"]]
    assert checkout.calls("python3")[-1][:2] == ["-m", "agterm_conformance.code_coverage"]


def test_no_python_with_tomllib_fails_before_cargo(checkout: Checkout) -> None:
    result = checkout.run(WITHOUT_TOMLLIB="python3 python python3.13 python3.12 python3.11")
    assert result.returncode == 2
    assert result.stdout == ""
    assert result.stderr == "unit-coverage: Python 3.11+ with tomllib is required (set PYTHON)\n"
    assert checkout.calls("cargo") == []


def test_harness_arguments_go_only_to_the_test_run(checkout: Checkout) -> None:
    result = checkout.run("-p", "ngterm-proto", "-p", "ngterm-pty", "--", "--skip", "a test", "--test-threads=1")
    assert result.returncode == 0, result.stderr
    build, run = checkout.calls("cargo")[-2:]
    assert build[-4:] == ["-p", "ngterm-proto", "-p", "ngterm-pty"]
    assert "--exclude" not in build
    assert run[-4:] == ["--", "--skip", "a test", "--test-threads=1"]
    report = checkout.calls("python3")[-1]
    assert report[-4:] == ["--crate", "ngterm-proto", "--crate", "ngterm-pty"]


@pytest.mark.parametrize("tail", [("--skip", "test_name"), ()])
def test_harness_arguments_work_with_default_selection_and_empty_tail(checkout: Checkout, tail: tuple[str, ...]) -> None:
    result = checkout.run("--", *tail)
    assert result.returncode == 0, result.stderr
    build, run = checkout.calls("cargo")[-2:]
    assert "--workspace" in build
    assert "--skip" not in build
    if tail:
        assert run[-3:] == ["--", *tail]
    else:
        assert "--" not in run


@pytest.mark.parametrize("args", [("-p",), ("--skip", "test_name")])
def test_bad_selection_is_a_usage_error(checkout: Checkout, args: tuple[str, ...]) -> None:
    result = checkout.run(*args)
    assert result.returncode == 2
    assert "usage:" in result.stderr
    assert "[-- TEST_ARGS...]" in result.stderr


def test_a_test_failure_still_reports(checkout: Checkout) -> None:
    result = checkout.run("-p", "ngterm-proto", TEST_STATUS="1")
    assert result.returncode == 1
    assert result.stderr == "unit-coverage: a test failed (the report is written)\n"
    assert checkout.calls("python3")[-1][:1] == ["-m"]


def test_a_build_failure_does_not_report(checkout: Checkout) -> None:
    result = checkout.run("-p", "ngterm-proto", BUILD_STATUS="1")
    assert result.returncode == 2
    assert result.stderr == "unit-coverage: the build failed\n"
    assert not any(call[:1] == ["-m"] for call in checkout.calls("python3"))


def test_report_failure_is_exit_two(checkout: Checkout) -> None:
    result = checkout.run("-p", "ngterm-proto", REPORT_STATUS="1")
    assert result.returncode == 2
    assert result.stderr == "unit-coverage: no report could be made from the run's profiles\n"
