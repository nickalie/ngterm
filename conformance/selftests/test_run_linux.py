import fcntl
import json
import os
import re
import selectors
import shutil
import subprocess
import sys
import time
from pathlib import Path

import pytest

SCRIPT = Path(__file__).parents[1] / "tools" / "run-linux.sh"
# What a Rust build's `version` answers; without the conformance block every test skips as not-yet-at-level.
AT_LEVEL = json.dumps({"app": {"version": "0.1.0"}, "conformance": {"parityLevel": "0.36.0", "capabilities": []}})
PROBE_ID = "tests/protocol/test_probe.py::test_reports_version"
OTHER_ID = "tests/protocol/test_other.py::test_other"

VERSION_TEST = """
from agterm_conformance.match import assert_ok

def test_reports_version(ctl):
    assert assert_ok(ctl.request("version"))["app"]["version"]
"""

OTHER_TEST = """
def test_other(ctl):
    pass
"""

FAILING_TEST = """
def test_other(ctl):
    assert False, "red by design"
"""

LEAKING_TEST = """
from agterm_conformance.impls.base import make_root

def test_reports_version(ctl):
    make_root()
"""

# Runs in the runner's own pytest process, which inherits the runner's descriptors and environment as they are; the
# app does not do for a probe, since the harness starts it with close_fds.
PROBE_FDS_TEST = """
import json
import os

def test_reports_version(ctl):
    fds = []
    for fd in os.listdir("/proc/self/fd"):
        try:
            found = os.stat(f"/proc/self/fd/{fd}")
        except OSError:
            continue  # the listing's own descriptor, closed by now
        fds.append([found.st_dev, found.st_ino])
    with open(os.environ["FD_PROBE"], "w") as sink:
        json.dump({"fds": fds, "env": sorted(os.environ)}, sink)
"""


# What the instance was started with, written where the test runner says.
LAUNCH_PROBE_TEST = """
import json
import os

def test_reports_version(app):
    weston = json.loads((app.root / "fake-compositor.json").read_text())["argv"]
    with open(os.environ["LAUNCH_PROBE"], "w") as sink:
        json.dump({"env": app.env, "weston": weston}, sink)
"""


@pytest.fixture
def layers(tmp_path: Path) -> Path:
    """Test layers under a rootdir that holds the repository's catalog files (the run's default catalog)."""
    tests = tmp_path / "tests"
    (tests / "protocol").mkdir(parents=True)
    for name in ("catalog.yaml", "ui-ids.yaml"):
        shutil.copy2(SCRIPT.parents[1] / name, tmp_path / name)
    (tests / "protocol" / "test_probe.py").write_text(VERSION_TEST)
    return tests


@pytest.fixture
def stage(tmp_path: Path, fake_bin: Path) -> Path:
    """A staged tree whose bin/ngterm is the fake app: no Rust binary needs to exist."""
    app = tmp_path / "stage" / "bin" / "ngterm"
    app.parent.mkdir(parents=True)
    shutil.copy2(fake_bin, app)
    return app.parent.parent


@pytest.fixture(autouse=True)
def lavapipe_icd(make_icd, monkeypatch: pytest.MonkeyPatch) -> Path:
    """A lavapipe ICD manifest, handed to scripts/fetch-lavapipe.sh as NGTERM_LAVAPIPE_ICD by every run."""
    icd = make_icd()
    monkeypatch.setenv("NGTERM_LAVAPIPE_ICD", str(icd))
    return icd


