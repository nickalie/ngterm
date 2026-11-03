"""The Swift reference's own leading client (impls/swift_macos.py): a second `zmx attach` claiming the lead."""

from pathlib import Path
from types import SimpleNamespace

import pytest
from agterm_conformance.impls.swift_macos import GONE_GUARD, leading_client
from agterm_conformance.pty_process import PtyProcess
from agterm_conformance.wait import wait_until


def _script(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "prog"
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
    return path


def _printed(process: PtyProcess, text: str) -> None:
    wait_until(lambda: text in process.output() or process.process.poll() is not None, timeout=5)
    assert text in process.output(), process.output()


@pytest.fixture
def instance(tmp_path: Path) -> SimpleNamespace:
    return SimpleNamespace(live_dir=str(tmp_path / "zmx"), home=tmp_path / "home")


def test_leading_client_attaches_claiming_the_lead(tmp_path: Path, instance: SimpleNamespace) -> None:
    fake_zmx = _script(
        tmp_path,
        'printf "%s|" "$@"; echo\n'
        'echo "env $ZMX_DIR $ZMX_MANAGED_CLAIM $ZMX_NO_DETACH_KEY [${ZMX_SESSION-unset}] [${ZMX_SESSION_PREFIX-unset}]"\n'
        'echo "nonce ${#ZMX_MANAGED} home $HOME"\n'
        "exec cat\n",
    )
    endpoint = {"executable": str(fake_zmx), "socketDirectory": instance.live_dir}
    with leading_client(instance, endpoint, "agterm-abc") as client:
        _printed(client, f"attach|agterm-abc|/bin/sh|-c|{GONE_GUARD}|")
        _printed(client, f"env {instance.live_dir} 1 1 [] []")
        _printed(client, f"nonce 32 home {instance.home}")


def test_leading_client_refuses_another_instances_daemons(instance: SimpleNamespace) -> None:
    with pytest.raises(ValueError, match="not this instance's"):
        leading_client(instance, {"executable": "/bin/true", "socketDirectory": "/tmp/agterm-zmx-other"}, "agterm-abc")


def test_leading_client_refuses_a_relative_executable(instance: SimpleNamespace) -> None:
    with pytest.raises(ValueError, match="not absolute"):
        leading_client(instance, {"executable": "zmx", "socketDirectory": instance.live_dir}, "agterm-abc")
