"""Another machine a viewer attaches remote sessions from, on loopback (POSIX hosts only): a second instance of the build
(`instance`, Live on) or the staged `ngterm-headless` (`headless`), each on its own private root, behind an unprivileged
`sshd` of the harness's own (`loopback_ssh.LoopbackMachine`) and a TCP proxy whose connections a test cuts, as a lost
network would (`SeveringProxy`).

A login to the origin gets the origin's own HOME, whose `~/.local/bin` holds the build's CLI as its installer links it,
and the origin's state dir, so the CLI there answers for that instance, as on a machine whose user runs it. A coverage
run's profile destination reaches the login too: the far side's CLI and session server count.

The viewer reaches it as `ORIGIN_HOST` through `RemoteOrigin.client`: a private ssh directory (config, known_hosts and
identity, all generated here) and an `ssh` that reads nothing else, which a viewer takes first on its PATH
(`RemoteOrigin.viewer_path`). Nothing reads or writes the account's `~/.ssh`, its agent or the system's sshd.

`RemoteOrigin.stop` ends the proxy and the sshd first, so no login arrives during the reap, then stops the origin
through the adapter, which reaps its live sessions through the origin's own server and requires none left behind.

The sshd setup and the proxy are ported from the mobile e2e test's support code
(`crates/ngterm-mobile/tests/support/sshd.rs`).
"""

from __future__ import annotations

import os
import shutil
import socket
import sys
import threading
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from agterm_conformance import llvm_profiles
from agterm_conformance.loopback_ssh import LoopbackMachine, SshClient, find_openssh

if TYPE_CHECKING:
    from agterm_conformance.impls.base import Adapter, Instance

ORIGIN_HOST = "origin"
KINDS = ("instance", "headless")
HEADLESS_PROGRAM = "ngterm-headless"
# The origin desktop app restores its sessions as live ones: only those can be attached.
LIVE_SETTINGS = {"restoreMode": "live"}
ACCEPT_POLL = 0.2
CONNECT_TIMEOUT = 5.0
BUFFER = 16 * 1024


def require(step: str, reason: str | None) -> None:
    """Nothing when `reason` is None. Otherwise the step cannot run: under CI the run ends (exit 2, naming the step), as a
    runner that should carry what it needs must not pass by skipping; elsewhere the test is skipped with the reason."""
    if reason is None:
        return
    message = f"remote origin: {step}: {reason}"
    if os.environ.get("CI"):
        pytest.exit(message, returncode=2)
    pytest.skip(message)


def _connect_target(port: int) -> socket.socket:
    stream = socket.create_connection(("127.0.0.1", port), timeout=CONNECT_TIMEOUT)
    stream.settimeout(None)
    return stream


def _shut(stream: socket.socket) -> None:
    try:
        stream.shutdown(socket.SHUT_RDWR)
    except OSError:
        pass


class SeveringProxy:
    """A TCP proxy on a free port of 127.0.0.1 in front of `target`'s: `sever` cuts every connection open through it,
    while the target and later connections carry on. `stop` closes it."""

    def __init__(self, target: int) -> None:
        self.target = target
        self._listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._listener.bind(("127.0.0.1", 0))
        self._listener.listen()
        # polled, so `stop` ends the accept loop without relying on a close waking a blocked accept
        self._listener.settimeout(ACCEPT_POLL)
        self.port: int = self._listener.getsockname()[1]
        self._open: set[socket.socket] = set()
        self._lock = threading.Lock()
        self._stopping = threading.Event()
        self._thread = threading.Thread(target=self._accept, name=f"proxy-{self.port}", daemon=True)
        self._thread.start()

    def connections(self) -> int:
        """How many connections are open through it."""
        with self._lock:
            return len(self._open) // 2

    def sever(self) -> None:
        """Cuts every connection open now: both ends see the stream end, as on a lost network."""
        with self._lock:
            streams = list(self._open)
        for stream in streams:
            _shut(stream)

    def stop(self) -> None:
        self._stopping.set()
        self._thread.join(timeout=10)
        self._listener.close()
        self.sever()

    def _accept(self) -> None:
        while not self._stopping.is_set():
            try:
                client, _ = self._listener.accept()
            except TimeoutError:
                continue
            except OSError:
                return
            client.settimeout(None)
            try:
                server = _connect_target(self.target)
            except OSError:
                client.close()
                continue
            with self._lock:
                self._open.update((client, server))
            for source, sink in ((client, server), (server, client)):
                threading.Thread(target=self._pump, args=(source, sink), daemon=True).start()

    def _pump(self, source: socket.socket, sink: socket.socket) -> None:
        """Copies `source` into `sink` until either side ends, then ends both."""
        try:
            while data := source.recv(BUFFER):
                sink.sendall(data)
        except OSError:
            pass
        _shut(source)
        _shut(sink)
        with self._lock:
            self._open.discard(source)
            self._open.discard(sink)
        source.close()


def headless_program(adapter: Adapter) -> Path | None:
    """The staged `ngterm-headless` beside the adapter's binary, or None."""
    if adapter.bin is None:
        return None
    program = Path(adapter.bin).parent / (HEADLESS_PROGRAM + (".exe" if sys.platform == "win32" else ""))
    return program if os.access(program, os.X_OK) else None