@pytest.fixture(autouse=True)
def weston_on_path(
    tmp_path: Path, fake_compositor_bin: Path, fake_session_bus_bin: Path, monkeypatch: pytest.MonkeyPatch
) -> Path:
    """The script's pytest is a subprocess the conftest patch cannot reach: it finds the fake weston and bus on PATH."""
    directory = tmp_path / "weston-bin"
    directory.mkdir()
    (directory / "weston").symlink_to(fake_compositor_bin)
    (directory / "dbus-daemon").symlink_to(fake_session_bus_bin)
    monkeypatch.setenv("PATH", f"{directory}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.delenv("GITHUB_STEP_SUMMARY", raising=False)
    return directory


@pytest.fixture(autouse=True)
def lock(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """The runner's admission lock, private to the test: a real suite run elsewhere never queues it or is queued."""
    path = tmp_path / "conformance.lock"
    monkeypatch.setenv("CONFORMANCE_LOCK", str(path))
    monkeypatch.delenv("CONFORMANCE_RUN_LOCKED", raising=False)
    return path


def ratchet(tmp_path: Path, *ids: str) -> Path:
    path = tmp_path / "rust-passing.txt"
    path.write_text("# ids the build passes\n" + "".join(f"{nodeid}\n" for nodeid in sorted(ids)))
    return path


def command(tmp_path: Path, stage: Path, layers: Path, listed: Path, *args: str) -> list[str]:
    return [
        str(SCRIPT),
        "--stage",
        str(stage),
        "--tests",
        str(layers),
        "--ratchet",
        str(listed),
        "--out",
        str(tmp_path / "report"),
        *args,
    ]


def run_env(**env: str) -> dict[str, str]:
    return {**os.environ, "PYTHON": sys.executable, "FAKE_VERSION": AT_LEVEL, **env}


def run(
    tmp_path: Path, stage: Path, layers: Path, listed: Path, *args: str, **env: str
) -> tuple[subprocess.CompletedProcess[str], str]:
    out = tmp_path / "report"
    completed = subprocess.run(
        command(tmp_path, stage, layers, listed, *args),
        env=run_env(**env),
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    report = out / "report.md"
    return completed, report.read_text() if report.exists() else ""


def test_a_holding_run_writes_the_report(tmp_path: Path, stage: Path, layers: Path) -> None:
    (tmp_path / "report").mkdir()
    (tmp_path / "report" / "keep.txt").write_text("mine")
    (tmp_path / "report" / "report.md").write_text("stale")
    stale_trace = tmp_path / "report/traces/removed-test"
    stale_trace.mkdir(parents=True)
    (stale_trace / "screenshot.png").write_bytes(b"stale")
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), AGTERM_SOCKET="/nowhere")
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert f"ngterm: `{stage}/bin/ngterm`" in report
    assert "layers: protocol" in report
    assert "display: headless" in report
    assert "rust: 1 pass, 0 fail" in report
    assert "### ratchet" in report
    assert "ratchet holds" in report and "not collected" not in completed.stdout
    assert "Left behind" not in report
    out = tmp_path / "report"
    assert (out / "junit.xml").is_file()
    records = [json.loads(line) for line in (out / "nodeids.jsonl").read_text().splitlines()]
    assert {"kind": "test", "nodeid": PROBE_ID, "outcome": "passed"}.items() <= records[1].items()
    assert "scrubbed from env" not in (out / "pytest.log").read_text()
    assert (out / "keep.txt").read_text() == "mine"
    assert not stale_trace.exists()


def test_a_run_launches_its_binary_on_lavapipe_with_a_seat(
    tmp_path: Path, stage: Path, layers: Path, lavapipe_icd: Path
) -> None:
    (layers / "protocol" / "test_probe.py").write_text(LAUNCH_PROBE_TEST)
    probe = tmp_path / "launch.json"
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), LAUNCH_PROBE=str(probe))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert report.startswith("## rust-linux conformance\n")
    assert "frontend" not in report
    assert f"ngterm: `{stage}/bin/ngterm`" in report
    assert f"Vulkan: lavapipe only, `{lavapipe_icd}`" in report
    assert "rust: 1 pass, 0 fail" in report and "ratchet holds" in report
    assert "rust frontend" not in (tmp_path / "report" / "pytest.log").read_text()
    launched = json.loads(probe.read_text())
    assert launched["weston"][-1] == "--fake-seat"
    env = launched["env"]
    assert env["VK_DRIVER_FILES"] == env["VK_ICD_FILENAMES"] == str(lavapipe_icd)
    assert env["LIBGL_ALWAYS_SOFTWARE"] == "1"
    # the override reached the script, never the instance
    assert not {"NGTERM_LAVAPIPE_ICD", "CONFORMANCE_LAVAPIPE_ICD"} & env.keys()


def test_a_run_without_the_binary_skips_everything(tmp_path: Path, layers: Path) -> None:
    """A stage without bin/ngterm (the app crate not built): nothing launches."""
    empty = tmp_path / "empty-stage"
    (empty / "bin").mkdir(parents=True)
    completed, report = run(tmp_path, empty, layers, ratchet(tmp_path))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "ngterm: not staged" in report
    assert "rust: 0 pass, 0 fail, 0 skip(unsupported), 1 skip(not-yet-at-level)" in report


def test_a_run_without_lavapipe_stops_before_anything_runs(
    tmp_path: Path, stage: Path, layers: Path, lock: Path
) -> None:
    with open(lock, "w") as held:
        fcntl.flock(held, fcntl.LOCK_EX)  # refused before queueing
        completed, report = run(
            tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), NGTERM_LAVAPIPE_ICD=str(tmp_path / "absent.json")
        )
    assert completed.returncode == 2
    assert "NGTERM_LAVAPIPE_ICD names no ICD manifest" in completed.stderr
    assert "no lavapipe for the GPUI build" in completed.stderr
    assert "waiting for another conformance run" not in completed.stdout
    assert report == ""


def vendored_weston(vendor: Path, fake_compositor_bin: Path) -> Path:
    """A complete scripts/build-weston.sh prefix under `vendor` whose weston is the fake one, with a seat."""
    repo = SCRIPT.parents[2]
    found = subprocess.run(
        [str(repo / "scripts/build-weston.sh"), "--print-prefix"],
        env={**os.environ, "NGTERM_VENDOR": str(vendor)},
        capture_output=True,
        text=True,
        check=True,
    )
    prefix = Path(found.stdout.strip())
    modules = ("lib/libweston-15/headless-backend.so", "lib/libweston-15/gl-renderer.so", "lib/weston/desktop-shell.so")
    for path in modules:
        (prefix / path).parent.mkdir(parents=True, exist_ok=True)
        (prefix / path).write_text("stand-in")
    for path in ("bin/weston", "libexec/weston-desktop-shell"):
        (prefix / path).parent.mkdir(parents=True, exist_ok=True)
        (prefix / path).write_text(f'#!/bin/sh\nexec env -u FAKE_COMPOSITOR_HELP "{fake_compositor_bin}" "$@"\n')
        (prefix / path).chmod(0o755)
    pins = (repo / "build/pins.env").read_text().splitlines()
    (prefix / "share/ngterm").mkdir(parents=True)
    manifest = "".join(f"{line}\n" for line in pins if line.startswith("WESTON_"))
    (prefix / "share/ngterm/weston-PINS").write_text(manifest)
    return prefix


