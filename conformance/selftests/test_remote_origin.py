"""The remote origin's harness (`agterm_conformance.remote_origin`): an origin on a private root behind an sshd that takes
loopback and keys alone, a viewer whose ssh reads only the origin's private files, a proxy whose links a test cuts, a
stop that ends the sshd, then the origin and its sessions through its own server, and a missing tool that fails a CI run
but skips elsewhere.

The fake app stands in for the origin. Most tests stand in for sshd too (a program that only sleeps), so they need
ssh-keygen alone; the last one logs in through a real sshd where the machine has one."""

import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

import pytest
from agterm_conformance import loopback_ssh, remote_origin
from agterm_conformance.impls import base
from agterm_conformance.remote_origin import ORIGIN_HOST, RemoteOrigin, SeveringProxy
from upstream_adapter import UpstreamVocabularyAdapter

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="the remote origin is POSIX only")

def _keygen() -> str:
    keygen = shutil.which("ssh-keygen")
    if keygen is None:
        pytest.skip("no ssh-keygen on this machine")
    return keygen


@pytest.fixture
def fake_ctl(tmp_path: Path) -> Path:
    """The CLI the origin's HOME links: prints the state dir it answers for and the profile destination it writes to."""
    ctl = tmp_path / "stage" / "agtermctl"
    ctl.parent.mkdir()
    ctl.write_text('#!/bin/sh\nprintf \'%s\\n\' "ctl $*" "${AGTERM_STATE_DIR:-}" "${LLVM_PROFILE_FILE:-none}"\n')
    ctl.chmod(0o755)
    return ctl


