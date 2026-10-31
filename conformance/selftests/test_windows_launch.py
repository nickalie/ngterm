"""The Rust adapter's Windows launch and stop, on a Windows host (the `windows` CI job runs these): the instance's
environment and named pipe, the quit asked of its window, the kill and report of an app that does not quit, and the
process tree reaped by pid. The fake app (fake_app.py) stands in for the Rust build, run by this Python."""

import json
import os
import sys
from collections.abc import Iterator
from pathlib import Path

import pytest
from agterm_conformance import transport, winproc
from agterm_conformance.impls import base
from agterm_conformance.impls.base import Instance, StopError
from agterm_conformance.impls.rust import RustAdapter
from agterm_conformance.vocabulary import LIVE, fnv1a64

pytestmark = pytest.mark.skipif(sys.platform != "win32", reason="the Windows launch")

FAKE_APP = Path(__file__).with_name("fake_app.py")


# The interpreter itself: a venv's python.exe is a launcher that runs it as a child, which would leave the launched
# pid without the fake's window.
PYTHON = getattr(sys, "_base_executable", None) or sys.executable


class FakeRust(RustAdapter):
    """The Rust adapter running fake_app.py through this Python: Windows runs no script by its shebang."""

    def executable(self) -> list[str]:
        return [PYTHON, os.fspath(FAKE_APP)]