def test_a_run_on_a_weston_without_a_seat_stops_naming_the_fix(tmp_path: Path, stage: Path, layers: Path) -> None:
    completed, report = run(
        tmp_path,
        stage,
        layers,
        ratchet(tmp_path, PROBE_ID),
        NGTERM_VENDOR=str(tmp_path / "vendor"),
        FAKE_COMPOSITOR_HELP="old",
    )
    assert completed.returncode == 2
    assert "has no --fake-seat" in completed.stderr and "scripts/build-weston.sh" in completed.stderr
    assert report == ""


def test_a_run_takes_the_vendored_weston_over_an_old_one(
    tmp_path: Path, stage: Path, layers: Path, fake_compositor_bin: Path
) -> None:
    prefix = vendored_weston(tmp_path / "vendor", fake_compositor_bin)
    (layers / "protocol" / "test_probe.py").write_text(LAUNCH_PROBE_TEST)
    probe = tmp_path / "launch.json"
    completed, report = run(
        tmp_path,
        stage,
        layers,
        ratchet(tmp_path, PROBE_ID),
        NGTERM_VENDOR=str(tmp_path / "vendor"),
        FAKE_COMPOSITOR_HELP="old",
        LAUNCH_PROBE=str(probe),
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "rust: 1 pass, 0 fail" in report
    # the old weston on PATH would have refused the launch: the vendored one served it
    assert json.loads(probe.read_text())["weston"][-1] == "--fake-seat"
    assert prefix.is_dir()


def test_a_ratchet_only_run_runs_the_listed_ids_and_judges_them(tmp_path: Path, stage: Path, layers: Path) -> None:
    """The unlisted red test is left out, so it costs nothing; the report names the mode and the suite time."""
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--ratchet-only")
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "- mode: ratchet-only, the ids of" in report and "rust-passing.txt`\n" in report
    assert re.search(r"^- suite time: \d+s$", report, re.MULTILINE)
    assert "rust: 1 pass, 0 fail" in report
    assert "ratchet holds: 1 listed id(s), ratchet-only run" in report
    header = json.loads((tmp_path / "report" / "nodeids.jsonl").read_text().splitlines()[0])
    assert {key: header[key] for key in ("kind", "partial", "mode")} == {
        "kind": "run", "partial": True, "mode": "ratchet-only"
    }


def test_a_ratchet_only_run_breaks_on_a_red_listed_id(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, OTHER_ID, PROBE_ID), "--ratchet-only")
    assert completed.returncode == 1
    assert f"BROKEN failed: {OTHER_ID}" in report


def test_a_ratchet_only_run_adds_the_tasks_extras(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(OTHER_TEST)
    (layers / "extra").mkdir()
    (layers / "extra" / "test_extra.py").write_text("def test_extra(ctl):\n    pass\n")
    (layers / "protocol" / "test_skipped.py").write_text("def test_never(ctl):\n    assert False\n")
    completed, report = run(
        tmp_path,
        stage,
        layers,
        ratchet(tmp_path, PROBE_ID),
        "--ratchet-only",
        "--also",
        "tests/extra/test_extra.py",
        "--also-k",
        "test_other",
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "--also tests/extra/test_extra.py + --also-k test_other" in report
    assert "rust: 3 pass, 0 fail" in report
    assert re.search(rf"^{re.escape(OTHER_ID)}$", completed.stdout, re.MULTILINE)
    assert re.search(r"^tests/extra/test_extra.py::test_extra$", completed.stdout, re.MULTILINE)


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (("--also", "tests/protocol"), "--also and --also-k need --ratchet-only"),
        (("--ratchet-only", "--also", "tests/nowhere.py"), "--also tests/nowhere.py: no "),
    ],
)
def test_ratchet_only_usage_errors_exit_2(
    tmp_path: Path, stage: Path, layers: Path, args: tuple[str, ...], message: str
) -> None:
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), *args)
    assert completed.returncode == 2
    assert message in completed.stderr
    assert report == ""


@pytest.mark.parametrize(
    ("extra", "message"),
    [
        (("--also", "tests/protocol/test_probe.py::test_typo"), "--also 'tests/protocol/test_probe.py::test_typo'"),
        (("--also-k", "sidbar"), "--also-k 'sidbar'"),
    ],
)
def test_an_extra_that_matches_no_test_exits_2(
    tmp_path: Path, stage: Path, layers: Path, extra: tuple[str, ...], message: str
) -> None:
    completed, _ = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--ratchet-only", *extra)
    assert completed.returncode == 2
    assert f"{message}: matches no collected test" in completed.stdout + completed.stderr


def test_a_full_run_says_so(tmp_path: Path, stage: Path, layers: Path) -> None:
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "- mode: full\n" in report and "ratchet holds: 1 listed id(s), full run" in report


def test_there_is_no_frontend_switch(tmp_path: Path, stage: Path, layers: Path) -> None:
    """One Rust build since the GTK frontend went: `--frontend` is an unknown argument."""
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path), "--frontend", "gpui")
    assert completed.returncode == 2
    assert "unknown argument: --frontend" in completed.stderr
    assert report == ""


