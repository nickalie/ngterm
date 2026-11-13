"""Another machine on loopback: an unprivileged `sshd` of the test's own and the viewer side's ssh client set up to
reach it, so the instance under test runs its real ssh against a real server (POSIX hosts only).

The server (`LoopbackMachine`) listens on 127.0.0.1 alone on a free port, takes one generated key and nothing else, and
reads only the files written here (its config, host key, `authorized_keys`), never the account's `~/.ssh` or the
system's sshd configuration. Every login runs the machine's forced command, which hands the client's command to the
account's login shell as sshd itself would, but with the machine's own HOME and the bare PATH sshd gives a command:
what that HOME's `.local/bin` holds is what the far side has installed. sshd gives the machine's HOME to the login shell
that starts the forced command as well, so that shell reads the machine's startup files, not the account's. It is
ported from the mobile e2e test's support code (`crates/ngterm-mobile/tests/support/sshd.rs`).

The client side (`SshClient`) is a directory holding an `ssh` first on the instance's PATH: OpenSSH reads the account's
`~/.ssh` from the password database, not from HOME, so the isolated HOME alone cannot hold its config. The stand-in runs
the system's ssh with `-F` and a config naming each machine by its alias, its known host key and its identity, with no
agent, no connection sharing and nothing read from the account's own files.
"""

from __future__ import annotations

import os
import shlex
import shutil
import socket
import subprocess
import time
from collections.abc import Mapping
from pathlib import Path

import pytest

SSHD_DIRECTORIES = ("/usr/sbin", "/usr/local/sbin", "/usr/bin", "/sbin")
READY_TIMEOUT = 15.0


def find_program(name: str, extra: tuple[str, ...] = ()) -> str | None:
    """`name` on PATH, then in `extra`: an absolute path, as sshd re-executes itself."""
    found = shutil.which(name)
    if found:
        return os.path.abspath(found)
    for directory in extra:
        candidate = os.path.join(directory, name)
        if os.access(candidate, os.X_OK):
            return candidate
    return None


def find_openssh() -> tuple[dict[str, str | None], str | None]:
    """sshd, ssh and ssh-keygen by name (None where missing), and why the loopback machine cannot run, or None."""
    found = {
        "sshd": find_program("sshd", SSHD_DIRECTORIES),
        "ssh": find_program("ssh"),
        "ssh-keygen": find_program("ssh-keygen"),
    }
    missing = [name for name, path in found.items() if path is None]
    if not missing:
        return found, None
    return found, f"no {', '.join(missing)} on this machine: the loopback machine needs OpenSSH's server and client"


def require_openssh() -> tuple[str, str, str]:
    """sshd, ssh and ssh-keygen, or the test is skipped with the missing one named; under CI it fails instead, as a
    runner that should carry them must not pass by skipping."""
    found, reason = find_openssh()
    if reason is not None:
        if os.environ.get("CI"):
            pytest.fail(reason)
        pytest.skip(reason)
    return found["sshd"], found["ssh"], found["ssh-keygen"]  # type: ignore[return-value]


def _keygen(keygen: str, path: Path, comment: str) -> str:
    subprocess.run(
        [keygen, "-q", "-t", "ed25519", "-N", "", "-C", comment, "-f", os.fspath(path)],
        check=True,
        stdin=subprocess.DEVNULL,
        capture_output=True,
    )
    return path.with_suffix(".pub").read_text().strip()


def _free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def _banner(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=2) as stream:
            stream.settimeout(2)
            return stream.recv(4) == b"SSH-"
    except OSError:
        return False


def _account():
    """The account's password-database entry (`pwd` exists on POSIX hosts only)."""
    import pwd

    return pwd.getpwuid(os.getuid())


def _config_word(path: Path | str) -> str:
    text = os.fspath(path)
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"'


