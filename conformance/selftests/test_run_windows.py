"""run-windows.sh on Linux, with stand-ins for Git Bash's uname and cygpath and for the WebView2 warm-up: what it gives
pytest, not the Windows app."""

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="the stand-ins are POSIX shell scripts")

SCRIPT = Path(__file__).parents[1] / "tools" / "run-windows.sh"
REPO = SCRIPT.parents[2]
PROBE_ID = "tests/protocol/test_probe.py::test_probe"
OTHER_ID = "tests/protocol/test_other.py::test_other"

UNAME = """#!/bin/sh
echo MINGW64_NT-10.0-26100
"""

# As Git Bash's cygpath, -u turns a drive (C:\, here CYGPATH_DRIVE) and backslashes into a POSIX path, -ua makes it
# absolute, and -w keeps a POSIX path (pytest on Linux reads it). A node id is not a path: the real one mangles it.
CYGPATH = r"""#!/bin/sh
case "$2" in
    *::* | *[[]*) echo "cygpath stand-in: not a path: $2" >&2; exit 1 ;;
esac
case "$1" in
    -u) printf '%s\n' "$2" | sed 's|^[A-Za-z]:\\|'"$CYGPATH_DRIVE"'/|' | tr '\\' / ;;
    -ua) realpath -m -- "$2" ;;
    -w) printf '%s\n' "$2" ;;
    *) echo "cygpath stand-in: $*" >&2; exit 2 ;;
esac
"""

# The warm-up opens a web page in the app: it passes here, and the suite's python is this one.
PYTHON = f"""#!/bin/sh
case "$1" in
    *warm_webview2.py) exit 0 ;;
esac
exec {sys.executable} "$@"
"""

PARAM_TEST = """
import pytest

@pytest.mark.parametrize("name", ["one", "two"])
def test_case(name):
    pass
"""


@pytest.fixture
def layers(tmp_path: Path) -> Path:
    tests = tmp_path / "tests"
    (tests / "protocol").mkdir(parents=True)
    (tests / "cli").mkdir()
    for name in ("catalog.yaml", "ui-ids.yaml"):
        shutil.copy2(SCRIPT.parents[1] / name, tmp_path / name)
    (tests / "protocol" / "test_probe.py").write_text("def test_probe():\n    pass\n")
    (tests / "protocol" / "test_other.py").write_text("def test_other():\n    assert False, 'red by design'\n")
    (tests / "protocol" / "test_param.py").write_text(PARAM_TEST)
    (tests / "cli" / "test_cli.py").write_text("def test_cli():\n    assert False, 'red by design'\n")
    return tests


@pytest.fixture
def git_bash(tmp_path: Path) -> Path:
    directory = tmp_path / "git-bash"
    directory.mkdir()
    for name, text in (("uname", UNAME), ("cygpath", CYGPATH), ("python", PYTHON)):
        (directory / name).write_text(text)
        (directory / name).chmod(0o755)
    return directory