def test_one_ratchet_list_holds_the_build() -> None:
    """rust-passing.txt with its `#` header, sorted; the GPUI build's own list was folded into it."""
    conformance = SCRIPT.parents[1]
    text = (conformance / "rust-passing.txt").read_text()
    assert text.startswith("#")
    ids = [line for line in text.splitlines() if line and not line.startswith("#")]
    assert ids == sorted(ids) and len(ids) == len(set(ids))
    assert not (conformance / "rust-gpui-passing.txt").exists()
    help_text = subprocess.run([str(SCRIPT), "--help"], capture_output=True, text=True, check=True).stdout
    assert "conformance/rust-passing.txt" in help_text and "--frontend" not in help_text


def test_a_red_unlisted_test_does_not_fail_the_run(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "rust: 1 pass, 1 fail" in report
    assert "pytest status: 1" in report
    assert "exit status: 0" in report


def test_a_red_listed_test_breaks_the_ratchet(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, OTHER_ID, PROBE_ID))
    assert completed.returncode == 1
    assert OTHER_ID in report
    assert "exit status: 1" in report


def test_a_newly_passing_id_is_printed(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(OTHER_TEST)
    listed = ratchet(tmp_path, PROBE_ID)
    completed, report = run(tmp_path, stage, layers, listed)
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert re.search(rf"^{re.escape(OTHER_ID)}$", completed.stdout, re.MULTILINE)
    assert OTHER_ID in report
    assert OTHER_ID not in listed.read_text()


def test_without_an_agterm_binary_every_test_skips(tmp_path: Path, layers: Path) -> None:
    """Before the app crate exists: a stage without bin/ngterm runs, skips everything and holds the empty ratchet."""
    empty = tmp_path / "stage"
    (empty / "bin").mkdir(parents=True)
    completed, report = run(tmp_path, empty, layers, ratchet(tmp_path))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "ngterm: not staged" in report
    assert "rust: 0 pass, 0 fail, 0 skip(unsupported), 1 skip(not-yet-at-level)" in report


def test_a_root_left_behind_fails_a_holding_run(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_probe.py").write_text(LEAKING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID))
    left = [Path(root) for root in re.findall(r"`(/tmp/agt-[^`]+)`", report)]
    ours = [root for root in left if root.parent.name.startswith("agt-run-")]
    for root in ours:
        shutil.rmtree(root, ignore_errors=True)
        root.parent.rmdir()
    assert completed.returncode == 1
    assert "rust: 1 pass, 0 fail" in report
    assert len(ours) == len(left) == 1


def test_the_shards_of_a_run_merge_into_the_whole_runs_report(tmp_path: Path, stage: Path, layers: Path) -> None:
    """Each shard judges the listed ids it ran; the merged report holds every listed id, as one whole run does."""
    (layers / "protocol" / "test_other.py").write_text(OTHER_TEST)
    listed = ratchet(tmp_path, OTHER_ID, PROBE_ID)
    ran = []
    for index in (1, 2):
        completed, report = run(
            tmp_path, stage, layers, listed, "--shard", f"{index}/2", "--out", str(tmp_path / f"shard-{index}")
        )
        assert completed.returncode == 0, completed.stdout + completed.stderr
        text = (tmp_path / f"shard-{index}" / "report.md").read_text()
        assert f"- shard: {index}/2" in text and "rust: 1 pass, 0 fail" in text
        assert f"ratchet holds: 2 listed id(s), full run, shard {index}/2 alone" in text
        lines = (tmp_path / f"shard-{index}" / "nodeids.jsonl").read_text().splitlines()
        ran += [json.loads(line)["nodeid"] for line in lines[1:]]
    assert sorted(ran) == [OTHER_ID, PROBE_ID]
    merged = subprocess.run(
        [sys.executable, str(SCRIPT.with_name("merge_shards.py")), "--out", str(tmp_path / "merged"), "--list",
         str(listed), "--title", "rust-linux", str(tmp_path / "shard-1"), str(tmp_path / "shard-2")],
        capture_output=True, text=True, timeout=120, check=False,
    )
    assert merged.returncode == 0, merged.stdout + merged.stderr
    text = (tmp_path / "merged" / "report.md").read_text()
    assert "ratchet holds: 2 listed id(s), full run\n" in text
    assert "2 pass, 0 fail" in text and "- tests: 2 of 2 collected" in text
    assert "ui ids exercised by a passing test:" in text


def test_a_red_listed_id_breaks_its_shard(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    statuses = []
    for index in (1, 2):
        completed, _ = run(tmp_path, stage, layers, ratchet(tmp_path, OTHER_ID, PROBE_ID), "--shard", f"{index}/2")
        statuses.append(completed.returncode)
    assert sorted(statuses) == [0, 1]


def test_build_only_takes_no_stage(tmp_path: Path, stage: Path, layers: Path) -> None:
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--build-only")
    assert completed.returncode == 2
    assert "--build-only builds the stage: it takes no --stage" in completed.stderr
    assert report == ""


@pytest.mark.parametrize("case", ["missing stage", "no layer", "missing ratchet"])
def test_bad_arguments_exit_2(tmp_path: Path, stage: Path, layers: Path, lock: Path, case: str) -> None:
    listed = ratchet(tmp_path)
    if case == "missing stage":
        stage = tmp_path / "absent"
    elif case == "no layer":
        layers = Path("/")
    else:
        listed = tmp_path / "absent.txt"
    with open(lock, "w") as held:
        fcntl.flock(held, fcntl.LOCK_EX)  # refused before queueing: a check behind the lock would wait here
        completed, report = run(tmp_path, stage, layers, listed)
    assert completed.returncode == 2
    assert "waiting for another conformance run" not in completed.stdout
    assert report == ""


def test_a_lock_that_cannot_be_opened_exits_2(
    tmp_path: Path, stage: Path, layers: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    lock = tmp_path / "absent" / "conformance.lock"
    monkeypatch.setenv("CONFORMANCE_LOCK", str(lock))
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID))
    assert completed.returncode == 2
    assert "waiting for another conformance run" not in completed.stdout
    assert f"cannot open the lock {lock}" in completed.stderr
    assert report == ""


@pytest.mark.parametrize("missing", ["weston", "dbus-daemon"])
def test_a_headless_run_without_weston_or_a_bus_names_the_opt_out(
    tmp_path: Path,
    stage: Path,
    layers: Path,
    weston_on_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    missing: str,
) -> None:
    bare = tmp_path / "bare-bin"
    bare.mkdir()
    for tool in ("bash", "dirname", "readlink", "cat", "git", "grep", "mktemp", "rmdir", "rm", "mkdir", "env", "sed"):
        if found := shutil.which(tool):
            (bare / tool).symlink_to(found)
    for helper in {"weston", "dbus-daemon"} - {missing}:
        (bare / helper).symlink_to(weston_on_path / helper)
    monkeypatch.setenv("PATH", str(bare))
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path))
    assert completed.returncode == 2
    assert f"no {missing} on PATH" in completed.stderr and "CONFORMANCE_LINUX_DISPLAY=session" in completed.stderr
    assert report == ""