class LoopbackMachine:
    """One machine: its sshd, its HOME and what its forced command exports. Stop it with `stop`."""

    def __init__(
        self, root: Path, alias: str, client_key: str, env: Mapping[str, str] | None = None, home: Path | None = None
    ) -> None:
        """`home` is the HOME logins get, by default one of the machine's own: an app already running there passes
        its own, so a login sees what that app's user has."""
        self.alias = alias
        self.directory = root / f"machine-{alias}"
        self.directory.mkdir(parents=True, exist_ok=True)
        self.home = home if home is not None else self.directory / "home"
        self.bin = self.home / ".local" / "bin"
        self.bin.mkdir(parents=True, exist_ok=True)
        self.env = dict(env or {})
        self.port = 0
        self.host_public = ""
        self._client_key = client_key
        self._process: subprocess.Popen[bytes] | None = None

    @property
    def log(self) -> Path:
        return self.directory / "sshd.log"

    def log_text(self) -> str:
        try:
            return self.log.read_text(errors="replace")
        except OSError:
            return ""

    def start(self, sshd: str, keygen: str) -> None:
        host_key = self.directory / "host_ed25519"
        self.host_public = _keygen(keygen, host_key, f"loopback-{self.alias}")
        authorized = self.directory / "authorized_keys"
        authorized.write_text(self._client_key + "\n")
        authorized.chmod(0o600)
        force = self._write_force_command()
        self._warm_login_shell()
        last = ""
        # a free port goes back to the kernel before sshd binds it, so another process may win it
        for _ in range(3):
            self.port = _free_port()
            config = self.directory / "sshd_config"
            config.write_text(self._config_text(host_key, authorized, force))
            self._process = subprocess.Popen(
                [sshd, "-D", "-f", os.fspath(config), "-E", os.fspath(self.log)],
                env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            deadline = time.monotonic() + READY_TIMEOUT
            while time.monotonic() < deadline:
                if self._process.poll() is not None:
                    break
                if _banner(self.port):
                    return
                time.sleep(0.05)
            last = f"sshd for {self.alias} did not answer on {self.port}:\n{self.log_text()}"
            self.stop()
        raise RuntimeError(last)

    def stop(self) -> None:
        process, self._process = self._process, None
        if process is None:
            return
        # our own child by its handle; each connection's process ends with its client
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)

    def _warm_login_shell(self) -> None:
        """Starts the account's shell once in this HOME, as a machine someone logs into has: a shell's first start in
        an empty HOME (fish writes its config and variables) can outlast a viewer's deadline on a loaded host."""
        shell = _account().pw_shell or "/bin/sh"
        subprocess.run(
            [shell, "-c", "true"],
            env={"HOME": os.fspath(self.home), "PATH": "/usr/bin:/bin", "USER": _account().pw_name},
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=120,
            check=False,
        )

    def _write_force_command(self) -> Path:
        """The command every login runs: the client's own through the account's login shell, as sshd runs it, with
        this machine's HOME, sshd's bare PATH and whatever the machine exports."""
        shell = _account().pw_shell or "/bin/sh"
        exports = {"HOME": os.fspath(self.home), "PATH": "/usr/bin:/bin", **self.env}
        assignments = " ".join(f"{name}={shlex.quote(value)}" for name, value in exports.items())
        script = self.directory / "force-command"
        script.write_text(
            "#!/bin/sh\n"
            # the variables sshd itself sets for a session, and nothing of the account's environment
            f'exec /usr/bin/env -i USER="$USER" LOGNAME="$LOGNAME" SHELL={shlex.quote(shell)} '
            'TERM="${TERM:-dumb}" SSH_CONNECTION="$SSH_CONNECTION" SSH_CLIENT="$SSH_CLIENT" '
            'SSH_TTY="${SSH_TTY:-}" '
            f'{assignments} {shlex.quote(shell)} -c "$SSH_ORIGINAL_COMMAND"\n'
        )
        script.chmod(0o755)
        return script

    def _config_text(self, host_key: Path, authorized: Path, force: Path) -> str:
        user = _account().pw_name
        return (
            "ListenAddress 127.0.0.1\n"
            f"Port {self.port}\n"
            f"HostKey {_config_word(host_key)}\n"
            f"PidFile {_config_word(self.directory / 'sshd.pid')}\n"
            # sshd expands %-tokens in it
            f"AuthorizedKeysFile {_config_word(authorized).replace('%', '%%')}\n"
            "AuthenticationMethods publickey\n"
            "PubkeyAuthentication yes\n"
            "PasswordAuthentication no\n"
            "KbdInteractiveAuthentication no\n"
            f"AllowUsers {user}\n"
            "StrictModes no\n"
            "PermitUserRC no\n"
            "PermitUserEnvironment no\n"
            "PermitTTY yes\n"
            "AllowTcpForwarding yes\n"
            "AllowAgentForwarding no\n"
            "X11Forwarding no\n"
            "PrintMotd no\n"
            "UsePAM no\n"
            # the login shell sshd starts the forced command with reads its startup files from HOME (fish's
            # config.fish, zsh's .zshenv, bash's .bashrc over ssh): this machine's, never the account's own
            f"SetEnv {_config_word(f'HOME={os.fspath(self.home)}')}\n"
            f"ForceCommand {force}\n"
            "LogLevel VERBOSE\n"
        )