@pytest.fixture
def stand_in_sshd(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """sshd as a program that records its arguments and sleeps, taken for serving on its port; ssh is the system's or a
    name nothing runs, and the client's first login is skipped, as nothing answers it."""
    program = tmp_path / "sshd-stand-in" / "sshd"
    program.parent.mkdir()
    record = program.with_name("argv")
    program.write_text(f"#!/bin/sh\nprintf '%s\\n' \"$@\" > '{record}'\nexec sleep 300\n")
    program.chmod(0o755)
    keygen = _keygen()
    ssh = shutil.which("ssh") or "/usr/bin/ssh"
    monkeypatch.setattr(
        remote_origin, "find_openssh", lambda: ({"sshd": str(program), "ssh": ssh, "ssh-keygen": keygen}, None)
    )
    monkeypatch.setattr(loopback_ssh, "_banner", lambda port: True)
    monkeypatch.setattr(loopback_ssh.SshClient, "warm", lambda self, machine: None)
    return program


@pytest.fixture
def adapter(fake_bin: Path, fake_ctl: Path) -> UpstreamVocabularyAdapter:
    return UpstreamVocabularyAdapter(fake_bin, fake_ctl)


@pytest.fixture
def origin(roots: list[Path], adapter, tmp_path: Path, stand_in_sshd: Path):
    started = RemoteOrigin(adapter, tmp_path / "world")
    try:
        yield started.start()
    finally:
        started.stop()


def _config(origin: RemoteOrigin) -> list[str]:
    assert origin.machine is not None
    return (origin.machine.directory / "sshd_config").read_text().splitlines()


def test_the_origins_sshd_listens_on_loopback_alone_and_takes_the_viewers_key_alone(origin, stand_in_sshd):
    lines = _config(origin)
    assert [line for line in lines if line.startswith(("ListenAddress", "Port"))] == [
        "ListenAddress 127.0.0.1",
        f"Port {origin.machine.port}",
    ]
    assert {
        "AuthenticationMethods publickey",
        "PubkeyAuthentication yes",
        "PasswordAuthentication no",
        "KbdInteractiveAuthentication no",
        "PermitUserRC no",
        "PermitUserEnvironment no",
    } <= set(lines)
    authorized = origin.machine.directory / "authorized_keys"
    assert f'AuthorizedKeysFile "{authorized}"' in lines
    assert authorized.read_text() == origin.client.public_key + "\n"
    # sshd ran on that config alone
    record = stand_in_sshd.with_name("argv")
    _wait(lambda: record.exists() and record.read_text().endswith("\n"))
    argv = record.read_text().splitlines()
    assert argv[:3] == ["-D", "-f", str(origin.machine.directory / "sshd_config")]


def test_a_login_gets_the_origins_home_and_state_dir_and_its_cli_on_the_path_ssh_adds(origin, adapter):
    instance = origin.instance
    assert origin.machine.home == instance.home
    link = instance.home / ".local" / "bin" / adapter.ctl_name
    assert link.resolve() == adapter.ctl_path.resolve()
    # the viewer's lines put `$HOME/.local/bin` on PATH themselves (crates/ngterm-core/src/remote/ssh.rs)
    command = """'/bin/sh' '-c' 'echo "$HOME"; PATH="$PATH:$HOME/.local/bin" && agtermctl version'"""
    result = subprocess.run(
        [origin.machine.directory / "force-command"],
        env={"SSH_ORIGINAL_COMMAND": command, "AGTERM_STATE_DIR": "/leaked", "LLVM_PROFILE_FILE": "/leaked"},
        capture_output=True,
        text=True,
        check=True,
    )
    assert result.stdout.splitlines() == [str(instance.home), "ctl version", str(instance.state_dir), "none"]


def test_a_coverage_runs_profile_destination_reaches_the_login(
    roots, adapter, tmp_path, stand_in_sshd, monkeypatch: pytest.MonkeyPatch
):
    monkeypatch.setenv("CONFORMANCE_COVERAGE_DIR", str(tmp_path / "profiles"))
    started = RemoteOrigin(adapter, tmp_path / "world")
    try:
        started.start()
        assert started.login_env() == {
            "AGTERM_STATE_DIR": str(started.instance.state_dir),
            "LLVM_PROFILE_FILE": str(tmp_path / "profiles" / "conformance-%4m.profraw"),
        }
    finally:
        started.stop()


def test_the_viewers_ssh_reads_only_the_origins_private_files(origin, adapter):
    client = origin.client
    assert origin.viewer_path == [client.bin]
    assert sorted(path.name for path in client.bin.iterdir()) == ["ssh"]
    instance = adapter.launch(path_prefix=origin.viewer_path)
    try:
        env = json.loads((instance.state_dir / "fake-env.json").read_text())
    finally:
        adapter.stop(instance)
    assert env["PATH"].split(os.pathsep)[0] == str(client.bin)
    if shutil.which("ssh") is None:
        pytest.skip("no ssh on this machine to resolve the client's config")
    resolved = subprocess.run(
        [client.bin / "ssh", "-G", ORIGIN_HOST], capture_output=True, text=True, check=True, stdin=subprocess.DEVNULL
    ).stdout.splitlines()
    settings = {}
    for line in resolved:
        key, _, value = line.partition(" ")
        settings.setdefault(key, []).append(value)
    assert settings["hostname"] == ["127.0.0.1"]
    # through the proxy, so a test can cut the link
    assert settings["port"] == [str(origin.proxy.port)]
    assert settings["identityfile"] == [str(client.identity)]
    assert settings["userknownhostsfile"] == [str(client.known_hosts)]
    assert settings["globalknownhostsfile"] == ["/dev/null"]
    assert settings["identityagent"] == ["none"]
    assert settings["stricthostkeychecking"] == ["true"]
    assert client.known_hosts.read_text() == f"{ORIGIN_HOST} {origin.machine.host_public}\n"
    own_ssh = os.path.join(os.path.expanduser("~"), ".ssh")
    assert not [line for line in resolved if own_ssh in line]


def _listen() -> socket.socket:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen()
    return listener


def _accept(listener: socket.socket, accepted: list[socket.socket]) -> None:
    while True:
        try:
            stream, _ = listener.accept()
        except OSError:
            return
        accepted.append(stream)


def _connect(port: int) -> socket.socket:
    stream = socket.create_connection(("127.0.0.1", port), timeout=5)
    return stream


def _wait(condition, timeout: float = 5.0) -> None:
    deadline = time.monotonic() + timeout
    while not condition():
        assert time.monotonic() < deadline, "timed out"
        time.sleep(0.02)


def test_the_proxy_carries_bytes_and_severing_cuts_open_links_but_not_later_ones():
    listener = _listen()
    accepted: list[socket.socket] = []
    threading.Thread(target=_accept, args=(listener, accepted), daemon=True).start()
    proxy = SeveringProxy(listener.getsockname()[1])
    try:
        first = _connect(proxy.port)
        _wait(lambda: len(accepted) == 1)
        first.sendall(b"ping")
        assert accepted[0].recv(4) == b"ping"
        accepted[0].sendall(b"pong")
        assert first.recv(4) == b"pong"

        proxy.sever()
        assert first.recv(4) == b""
        assert accepted[0].recv(4) == b""
        _wait(lambda: proxy.connections() == 0)

        second = _connect(proxy.port)
        _wait(lambda: len(accepted) == 2)
        second.sendall(b"again")
        assert accepted[1].recv(5) == b"again"
    finally:
        proxy.stop()
        listener.close()
    with pytest.raises(OSError):
        _connect(proxy.port).recv(1)


def test_stop_ends_the_proxy_and_sshd_then_the_origin_reaping_its_sessions_through_its_own_server(
    roots, adapter, tmp_path, stand_in_sshd, monkeypatch: pytest.MonkeyPatch
):
    started = RemoteOrigin(adapter, tmp_path / "world").start()
    instance, sshd, proxy = started.instance, started.machine._process, started.proxy
    sleeper = subprocess.Popen(["sleep", "300"])
    try:
        assert instance.request("fake.adopt", pid=sleeper.pid).ok
        seen = {}
        stop = adapter.stop

        def recording(stopped, cleanup=True):
            seen["sshd"] = sshd.poll()
            seen["proxy"] = proxy._stopping.is_set()
            stop(stopped, cleanup=False)
            seen["requests"] = (stopped.state_dir / "fake-requests.jsonl").read_text().splitlines()
            base.remove_live_dir(stopped.live_dir)

        monkeypatch.setattr(adapter, "stop", recording)
        started.stop()
        assert seen["sshd"] is not None and seen["proxy"], seen
        assert sleeper.wait(timeout=10) is not None
        assert instance.process.poll() is not None
        requests = [json.loads(line) for line in seen["requests"]]
        assert any(request["cmd"] == "zmx.kill" for request in requests), requests
        assert (started.instance, started.machine, started.proxy) == (None, None, None)
    finally:
        sleeper.kill()
        sleeper.wait()


def test_stop_after_a_failed_start_ends_what_had_started(roots, adapter, tmp_path, stand_in_sshd, monkeypatch):
    def no_port(self, target):
        raise OSError("no port")

    monkeypatch.setattr(SeveringProxy, "__init__", no_port)
    started = RemoteOrigin(adapter, tmp_path / "world")
    with pytest.raises(OSError, match="no port"):
        started.start()
    sshd, instance = started.machine._process, started.instance
    started.stop()
    assert sshd.poll() is not None
    assert instance.process.poll() is not None


def test_a_headless_origin_runs_the_staged_ngterm_headless_without_a_display(
    roots, fake_bin, fake_ctl, tmp_path, stand_in_sshd
):
    headless = fake_bin.with_name("ngterm-headless")
    shutil.copy2(fake_bin, headless)
    adapter = UpstreamVocabularyAdapter(fake_bin, fake_ctl)
    started = RemoteOrigin(adapter, tmp_path / "world", "headless")
    try:
        started.start()
        instance = started.instance
        assert instance.process.args == [str(headless), "--skip-agent-setup"]
        assert (instance.compositor, instance.session_bus) == (None, None)
        assert instance.state_dir.is_relative_to(instance.root) and instance.home.is_relative_to(instance.root)
        assert started.machine.home == instance.home
    finally:
        started.stop()


def _missing_sshd(monkeypatch: pytest.MonkeyPatch) -> None:
    real = loopback_ssh.find_program
    monkeypatch.setattr(
        loopback_ssh, "find_program", lambda name, extra=(): None if name == "sshd" else real(name, extra)
    )


def test_missing_sshd_ends_a_ci_run_with_exit_2_naming_the_step(roots, adapter, tmp_path, monkeypatch):
    _missing_sshd(monkeypatch)
    monkeypatch.setenv("CI", "true")
    with pytest.raises(pytest.exit.Exception) as raised:
        RemoteOrigin(adapter, tmp_path / "world").start()
    assert raised.value.returncode == 2
    assert str(raised.value.msg).startswith("remote origin: start the origin's sshd: no sshd on this machine")
    assert roots == []


def test_missing_sshd_skips_a_local_run_with_the_reason(roots, adapter, tmp_path, monkeypatch):
    _missing_sshd(monkeypatch)
    monkeypatch.delenv("CI", raising=False)
    with pytest.raises(pytest.skip.Exception, match="remote origin: start the origin's sshd: no sshd on this machine"):
        RemoteOrigin(adapter, tmp_path / "world").start()
    assert roots == []


def test_a_missing_ngterm_headless_ends_a_ci_run_naming_the_step(roots, adapter, tmp_path, stand_in_sshd, monkeypatch):
    monkeypatch.setenv("CI", "true")
    with pytest.raises(pytest.exit.Exception) as raised:
        RemoteOrigin(adapter, tmp_path / "world", "headless").start()
    assert raised.value.returncode == 2
    assert str(raised.value.msg).startswith("remote origin: launch the staged ngterm-headless: no executable")
    assert roots == []


def test_an_unknown_kind_is_refused(adapter, tmp_path):
    with pytest.raises(ValueError, match="one of instance, headless"):
        RemoteOrigin(adapter, tmp_path, "agterm")


def _ssh(origin: RemoteOrigin, *command: str, **options) -> subprocess.Popen[str]:
    return subprocess.Popen(
        [origin.client.bin / "ssh", "-T", "-o", "BatchMode=yes", ORIGIN_HOST, *command],
        env={"PATH": "/usr/bin:/bin", "HOME": str(origin.client.directory)},
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        **options,
    )


def test_through_a_real_sshd_a_login_answers_for_the_origin_and_a_severed_link_drops_only_open_sessions(
    roots, adapter, tmp_path
):
    _found, reason = loopback_ssh.find_openssh()
    if reason is not None:
        pytest.skip(reason)
    started = RemoteOrigin(adapter, tmp_path / "world")
    try:
        started.start()
        line = """'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin" && agtermctl live tree'"""
        stdout, stderr = _ssh(started, line).communicate(timeout=60)
        assert stdout.splitlines()[:2] == ["ctl live tree", str(started.instance.state_dir)], stderr

        held = _ssh(started, "sleep 60")
        _wait(lambda: started.proxy.connections() == 1, timeout=30)
        started.sever()
        assert held.wait(timeout=30) != 0
        held.communicate()

        stdout, stderr = _ssh(started, "echo again").communicate(timeout=60)
        assert stdout == "again\n", stderr
    finally:
        started.stop()
