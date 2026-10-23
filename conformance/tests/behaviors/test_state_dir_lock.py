"""One instance owns a state directory (upstream #749, `StateDirectoryLock`): a second process started on it, with an
app id and a control socket of its own so nothing forwards or refuses it earlier, exits 1 at launch before it reads or
writes the directory, and the owner's lock goes with the owner's process."""

import time
from pathlib import Path

import pytest
from agterm_conformance.match import assert_ok, session_ids, window_ids, workspace_ids
from agterm_conformance.wait import wait_until

# "Within seconds": the refusal comes before the app loads anything, let alone opens a window.
REFUSAL_TIMEOUT = 15.0
SETTLE = 1.0


def _window_files(state_dir: Path) -> dict[str, bytes]:
    """`windows.json` and every window file, by path relative to the state dir."""
    paths = [state_dir / "windows.json", *sorted((state_dir / "windows").glob("*.json"))]
    return {path.relative_to(state_dir).as_posix(): path.read_bytes() for path in paths if path.is_file()}


class _Settled:
    """A `wait_until` check: the window files once one of them holds `session_id` and none has changed for SETTLE
    seconds, longer than the debounce of a selection's save (0.3s upstream), so no save of the owner is pending."""

    def __init__(self, state_dir: Path, session_id: str) -> None:
        self.state_dir = state_dir
        self.needle = session_id.lower().encode()
        self.seen: dict[str, bytes] | None = None
        self.since = 0.0

    def __call__(self) -> dict[str, bytes] | None:
        files = _window_files(self.state_dir)
        if not any(self.needle in content.lower() for content in files.values()):
            return None
        now = time.monotonic()
        if files != self.seen:
            self.seen, self.since = files, now
            return None
        return files if now - self.since >= SETTLE else None


def _model(ctl) -> tuple[list[str], list[str], list[str]]:
    tree = ctl.tree()
    return window_ids(ctl.request("window.list")), workspace_ids(tree), session_ids(tree)


@pytest.mark.covers("behavior:app.state-dir-owned")
def test_a_second_instance_on_an_owned_state_dir_exits_1_without_writing(launch, adapter):
    ctl = launch()
    owner = ctl.instance
    sid = assert_ok(ctl.request("session.new", name="owned"))["id"]
    ctl.wait_mounted(sid, "left")
    # The owner's saves of the new session have landed.
    files = wait_until(_Settled(owner.state_dir, sid))
    model = _model(ctl)

    second = adapter.launch_beside(owner, timeout=REFUSAL_TIMEOUT)

    assert second.returncode == 1, second.stderr
    expected = (
        f"{adapter.app_dir_name}: another instance is already running on {owner.state_dir}; "
        f"control it with {adapter.ctl_name}"
    )
    assert expected in second.stderr.splitlines(), second.stderr
    assert not second.socket.exists()
    assert _window_files(owner.state_dir) == files
    assert _model(ctl) == model


@pytest.mark.covers("behavior:app.state-dir-owned")
def test_the_lock_is_released_at_exit(launch, adapter):
    ctl = launch()
    lock = ctl.instance.state_dir / f"{adapter.app_dir_name}.lock"
    assert lock.is_file()
    # The relaunch waits for the old process to exit, then for the new one to serve: the next owner got the lock.
    ctl = launch.relaunch(ctl)
    assert_ok(ctl.request("window.list"))
    assert lock.is_file()