def test_pytest_args_reach_pytest(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_other.py").write_text(FAILING_TEST)
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--", "-k", "version")
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "pytest args: -k version" in report
    assert "rust: 1 pass, 0 fail" in report


def test_pytest_refusing_its_arguments_leaves_no_usable_report(tmp_path: Path, stage: Path, layers: Path) -> None:
    """A typo in a path after `--`: nothing runs, so the run must not read as a holding ratchet."""
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--", str(layers / "protcol"))
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert "pytest status: 4" in report
    assert "exit status: 2" in report
    assert "ratchet holds" not in report


def read_until(process: subprocess.Popen[str], needle: str, timeout: float = 30.0) -> str:
    """The runner's stdout up to the line holding `needle`; fails on EOF or after `timeout` seconds."""
    assert process.stdout is not None
    seen = ""
    deadline = time.monotonic() + timeout
    with selectors.DefaultSelector() as selector:
        selector.register(process.stdout, selectors.EVENT_READ)
        while needle not in seen:
            left = deadline - time.monotonic()
            assert left > 0 and selector.select(left), f"no {needle!r} line within {timeout}s: {seen!r}"
            line = process.stdout.readline()
            assert line, f"the runner closed its output before {needle!r}: {seen!r}"
            seen += line
    return seen


def test_a_second_run_waits_for_the_lock(tmp_path: Path, stage: Path, layers: Path, lock: Path) -> None:
    with open(lock, "w") as held:
        fcntl.flock(held, fcntl.LOCK_EX)
        process = subprocess.Popen(
            command(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID)),
            env=run_env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        try:
            seen = read_until(process, "waiting for another conformance run")
            assert f"(lock {lock})" in seen
            time.sleep(0.5)  # long enough for an unqueued runner to get as far as its report
            assert process.poll() is None
            assert not (tmp_path / "report").exists()
        except BaseException:
            process.kill()
            process.communicate()
            raise
        fcntl.flock(held, fcntl.LOCK_UN)
    output, _ = process.communicate(timeout=120)
    assert process.returncode == 0, output
    assert "rust: 1 pass, 0 fail" in (tmp_path / "report" / "report.md").read_text()


def test_the_run_does_not_inherit_the_lock(tmp_path: Path, stage: Path, layers: Path, lock: Path) -> None:
    """Live sessions' servers outlive a run by design: started holding the lock's descriptor, they would hold it forever."""
    (layers / "protocol" / "test_probe.py").write_text(PROBE_FDS_TEST)
    probe = tmp_path / "fds.json"
    completed, _ = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), FD_PROBE=str(probe))
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "waiting for another conformance run" not in completed.stdout
    found = json.loads(probe.read_text())
    held = os.stat(lock)
    assert found["fds"], "the probe saw no descriptors at all"
    assert [held.st_dev, held.st_ino] not in found["fds"]
    assert "CONFORMANCE_RUN_LOCKED" not in found["env"]


def test_the_default_lock_is_the_users_runtime_dir(
    tmp_path: Path, stage: Path, layers: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    runtime = tmp_path / "runtime"
    runtime.mkdir(mode=0o700)
    monkeypatch.delenv("CONFORMANCE_LOCK")
    monkeypatch.setenv("XDG_RUNTIME_DIR", str(runtime))
    default = runtime / "agterm-conformance.lock"
    with open(default, "w") as held:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        process = subprocess.Popen(
            command(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID)),
            env=run_env(),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        try:
            assert f"(lock {default})" in read_until(process, "waiting for another conformance run")
        finally:
            fcntl.flock(held, fcntl.LOCK_UN)
            output, _ = process.communicate(timeout=120)
    assert process.returncode == 0, output


def test_ui_selection_refuses_session_display_before_starting(tmp_path, stage, layers):
    ui = layers / 'ui'
    ui.mkdir()
    (ui / 'test_ui.py').write_text('def test_ui(impl_options): pass\n')
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID),
                            '--ratchet-only', '--also', 'tests/ui', CONFORMANCE_LINUX_DISPLAY='session')
    assert completed.returncode == 2
    assert 'Linux UI instances require headless' in completed.stderr
    assert not report