@pytest.fixture
def adapter(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> FakeRust:
    shell = tmp_path / "Git" / "bin" / "sh.exe"
    shell.parent.mkdir(parents=True)
    shell.write_bytes(b"")
    monkeypatch.setenv(base.POSIX_SHELL_VARIABLE, os.fspath(shell))
    monkeypatch.setenv("FAKE_LIVE_GROUP", "live")
    # the fake imports the harness's transport
    monkeypatch.setenv("PYTHONPATH", os.fspath(Path(__file__).resolve().parents[1]))
    fake = FakeRust(tmp_path / "stage" / "bin" / "ngterm.exe")
    fake._profile = base.NO_PROFILE
    return fake


@pytest.fixture
def launched(adapter: FakeRust, roots: list[Path]) -> Iterator[Instance]:
    instance = adapter.launch()
    yield instance
    if instance.process.poll() is None:
        adapter.stop(instance)


def fake_env(instance: Instance) -> dict[str, str]:
    return json.loads((instance.state_dir / "fake-env.json").read_text())


def test_the_instance_lives_under_its_root_and_serves_the_pipe_its_path_names(launched: Instance) -> None:
    env = fake_env(launched)
    root = launched.root.resolve()
    for name in ("HOME", "LOCALAPPDATA", "APPDATA", "NGTERM_STATE_DIR", "NGTERM_CONTROL_SOCKET"):
        assert Path(env[name]).resolve().is_relative_to(root), name
    assert env["PATH"] == base.MINIMAL_PATH
    assert env["NGTERM_APP_ID"].startswith(base.DEFAULT_APP_ID + ".conformance.")
    assert not any(name.startswith(("AGTERM_", "ZMX_")) for name in env)
    config = (launched.state_dir / "config" / "ghostty.conf").read_text()
    assert config.startswith(f"command = {os.environ[base.POSIX_SHELL_VARIABLE]}\n")
    # Git's sh is an interactive bash that is no login shell: its prompt comes from ~/.bashrc
    assert (launched.home / ".bashrc").read_text() == "PS1='$ '\n"
    assert not launched.socket.exists()
    assert launched.request("version").ok
    with transport.connect(launched.socket, 5) as conn:
        conn.sendall(b'{"cmd":"version"}\n')
        assert b'"ok": true' in conn.recv(65536)


def test_the_live_namespace_is_under_the_instances_local_app_data(launched: Instance) -> None:
    local = fake_env(launched)["LOCALAPPDATA"]
    digest = fnv1a64(os.path.realpath(launched.state_dir))
    assert launched.live_dir == f"{local}\\ngterm\\live\\{digest}"
    assert LIVE.socket_directory(launched.state_dir, "ngterm", launched.env) == launched.live_dir


def test_the_stop_asks_the_quit_of_the_apps_window(adapter: FakeRust, roots: list[Path]) -> None:
    instance = adapter.launch()
    adapter.stop(instance)
    assert instance.process.returncode == 0
    assert not instance.root.exists()


def test_an_app_ignoring_the_sessions_end_is_ended_and_reported(
    adapter: FakeRust, roots: list[Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_APP_MODE", "ignore-term")
    monkeypatch.setattr(base, "STOP_TIMEOUT", 1.0)
    instance = adapter.launch()
    with pytest.raises(StopError, match="did not exit within 1.0s of the session's end; ended by its pid"):
        adapter.stop(instance)
    assert instance.process.poll() is not None
    assert (instance.state_dir / "fake-signals").read_text() == "ENDSESSION\n"


def test_an_app_without_its_quit_window_is_ended_and_reported(
    adapter: FakeRust, roots: list[Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_APP_MODE", "no-window")
    monkeypatch.setattr(base, "QUIT_WINDOW_WAIT", 0.3)
    instance = adapter.launch()
    with pytest.raises(StopError, match="showed no ngterm-system-window window to ask the quit of"):
        adapter.stop(instance)
    assert instance.process.poll() is not None


def test_the_stop_reaps_the_apps_process_tree_by_pid(
    adapter: FakeRust, roots: list[Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_CHILD", "1")
    monkeypatch.setattr(base, "REAP_TREE_GRACE", 0.3)
    instance = adapter.launch()
    pid = int((instance.state_dir / "fake-child").read_text())
    child = winproc.Process(pid, instance.pid, "python.exe", winproc.creation_time(pid))
    assert child in winproc.descendants([winproc.Process(instance.pid, 0, "", instance.launched[0][1])])
    adapter.stop(instance)
    assert not winproc.alive(child)


def test_the_stop_reaps_a_program_whose_launcher_left(
    adapter: FakeRust, roots: list[Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_CHILD", "orphan")
    monkeypatch.setattr(base, "REAP_TREE_GRACE", 0.3)
    instance = adapter.launch()
    pid = int((instance.state_dir / "fake-grandchild").read_text())
    orphan = winproc.Process(pid, 0, "python.exe", winproc.creation_time(pid))
    # its parent is gone: the process table no longer leads from the app to it
    app = winproc.Process(instance.pid, 0, "", instance.launched[0][1])
    assert orphan.pid not in {process.pid for process in winproc.descendants([app])}
    adapter.stop(instance)
    assert not winproc.alive(orphan)


def test_a_relaunch_runs_again_and_the_stop_reaps_both_trees(
    adapter: FakeRust, roots: list[Path], monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_CHILD", "1")
    monkeypatch.setattr(base, "REAP_TREE_GRACE", 0.3)
    first = adapter.launch()
    first_child = int((first.state_dir / "fake-child").read_text())
    relaunched = adapter.relaunch(first)
    assert first.process.returncode == 0
    assert [process.pid for process, _ in relaunched.launched] == [first.pid, relaunched.pid]
    second_child = int((relaunched.state_dir / "fake-child").read_text())
    children = [winproc.Process(pid, 0, "", winproc.creation_time(pid)) for pid in (first_child, second_child)]
    adapter.stop(relaunched)
    assert not any(winproc.alive(child) for child in children)


def test_a_launch_beside_outliving_its_timeout_is_ended_by_pid(
    launched: Instance, adapter: FakeRust, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("FAKE_APP_MODE", "silent")
    beside = adapter.launch_beside(launched, timeout=2)
    assert beside.returncode is None


def test_the_leftover_check_finds_a_session_server_by_its_argv_and_its_sessions(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    live_dir = r"C:\Users\u\AppData\Local\ngterm\live\0123456789abcdef"
    table = [
        winproc.Process(10, 1, "ngterm-session.exe", 100),
        winproc.Process(11, 10, "conhost.exe", 101),
        winproc.Process(12, 10, "sh.exe", 102),
        winproc.Process(13, 12, "cat.exe", 103),
        # a pid reused after the server started: not its child
        winproc.Process(14, 10, "other.exe", 50),
        winproc.Process(20, 1, "NGTERM-SESSION.EXE", 100),
        winproc.Process(30, 1, "ngterm-session.exe", 100),
    ]
    argv = {
        10: [r"C:\stage\bin\ngterm-session.exe", "server", live_dir],
        20: [r"C:\stage\bin\NGTERM-SESSION.EXE", "server", live_dir.upper()],
        30: [r"C:\stage\bin\ngterm-session.exe", "server", live_dir + "0"],
    }
    monkeypatch.setattr(winproc, "processes", lambda: table)
    monkeypatch.setattr(winproc, "command_line", argv.get)
    found = sorted(pid for pid, _, _ in base.find_leftovers(live_dir, LIVE))
    assert found == [10, 11, 12, 13, 20]


def test_posix_shell_is_found_and_a_missing_one_refused(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    shell = tmp_path / "sh.exe"
    shell.write_bytes(b"")
    assert base.posix_shell({base.POSIX_SHELL_VARIABLE: os.fspath(shell), "PATH": ""}) == os.fspath(shell)
    monkeypatch.setattr(base, "GIT_SHELL_DEFAULT", os.fspath(tmp_path / "none" / "sh.exe"))
    with pytest.raises(base.LaunchError, match="no POSIX sh for the panes"):
        base.posix_shell({"PATH": os.fspath(tmp_path / "empty")})