class SshClient:
    """The viewer side: a key, a config naming each machine and an `ssh` stand-in that reads only that config."""

    def __init__(self, root: Path, ssh: str, keygen: str) -> None:
        self.directory = root / "ssh-client"
        self.bin = self.directory / "bin"
        self.bin.mkdir(parents=True)
        self.identity = self.directory / "id_ed25519"
        self.public_key = _keygen(keygen, self.identity, "loopback-viewer")
        self.config = self.directory / "config"
        self.known_hosts = self.directory / "known_hosts"
        self.calls = self.directory / "calls"
        self._machines: list[tuple[LoopbackMachine, int | None]] = []
        wrapper = self.bin / "ssh"
        wrapper.write_text(
            "#!/bin/sh\n"
            # one line per invocation, its words tab-joined, so a test can tell what the app ran
            f"(IFS='\t'; printf '%s\\n' \"$*\") >> {shlex.quote(os.fspath(self.calls))}\n"
            f"exec {shlex.quote(ssh)} -F {shlex.quote(os.fspath(self.config))} \"$@\"\n"
        )
        wrapper.chmod(0o755)
        self._write()

    def add(self, machine: LoopbackMachine, port: int | None = None) -> None:
        """`port` reaches the machine through something in front of its sshd (a proxy), by default its sshd itself."""
        self._machines.append((machine, port))
        self._write()

    def warm(self, machine: LoopbackMachine) -> None:
        """One login to `machine` before a test asks anything of it, so the first answer a test reads does not pay for
        the host's cold start of ssh, sshd and the account's shell."""
        subprocess.run(
            [os.fspath(self.bin / "ssh"), "-T", "-o", "BatchMode=yes", machine.alias, "true"],
            env={"PATH": "/usr/bin:/bin", "HOME": os.fspath(self.directory)},
            stdin=subprocess.DEVNULL,
            capture_output=True,
            timeout=120,
            check=True,
        )
        self.calls.unlink(missing_ok=True)

    def invocations(self) -> list[list[str]]:
        try:
            lines = self.calls.read_text().splitlines()
        except FileNotFoundError:
            return []
        return [line.split("\t") for line in lines]

    def _write(self) -> None:
        user = _account().pw_name
        blocks = [
            f"Host {machine.alias}\n"
            "  HostName 127.0.0.1\n"
            f"  Port {port or machine.port}\n"
            f"  User {user}\n"
            f"  HostKeyAlias {machine.alias}\n"
            f"  IdentityFile {_config_word(self.identity)}\n"
            "  IdentitiesOnly yes\n"
            "  IdentityAgent none\n"
            for machine, port in self._machines
        ]
        common = (
            "Host *\n"
            f"  UserKnownHostsFile {_config_word(self.known_hosts)}\n"
            "  GlobalKnownHostsFile /dev/null\n"
            "  StrictHostKeyChecking yes\n"
            "  UpdateHostKeys no\n"
            "  ControlMaster no\n"
            "  ControlPath none\n"
            "  ForwardAgent no\n"
        )
        self.config.write_text("".join(blocks) + common)
        self.known_hosts.write_text(
            "".join(f"{machine.alias} {machine.host_public}\n" for machine, _ in self._machines)
        )