def test_a_full_run_takes_the_ui_layer(tmp_path, stage, layers):
    # ui is a default layer: a full run preflights it, so session display mode stops it before anything runs
    ui = layers / 'ui'
    ui.mkdir()
    (ui / 'test_ui.py').write_text('def test_ui(impl_options): pass\n')
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), CONFORMANCE_LINUX_DISPLAY='session')
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert 'Linux UI instances require headless' in completed.stderr
    assert not report


def test_ui_setup_failure_withholds_runner_report(tmp_path, stage, layers):
    # A ui dir outside the default layers, passed as an extra pytest path, bypasses the runner's early preflight (which
    # would need at-spi2-core on the host): exercise the plugin's failure propagation.
    ui = tmp_path / 'extra' / 'tests' / 'ui'
    ui.mkdir(parents=True)
    (ui / 'test_ui.py').write_text('def test_ui(impl_options): pass\n')
    (layers.parent / 'conftest.py').write_text("""
from agterm_conformance import linux_ui
from agterm_conformance.impls.base import LaunchError

def pytest_configure(config):
    def fail(self):
        error = LaunchError('owned launcher failed')
        error.add_note('launcher log diagnostic')
        raise error
    linux_ui.A11yRun.start = fail
""")
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID),
                            '--', str(ui), '-k', 'test_ui')
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert 'owned launcher failed' in completed.stdout + completed.stderr
    assert 'launcher log diagnostic' in completed.stdout + completed.stderr
    assert not (tmp_path / 'report/nodeids.jsonl').exists()
    assert 'ratchet holds' not in report


def test_ui_preflight_without_xvfb_names_component(tmp_path, stage, layers, weston_on_path, monkeypatch):
    ui = layers / 'ui'
    ui.mkdir()
    (ui / 'test_ui.py').write_text('def test_ui(impl_options): pass\n')
    bare = tmp_path / 'bare-bin'
    bare.mkdir()
    for tool in ('bash', 'dirname', 'readlink', 'cat', 'git', 'grep', 'mktemp', 'rmdir', 'rm', 'mkdir', 'env', 'sed'):
        if found := shutil.which(tool):
            (bare / tool).symlink_to(found)
    for helper in ('weston', 'dbus-daemon'):
        (bare / helper).symlink_to(weston_on_path / helper)
    # Xvfb can also be discovered under libexec; isolate discovery in the child interpreter.
    (tmp_path / 'sitecustomize.py').write_text("""
from agterm_conformance import linux_ui
linux_ui.LIBEXEC_DIRS = ()
""")
    monkeypatch.setenv('PATH', str(bare))
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID),
                            '--ratchet-only', '--also', 'tests/ui', PYTHONPATH=str(tmp_path))
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert 'missing Xvfb' in completed.stderr
    assert not report
    assert not (tmp_path / 'report/nodeids.jsonl').exists()


# What the instance's profile variable was, written where the test runner says; a CLI of the build's ended by the
# harness before it exited, which the run must list as lost.
COVERAGE_PROBE_TEST = """
import os
import sys
from agterm_conformance.plugin import CliProcess

def test_reports_version(app):
    with open(os.environ["LAUNCH_PROBE"], "w") as sink:
        sink.write(app.env["LLVM_PROFILE_FILE"])
    ctl = app.root / "ngtermctl"
    ctl.write_text(f"#!{sys.executable}\\nimport time\\nprint('ready', flush=True)\\ntime.sleep(60)\\n")
    ctl.chmod(0o755)
    with CliProcess([str(ctl)], app.env, app.home, None) as process:
        assert process.line() == "ready"
"""


# The probe above, which also leaves a profile where the instance's variable points, as an instrumented app would.
PROFILE_WRITING_TEST = COVERAGE_PROBE_TEST.replace(
    '        sink.write(app.env["LLVM_PROFILE_FILE"])\n',
    '        sink.write(app.env["LLVM_PROFILE_FILE"])\n'
    '    from pathlib import Path\n'
    '    (Path(app.env["LLVM_PROFILE_FILE"]).parent / "conformance-1_0.profraw").write_bytes(b"raw")\n',
)

# `cargo llvm-cov show-env`: exports a marker the llvm tools below demand (the runner evaluated show-env).
FAKE_CARGO = """
import sys

args = sys.argv[1:]
assert args[:2] == ["llvm-cov", "show-env"], args
print("export FAKE_LLVM_COV_ENV=1")
"""


# `llvm-profdata merge` and `llvm-cov export` (the report's own, `LLVM_PROFDATA` / `LLVM_COV`): both demand show-env's
# marker; every program's export reads one ngterm-core file of 8 lines, 2 of them run.
FAKE_LLVM = """
import os
import sys
from pathlib import Path

args = sys.argv[1:]
if os.environ.get("FAKE_LLVM_COV_ENV") != "1":
    sys.exit("llvm tool outside show-env")
if args[0] == "merge":
    Path(args[args.index("-o") + 1]).write_bytes(b"merged")
    sys.exit(0)
assert args[:2] == ["export", "-format=lcov"], args
source = str(Path.cwd() / "crates/ngterm-core/src/lib.rs")
lines = [f"DA:{line},{1 if line <= 2 else 0}" for line in range(1, 9)]
print("\\n".join([f"SF:{source}", *lines, "end_of_record"]))
"""


