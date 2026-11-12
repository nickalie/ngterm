"""The `agterm-peer` tests' harness, host-free: the loopback machine's sshd and ssh client configs reach nothing of the
account's, and the stand-in agterm answers as upstream does (`fake_agterm`), run as the ssh session would run it."""

import json
import subprocess
import sys
import uuid

import pytest
from agterm_conformance import fake_agterm
from agterm_conformance.loopback_ssh import LoopbackMachine, SshClient

pytestmark = pytest.mark.skipif(sys.platform == "win32", reason="the loopback machine is POSIX only")


def test_the_sshd_config_listens_on_loopback_alone_and_takes_keys_only(tmp_path):
    machine = LoopbackMachine(tmp_path, "box", "ssh-ed25519 AAAA test")
    machine.port = 2222
    text = machine._config_text(tmp_path / "hk", tmp_path / "ak", tmp_path / "force")
    lines = text.splitlines()
    assert "ListenAddress 127.0.0.1" in lines and "Port 2222" in lines
    assert {"AuthenticationMethods publickey", "PasswordAuthentication no", "KbdInteractiveAuthentication no"} <= set(
        lines
    )
    assert {"PermitUserRC no", "PermitUserEnvironment no"} <= set(lines)
    assert f'AuthorizedKeysFile "{tmp_path / "ak"}"' in lines
    assert f"ForceCommand {tmp_path / 'force'}" in lines
    # the shell sshd starts the forced command with reads the machine's startup files, not the account's
    assert f'SetEnv "HOME={machine.home}"' in lines


COMMAND = """'/bin/sh' '-c' 'echo "$HOME"; echo "$PATH"; echo "${LEAK:-none}"'"""


def test_the_forced_command_runs_the_clients_command_in_the_machines_home_and_nothing_inherited(tmp_path):
    machine = LoopbackMachine(tmp_path, "box", "key")
    script = machine._write_force_command()
    result = subprocess.run(
        [script],
        # quoted for any login shell, as the viewer's lines are
        env={"SSH_ORIGINAL_COMMAND": COMMAND, "LEAK": "x"},
        capture_output=True,
        text=True,
        check=True,
    )
    assert result.stdout.splitlines() == [str(machine.home), "/usr/bin:/bin", "none"]


def test_the_ssh_stand_in_reads_only_its_own_config(tmp_path):
    keygen = subprocess.run(["sh", "-c", "command -v ssh-keygen"], capture_output=True, text=True).stdout.strip()
    if not keygen:
        pytest.skip("no ssh-keygen on this machine")
    client = SshClient(tmp_path, "/usr/bin/ssh", keygen)
    machine = LoopbackMachine(tmp_path, "box", client.public_key)
    machine.port, machine.host_public = 2222, "ssh-ed25519 AAAAhost"
    client.add(machine)
    config = client.config.read_text()
    assert "Host box\n  HostName 127.0.0.1\n  Port 2222\n" in config
    assert "IdentitiesOnly yes" in config and "IdentityAgent none" in config
    assert "GlobalKnownHostsFile /dev/null" in config and "StrictHostKeyChecking yes" in config
    assert client.known_hosts.read_text() == "box ssh-ed25519 AAAAhost\n"
    wrapper = (client.bin / "ssh").read_text()
    assert "/usr/bin/ssh -F " in wrapper and str(client.config) in wrapper, wrapper


def _machine(tmp_path, sessions, daemons):
    state = {
        "running": True,
        "restore": "live",
        "home": "/Users/me",
        "zmx": "/Applications/agterm.app/zmx",
        "socketDirectory": "/tmp/zmx",
        "window": {"id": "W", "name": "Main"},
        "workspace": {"id": "S", "name": "work"},
        "sessions": sessions,
        "daemons": daemons,
    }
    fake_agterm.save_state(tmp_path, state)
    return tmp_path


def _run(directory, *argv, input=None):
    return subprocess.run(
        [sys.executable, "-I", fake_agterm.__file__, str(directory), *argv],
        input=input,
        capture_output=True,
        text=True,
        timeout=30,
    )


def _session(name, daemon):
    return {"id": str(uuid.uuid4()).upper(), "name": name, "cwd": "/r", "panes": [{"pane": "left", "daemon": daemon}]}


def test_the_listing_offers_only_sessions_whose_daemons_run(tmp_path):
    live, gone = f"agterm-{uuid.uuid4().hex}", f"agterm-{uuid.uuid4().hex}"
    kept, dropped = _session("kept", live), _session("dropped", gone)
    directory = _machine(tmp_path, [kept, dropped], [live])
    answer = json.loads(_run(directory, "agtermctl", "zmx", "tree", "--json").stdout)
    remote = answer["result"]["remote"]
    assert answer["ok"] is True and remote["presentation"] == 1
    assert remote["endpoint"] == {"executable": "/Applications/agterm.app/zmx", "socketDirectory": "/tmp/zmx"}
    assert [s["id"] for s in remote["sessions"]] == [kept["id"]]
    assert json.loads((directory / "listing.json").read_text()) == answer


def test_the_stream_answers_hello_with_the_shared_kinds_and_the_snapshot_then_grants_the_presenter(tmp_path):
    daemon = f"agterm-{uuid.uuid4().hex}"
    session = _session("s", daemon)
    directory = _machine(tmp_path, [session], [daemon])
    offer = {"version": 3, "kinds": ["status", "x"], "mode": "presenter"}
    hello = {"kind": "hello", "gen": 1, "rev": 1, "hello": offer}
    frames = [hello, {"kind": "presenter.acquire", "gen": 1, "rev": 2}, {"kind": "ping", "gen": 1, "rev": 3}]
    stdin = "".join(json.dumps(frame) + "\n" for frame in frames)
    result = _run(directory, "agtermctl", "zmx", "present", session["id"], input=stdin)
    sent = [json.loads(line) for line in result.stdout.splitlines()]
    assert [frame["kind"] for frame in sent] == ["hello", "snapshot", "presenter.granted", "ack"]
    assert sent[0]["hello"] == {"version": 1, "kinds": ["status"], "mode": "presenter"}
    identity = str(uuid.UUID(daemon.removeprefix("agterm-"))).upper()
    assert sent[1]["snapshot"]["layout"] == {"panes": [identity], "primary": identity, "shown": False}
    assert [frame["rev"] for frame in sent] == [1, 2, 3, 4]


def test_a_refused_stream_exits_with_its_reason_on_stderr(tmp_path):
    directory = _machine(tmp_path, [], [])
    result = _run(directory, "agtermctl", "zmx", "present", "NOPE")
    assert (result.returncode, result.stdout, result.stderr) == (1, "", "Error: no such session\n")