def launch_headless(adapter: Adapter, program: Path) -> Instance:
    """`program` (the staged `ngterm-headless`) on a private root laid out and isolated as `Adapter.launch` lays out an
    instance's, without a display; the adapter stops it as it stops any instance."""
    from agterm_conformance.impls import base

    root = base.make_root()
    try:
        env = adapter.build_env(root)
        state, home = Path(env[adapter.variable("STATE_DIR")]), Path(env["HOME"])
        base._prepare_tree(root, home, state, adapter.ghostty_config())
        if adapter.needs_runtime_dir:
            os.mkdir(env["XDG_RUNTIME_DIR"], 0o700)  # the XDG spec's mode
        log = root / "app.log"
        # the private home's shells stay as the harness set them up: no agent skill, no hooks
        process = base._spawn([os.fspath(program), "--skip-agent-setup"], env, log, "wb")
    except BaseException:
        shutil.rmtree(root, ignore_errors=True)
        raise
    instance = base.Instance(
        process=process,
        root=root,
        state_dir=state,
        home=home,
        socket=Path(env[adapter.variable("CONTROL_SOCKET")]),
        env=env,
        log=log,
        app_id=None,
        live_dir=adapter.vocabulary.socket_directory(os.fspath(state), adapter.app_dir_name, env),
        vocabulary=adapter.vocabulary,
        env_prefix=adapter.env_prefix,
    )
    try:
        instance.version = base.wait_for_socket(instance, base.LAUNCH_TIMEOUT)
    except BaseException:
        adapter.stop(instance, cleanup=False)
        raise
    return instance


class RemoteOrigin:
    """The origin (`kind`: one of `KINDS`), its sshd, the proxy in front of it and the viewer's ssh client, under
    `root`. `start` brings them up; `stop` ends whatever started, also after a failed `start`."""

    def __init__(self, adapter: Adapter, root: Path, kind: str = "instance") -> None:
        if kind not in KINDS:
            raise ValueError(f"remote origin kind {kind!r}: one of {', '.join(KINDS)}")
        self.adapter = adapter
        self.root = root
        self.kind = kind
        self.host = ORIGIN_HOST
        self.instance: Instance | None = None
        self.client: SshClient | None = None
        self.machine: LoopbackMachine | None = None
        self.proxy: SeveringProxy | None = None

    @property
    def viewer_path(self) -> list[Path]:
        """What a viewer puts first on its PATH (`Adapter.launch(path_prefix=...)`): the client's `ssh` alone."""
        assert self.client is not None, "the remote origin has not started"
        return [self.client.bin]

    def start(self) -> RemoteOrigin:
        if sys.platform == "win32":
            pytest.skip("the remote origin is POSIX only: a Windows host runs no unprivileged sshd")
        found, reason = find_openssh()
        require("start the origin's sshd", reason)
        ctl = self.adapter.ctl_path
        require(
            f"install {self.adapter.ctl_name} on the origin",
            None if ctl is not None else f"{self.adapter.name} has no {self.adapter.ctl_name}: pass --ctl",
        )
        program = None
        if self.kind == "headless":
            program = headless_program(self.adapter)
            require(
                f"launch the staged {HEADLESS_PROGRAM}",
                None if program is not None else f"no executable {HEADLESS_PROGRAM} beside {self.adapter.bin}",
            )
        sshd, ssh, keygen = found["sshd"], found["ssh"], found["ssh-keygen"]
        assert sshd and ssh and keygen and ctl
        self.client = SshClient(self.root, ssh, keygen)
        if program is not None:
            self.instance = launch_headless(self.adapter, program)
        else:
            self.instance = self.adapter.launch(settings=LIVE_SETTINGS)
        self.machine = LoopbackMachine(
            self.root, ORIGIN_HOST, self.client.public_key, env=self.login_env(), home=self.instance.home
        )
        self._install_ctl(ctl)
        self.machine.start(sshd, keygen)
        self.proxy = SeveringProxy(self.machine.port)
        self.client.add(self.machine, port=self.proxy.port)
        self.client.warm(self.machine)
        return self

    def login_env(self) -> dict[str, str]:
        """What a login to the origin exports besides HOME and PATH: the origin's state dir and, in a coverage run, the
        pooled profile destination its binaries write to."""
        assert self.instance is not None
        env = {self.adapter.variable("STATE_DIR"): os.fspath(self.instance.state_dir)}
        if (profile := self.instance.env.get(llvm_profiles.PROFILE_VARIABLE)) is not None:
            env[llvm_profiles.PROFILE_VARIABLE] = profile
        return env

    def sever(self) -> None:
        """Cuts every ssh connection open to the origin now; sshd, the origin and later connections carry on."""
        assert self.proxy is not None, "the remote origin has not started"
        self.proxy.sever()

    def stop(self) -> None:
        """The proxy, then the sshd, then the origin (its live sessions reaped through its own server); every step runs
        whatever an earlier one raised."""
        proxy, self.proxy = self.proxy, None
        machine, self.machine = self.machine, None
        instance, self.instance = self.instance, None
        try:
            if proxy is not None:
                proxy.stop()
        finally:
            try:
                if machine is not None:
                    machine.stop()
            finally:
                if instance is not None:
                    self.adapter.stop(instance)

    def _install_ctl(self, ctl: Path) -> None:
        assert self.machine is not None
        link = self.machine.bin / self.adapter.ctl_name
        link.unlink(missing_ok=True)
        link.symlink_to(ctl)