@pytest.fixture
def fake_cargo(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    directory = tmp_path / "cargo-bin"
    directory.mkdir()
    cargo = directory / "cargo"
    cargo.write_text(f"#!{sys.executable}\n{FAKE_CARGO}")
    cargo.chmod(0o755)
    for tool in ("llvm-cov", "llvm-profdata"):
        fake = directory / tool
        fake.write_text(f"#!{sys.executable}\n{FAKE_LLVM}")
        fake.chmod(0o755)
        monkeypatch.setenv(tool.upper().replace("-", "_"), str(fake))
    monkeypatch.setenv("PATH", f"{directory}{os.pathsep}{os.environ['PATH']}")
    return cargo


# The desktop app's programs on Linux (conformance/coverage/apps.toml), whose coverage maps its e2e report reads.
DESKTOP_PROGRAMS = ("ngterm", "ngtermctl", "ngterm-session", "ngterm-web")


def built(stage: Path, target: Path) -> Path:
    """The instrumented build's own binaries of the desktop app, staged and in its release directory, where the report
    reads their coverage maps: the staged app's, and an instrumented stand-in for each other program. Returns the
    app's."""
    release = target / "llvm-cov-target" / "release"
    release.mkdir(parents=True, exist_ok=True)
    for name in DESKTOP_PROGRAMS:
        staged = stage / "bin" / name
        if not staged.exists():
            staged.write_bytes(b"#!/bin/sh\nexit 0\n# LLVM Profile Error\n")
            staged.chmod(0o755)
        shutil.copy2(staged, release / name)
    return release / "ngterm"


def instrument(binary: Path) -> None:
    """Gives a fake binary the profile runtime's marker (`llvm_profiles.RUNTIME_MARKER`), as a coverage build has."""
    with open(binary, "ab") as sink:
        sink.write(b"\n# LLVM Profile Error\n")


def test_a_coverage_run_refuses_a_stage_built_without_coverage(tmp_path: Path, stage: Path, layers: Path) -> None:
    completed, report = run(tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage")
    assert completed.returncode == 2, completed.stdout + completed.stderr
    assert "not built with coverage" in completed.stderr
    assert f"{stage}/bin/ngterm" in completed.stderr
    assert report == ""


def test_a_coverage_run_pools_the_profiles_and_lists_the_lost(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    instrument(stage / "bin" / "ngterm")
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    target = tmp_path / "target"
    built(stage, target)
    profiles = target / "llvm-cov-target"
    (profiles / "stale-1-2.profraw").write_bytes(b"last run")
    (profiles / "lost-profiles.txt").write_text("1 ngterm: last run\n")
    probe = tmp_path / "launch.txt"
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage",
        LAUNCH_PROBE=str(probe), CARGO_TARGET_DIR=str(target), FAKE_TERM="quit",
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert probe.read_text() == f"{profiles}/conformance-%4m.profraw"
    # the last run's profiles are gone; only the probe's is left
    assert [path.name for path in profiles.glob("*.profraw")] == ["conformance-1_0.profraw"]
    [lost] = (tmp_path / "report" / "lost-profiles.txt").read_text().splitlines()
    assert re.fullmatch(r"\d+ ngtermctl: CliProcess.close: ended before it exited", lost), lost
    assert f"- coverage: 1 profile files in `{profiles}`, 1 processes ended without one (lost-profiles.txt)" in report


def test_a_coverage_run_reports_code_and_catalog_coverage(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    instrument(stage / "bin" / "ngterm")
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    target = tmp_path / "target"
    built(stage, target)
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage",
        LAUNCH_PROBE=str(tmp_path / "launch.txt"), CARGO_TARGET_DIR=str(target), FAKE_TERM="quit",
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    out = tmp_path / "report" / "coverage"
    code = report.split("### code coverage\n", 1)[1].split("\n### catalog coverage", 1)[0]
    # the desktop app's e2e report: its programs alone, the total's distance to 90% and the share markers exclude
    assert "E2E line coverage of the `desktop` app on `linux`: the lines of its programs ngterm, ngtermctl, " \
        "ngterm-session, ngterm-web " in code
    assert "**Total: 25.00% of lines (2/8), 65.00 pt to 90%. Excluded by markers: 0.00% of lines (0/8).**" in code
    assert "| ngterm-core | 25.00% (2/8) | 0.00% (0/8) |" in code
    assert "| total | 25.00% (2/8) | 0.00% (0/8) |" in code
    assert "### Exclusion markers" in code
    assert f"lcov: `{out}/lcov.info`, after the markers: `{out}/lcov-measured.info`" in code
    assert (out / "summary.md").read_text() in report
    assert (out / "lcov.info").is_file() and (out / "lcov-measured.info").is_file()
    assert json.loads((out / "summary.json").read_text())["app"] == "desktop"
    catalog = report.split("### catalog coverage\n", 1)[1]
    assert re.search(r"^ui ids exercised by a passing test: \d+/\d+ \(\d+\.\d%\)$", catalog, re.MULTILINE), catalog


def test_a_coverage_run_on_a_stage_from_another_machine_reads_the_staged_programs(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    """A shard runs the stage a build job made: no target dir of the build, and no cargo-llvm-cov to ask."""
    target = tmp_path / "target"
    shutil.rmtree(built(stage, target).parent)
    instrument(stage / "bin" / "ngterm")
    fake_cargo.write_text(f"#!{sys.executable}\nimport sys\nsys.exit('cargo is not needed for a staged build')\n")
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage", LAUNCH_PROBE=str(tmp_path / "launch.txt"),
        CARGO_TARGET_DIR=str(target), FAKE_TERM="quit", FAKE_LLVM_COV_ENV="1",
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "**Total: 25.00% of lines (2/8)" in report
    assert (tmp_path / "report" / "coverage" / "lcov.info").is_file()


def test_a_coverage_run_without_profiles_exits_2_with_the_reason(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    instrument(stage / "bin" / "ngterm")
    target = tmp_path / "target"
    built(stage, target)
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage", CARGO_TARGET_DIR=str(target)
    )
    assert completed.returncode == 2, completed.stdout + completed.stderr
    profiles = target / "llvm-cov-target"
    assert f"code coverage: no profiles in {profiles}" in completed.stderr
    assert f"no report: no profiles in {profiles}" in report
    assert "ratchet holds" in report and "### catalog coverage" in report


def test_a_coverage_run_on_another_builds_stage_exits_2_with_the_reason(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    target = tmp_path / "target"
    built(stage, target).write_bytes(b"another instrumented build\n# LLVM Profile Error\n")
    instrument(stage / "bin" / "ngterm")
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage",
        LAUNCH_PROBE=str(tmp_path / "launch.txt"), CARGO_TARGET_DIR=str(target), FAKE_TERM="quit",
    )
    assert completed.returncode == 2, completed.stdout + completed.stderr
    reason = f"binary mismatch: {stage}/bin/ngterm is not {target}/llvm-cov-target/release/ngterm"
    assert f"code coverage: {reason}" in completed.stderr
    assert f"no report: {reason}" in report


def test_a_relative_target_dir_gives_the_instances_an_absolute_profile(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    instrument(stage / "bin" / "ngterm")
    built(stage, tmp_path / "target")
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    probe = tmp_path / "launch.txt"
    # relative to the runner's working directory (`command`'s runner runs from the test's cwd)
    target = os.path.relpath(tmp_path / "target", Path.cwd())
    completed, _ = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage",
        LAUNCH_PROBE=str(probe), CARGO_TARGET_DIR=target, FAKE_TERM="quit",
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert probe.read_text() == f"{tmp_path / 'target' / 'llvm-cov-target'}/conformance-%4m.profraw"


def test_a_run_without_coverage_names_no_profile(tmp_path: Path, stage: Path, layers: Path) -> None:
    (layers / "protocol" / "test_probe.py").write_text(LAUNCH_PROBE_TEST)
    probe = tmp_path / "launch.json"
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), LAUNCH_PROBE=str(probe),
        LLVM_PROFILE_FILE=str(tmp_path / "inherited-%p.profraw"),
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "LLVM_PROFILE_FILE" not in json.loads(probe.read_text())["env"]
    assert "- coverage:" not in report and not (tmp_path / "report" / "lost-profiles.txt").exists()
    assert "### code coverage" not in report and not (tmp_path / "report" / "coverage").exists()
    assert "### catalog coverage\n\nui ids exercised by a passing test: " in report


def test_the_catalog_line_counts_against_the_runs_catalog(tmp_path: Path, stage: Path, layers: Path) -> None:
    # a --catalog through the pytest args: one active ui id, which the passing test covers
    (layers / "protocol" / "test_probe.py").write_text(
        VERSION_TEST.replace("\ndef ", '\nimport pytest\n\n@pytest.mark.covers("ui:only-id")\ndef ', 1)
    )
    custom = tmp_path / "custom"
    custom.mkdir()
    (custom / "catalog.yaml").write_text("commands: {}\n")
    (custom / "ui-ids.yaml").write_text("ids:\n  only-id: {status: uncovered}\n")
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--", "--catalog", str(custom / "catalog.yaml")
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr
    assert "### catalog coverage\n\nui ids exercised by a passing test: 1/1 (100.0%)\n" in report, report


def test_a_coverage_run_on_a_stage_without_one_of_the_apps_programs_exits_2_naming_it(
    tmp_path: Path, stage: Path, layers: Path, fake_cargo: Path
) -> None:
    instrument(stage / "bin" / "ngterm")
    target = tmp_path / "target"
    built(stage, target)
    (target / "llvm-cov-target" / "release" / "ngterm-web").unlink()
    (layers / "protocol" / "test_probe.py").write_text(PROFILE_WRITING_TEST)
    completed, report = run(
        tmp_path, stage, layers, ratchet(tmp_path, PROBE_ID), "--coverage",
        LAUNCH_PROBE=str(tmp_path / "launch.txt"), CARGO_TARGET_DIR=str(target), FAKE_TERM="quit",
    )
    assert completed.returncode == 2, completed.stdout + completed.stderr
    reason = f"the desktop app's program ngterm-web is not in {target}/llvm-cov-target/release"
    assert f"code coverage: {reason}" in completed.stderr
    assert f"no report: {reason}" in report


@pytest.mark.parametrize("runner, platform", [("run-linux.sh", "linux"), ("run-windows.sh", "windows")])
def test_each_runner_reports_the_desktop_app_on_its_platform(runner: str, platform: str) -> None:
    # run-windows.sh runs on Windows alone; its report call is read from the script
    text = (SCRIPT.parent / runner).read_text()
    calls = re.findall(r"-m agterm_conformance\.code_coverage( [^\n]*)", text)
    assert calls == [f" e2e --app desktop --platform {platform} \\"], calls
