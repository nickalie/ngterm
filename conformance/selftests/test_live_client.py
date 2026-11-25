import os
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from agterm_conformance.live_client import GONE_GUARD, WINDOWS_GONE_GUARD, leading_client
from agterm_conformance.piped_process import PipedProcess
from agterm_conformance.wait import wait_until
from agterm_conformance.pty_process import PtyProcess


def _script(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "ngterm-session"
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
    return path


def _printed(process: PtyProcess | PipedProcess, text: str) -> None:
    wait_until(lambda: text in process.output() or process.process.poll() is not None, timeout=5)
    assert text in process.output(), process.output()


@pytest.fixture
def instance(tmp_path: Path) -> SimpleNamespace:
    return SimpleNamespace(live_dir=str(tmp_path / "ngterm-live-x"), home=tmp_path / "home")


def test_leading_client_attaches_under_the_attach_contract_claiming_the_lead(
    tmp_path: Path, instance: SimpleNamespace
) -> None:
    fake = _script(
        tmp_path,
        'printf "%s|" "$@"; echo\n'
        'echo "env $NGTERM_LIVE_DIR $NGTERM_LIVE_MANAGED_CLAIM $NGTERM_LIVE_NO_DETACH_KEY '
        '[${NGTERM_LIVE_SESSION-unset}] [${NGTERM_LIVE_SESSION_PREFIX-unset}]"\n'
        'echo "nonce ${#NGTERM_LIVE_MANAGED} home $HOME"\n'
        "exec cat\n",
    )
    endpoint = {"executable": str(fake), "socketDirectory": instance.live_dir}
    with leading_client(instance, endpoint, "ngterm-abc") as client:
        _printed(client, f"attach|ngterm-abc|/bin/sh|-c|{GONE_GUARD}|")
        _printed(client, f"env {instance.live_dir} 1 1 [] []")
        _printed(client, f"nonce 32 home {instance.home}")


def test_leading_client_refuses_another_instances_namespace(instance: SimpleNamespace) -> None:
    with pytest.raises(ValueError, match="not this instance's"):
        leading_client(
            instance, {"executable": "/bin/true", "socketDirectory": "/tmp/ngterm-live-other"}, "ngterm-abc"
        )


def test_leading_client_refuses_a_relative_executable(instance: SimpleNamespace) -> None:
    with pytest.raises(ValueError, match="not absolute"):
        leading_client(instance, {"executable": "ngterm-session", "socketDirectory": instance.live_dir}, "ngterm-abc")


def test_piped_process_echoes_its_input_and_exits_on_its_own_when_stopped() -> None:
    process = PipedProcess(["cat"], {"PATH": os.environ["PATH"]})
    assert process.process.stdin is not None
    process.process.stdin.write(b"hello pipe\n")
    process.process.stdin.flush()
    _printed(process, "hello pipe")
    process.stop()
    assert process.process.returncode == 0


def test_leading_client_on_windows_attaches_on_pipes_without_the_instances_app_variables(
    tmp_path: Path, instance: SimpleNamespace, monkeypatch: pytest.MonkeyPatch
) -> None:
    fake = _script(
        tmp_path,
        'printf "%s|" "$@"; echo\n'
        'echo "env [${NGTERM_SOCKET-unset}] [$KEPT] $NGTERM_LIVE_DIR $NGTERM_LIVE_MANAGED_CLAIM"\n'
        "exec cat\n",
    )
    instance.env = {"NGTERM_SOCKET": "/tmp/the-apps-socket", "KEPT": "kept", "PATH": "/nowhere"}
    endpoint = {"executable": str(fake), "socketDirectory": instance.live_dir}
    monkeypatch.setattr(sys, "platform", "win32")
    with leading_client(instance, endpoint, "ngterm-abc") as client:
        assert isinstance(client, PipedProcess)
        _printed(client, "attach|ngterm-abc|" + "|".join(WINDOWS_GONE_GUARD) + "|")
        _printed(client, f"env [unset] [kept] {instance.live_dir} 1")
    assert client.process.returncode == 0