def run(
    tmp_path: Path, git_bash: Path, layers: Path, *args: str, cwd: Path = REPO
) -> tuple[subprocess.CompletedProcess[str], str, list[str]]:
    """Runs with the red OTHER_ID listed. The tests take no app, but its --bin names the rootdir, as on Windows."""
    stage = tmp_path / "stage"
    (stage / "bin").mkdir(parents=True, exist_ok=True)
    (stage / "bin" / "ngterm.exe").write_text("#!/bin/sh\nexit 1\n")
    (stage / "bin" / "ngterm.exe").chmod(0o755)
    listed = tmp_path / "rust-passing-windows.txt"
    listed.write_text(f"# ids the build passes\n{OTHER_ID}\n{PROBE_ID}\n")
    (tmp_path / "temp").mkdir(exist_ok=True)
    out = tmp_path / "report"
    env = {key: value for key, value in os.environ.items() if not key.startswith(("NGTERM_", "AGTERM_"))}
    env.pop("PYTHONPATH", None)
    env.pop("GITHUB_STEP_SUMMARY", None)
    env.update(
        PATH=f"{git_bash}{os.pathsep}{os.environ['PATH']}",
        PYTHON=str(git_bash / "python"),
        TEMP=str(tmp_path / "temp"),
        CYGPATH_DRIVE=str(tmp_path),
    )
    completed = subprocess.run(
        [str(SCRIPT), "--stage", str(stage), "--tests", str(layers), "--ratchet", str(listed), "--out", str(out), *args],
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    report = out / "report.md"
    nodeids = out / "nodeids.jsonl"
    records = [json.loads(line) for line in nodeids.read_text().splitlines()] if nodeids.exists() else []
    ran = sorted(record["nodeid"] for record in records if record["kind"] == "test")
    return completed, report.read_text() if report.exists() else "", ran


def test_without_a_selector_every_layer_runs(tmp_path: Path, git_bash: Path, layers: Path) -> None:
    completed, report, ran = run(tmp_path, git_bash, layers)
    assert completed.returncode == 1, completed.stdout + completed.stderr
    assert "- mode: full" in report
    assert "- layers: protocol cli" in report
    assert ran == [
        "tests/cli/test_cli.py::test_cli",
        OTHER_ID,
        "tests/protocol/test_param.py::test_case[one]",
        "tests/protocol/test_param.py::test_case[two]",
        PROBE_ID,
    ]


@pytest.mark.parametrize(
    ("selector", "where", "selected"),
    [
        (PROBE_ID, "repo", [PROBE_ID]),
        (f"conformance/{PROBE_ID}", "repo", [PROBE_ID]),
        (f"./{PROBE_ID}", "repo", [PROBE_ID]),
        ("conformance\\tests\\protocol\\test_probe.py::test_probe", "repo", [PROBE_ID]),
        ("tests/protocol/test_param.py::test_case[one]", "repo", ["tests/protocol/test_param.py::test_case[one]"]),
        ("protocol/test_probe.py::test_probe", "tests", [PROBE_ID]),
        ("protocol\\test_probe.py", "tests", [PROBE_ID]),
        ("tests/protocol/test_probe.py", "absolute", [PROBE_ID]),
        ("C:\\tests\\protocol\\test_param.py::test_case[two]", "repo", ["tests/protocol/test_param.py::test_case[two]"]),
    ],
)
def test_a_node_id_after_the_separator_runs_only_that_node(
    tmp_path: Path, git_bash: Path, layers: Path, selector: str, where: str, selected: list[str]
) -> None:
    """The red listed OTHER_ID is not run and not judged: the run holds."""
    cwd = layers if where == "tests" else REPO
    if where == "absolute":
        selector = str(layers.parent / selector)
    completed, report, ran = run(tmp_path, git_bash, layers, "--", selector, cwd=cwd)
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert ran == selected
    assert "- mode: partial, the tests that `-- " in report


def test_a_node_id_and_k_narrow_together_and_a_value_stays_its_options(
    tmp_path: Path, git_bash: Path, layers: Path
) -> None:
    completed, report, ran = run(
        tmp_path, git_bash, layers, "--", "--deselect", OTHER_ID, "tests/protocol/test_param.py", "-k", "two"
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert ran == ["tests/protocol/test_param.py::test_case[two]"]
    assert "- mode: partial" in report


def test_k_alone_keeps_the_layers(tmp_path: Path, git_bash: Path, layers: Path) -> None:
    completed, report, ran = run(tmp_path, git_bash, layers, "--", "-k", "probe or cli")
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert ran == ["tests/cli/test_cli.py::test_cli", PROBE_ID]
    assert "- mode: partial" in report


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (("--", "tests/protcol/test_probe.py::test_probe"), "no test path tests/protcol/test_probe.py"),
        (("--ratchet-only", "--", PROBE_ID), "--ratchet-only takes its extra node ids and paths with --also"),
    ],
)
def test_a_selector_that_cannot_work_exits_2_before_anything_runs(
    tmp_path: Path, git_bash: Path, layers: Path, args: tuple[str, ...], message: str
) -> None:
    completed, report, ran = run(tmp_path, git_bash, layers, *args)
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert message in completed.stderr
    assert report == "" and ran == []
