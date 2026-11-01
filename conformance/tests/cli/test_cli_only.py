"""The `agtermctl` subcommands with no socket command of their own: `clipboard set` and `terminfo install`, which
never open the socket, and `session wait`, which composes existing reads (`events.read`, `window.list`, `tree`); and
the usage errors `session overlay open --companion` gives before any connection.

The first two run through `cli.local`, without `--socket` (they refuse it) and in the instance's env, whose state
directory variable (`AGTERM_STATE_DIR`, the Rust build's `NGTERM_STATE_DIR`) is under the temp root. `clipboard set`
writes nothing outside a Live pane, and inside one the instance's terminals drop the OSC 52 write
(`clipboard-write = deny`), so the desktop clipboard is never touched.
`terminfo install` talks to a stand-in `ssh` on PATH that runs the remote command against the isolated HOME (on
Windows an `ssh.exe` launcher over Python, running it in Git for Windows' `sh` with its `tic`).
`session wait` runs against the instance's socket, or through a relay in the instance root that notes each request, so
a test acts only once the wait has read the tree and is polling."""

import base64
import dataclasses
import json
import os
import shlex
import shutil
import stat
import sys
import threading
import time
import uuid
from contextlib import contextmanager
from pathlib import Path

import pytest
from agterm_conformance import transport
from agterm_conformance.impls.base import posix_shell
from agterm_conformance.match import assert_ok, find_session
from agterm_conformance.wait import wait_until

# `TerminalClipboard.maxEncodedBytes`: under the 8 MiB above which libghostty drops an OSC whole.
MAX_ENCODED = 8 * 1024 * 1024 - 64
ENTRY = "xterm-ghostty"
NOT_LIVE = "this pane has no zmx daemon; run it in a main or split pane started under Live sessions"


def _failed(result, message, code=1):
    """A local failure: `Error: <message>` on stderr and no usage block under it."""
    assert (result.returncode, result.stdout, result.stderr) == (code, "", f"Error: {message}\n"), result.argv


def _usage(result, message=None):
    assert (result.returncode, result.stdout) == (64, ""), (result.argv, result.returncode, result.stderr)
    assert result.stderr.startswith(f"Error: {message}\n" if message else "Error: "), result.stderr
    assert f"\nUsage: {result.program} " in result.stderr, result.stderr


def _env(cli, session=None, directory=None, **extra):
    """The instance's env outside any live pane, or naming `session` under `directory` as a live pane's children do."""
    vocabulary = cli.vocabulary
    env = {name: value for name, value in cli.instance.env.items() if not name.startswith(vocabulary.variable_prefix)}
    if session is not None:
        env.update({vocabulary.session_variable: session, vocabulary.directory_variable: directory})
    env.update(extra)
    return env


# -- clipboard set


@pytest.mark.covers("cli:clipboard set")
def test_clipboard_set_refuses_outside_a_live_pane(cli):
    not_live = cli.vocabulary.text(NOT_LIVE)
    _failed(cli.local("clipboard", "set", "hello", input=""), not_live)
    _failed(cli.local("clipboard", "set", input="hello"), not_live)
    # A session variable naming a session the app would never have named is not printed into.
    env = _env(cli, "someone-elses-session", cli.instance.live_dir)
    _failed(cli.local("clipboard", "set", "hello", env=env, input=""), not_live)


@pytest.mark.covers("cli:clipboard set")
def test_clipboard_set_checks_the_text_before_the_pane(cli):
    _failed(cli.local("clipboard", "set", input=""), "nothing to copy")
    _failed(cli.local("clipboard", "set", "", input=""), "nothing to copy")
    size = 6_300_000
    encoded = len(base64.b64encode(b"x" * size))
    assert encoded > MAX_ENCODED
    message = f"text is too large to copy through the terminal: {encoded} bytes encoded, the limit is {MAX_ENCODED}"
    _failed(cli.local("clipboard", "set", input="x" * size), message)
    # Local-only: there is no socket to name.
    _usage(cli.local("clipboard", "set", "--socket", os.fspath(cli.instance.socket), "hello", input=""))


@pytest.mark.requires(
    "live-sessions", reason="needs panes in zmx daemons, which upstream creates only for a zsh or fish login shell"
)
@pytest.mark.covers("cli:clipboard set")
def test_clipboard_set_reaches_every_terminal_attached_to_the_pane(cli, launch):
    ctl = launch(settings={"restoreMode": "live"})
    local = cli.on(ctl.instance)
    sid = assert_ok(ctl.request("session.new"))["id"]
    vocabulary = ctl.vocabulary
    ctl.wait_tree(lambda tree: find_session(tree, sid).get(vocabulary.backed_key) is True)

    def left_row(response):
        rows = [row for row in assert_ok(response)[vocabulary.result_key]["entries"] if row.get("sessionID") == sid]
        # Running first: a viewer attaching to a daemon not up yet would start one under that name.
        return [row for row in rows if row["pane"] == "left" and row.get("observation") == "running"]

    (row,) = left_row(ctl.wait_response(vocabulary.command("zmx.list"), None, left_row))
    daemon = row["daemon"]
    endpoint = assert_ok(ctl.request(vocabulary.command("zmx.tree")))["remote"]["endpoint"]
    text = f"clip-{uuid.uuid4().hex[:8]}"
    sequence = f"\x1b]52;c;{base64.b64encode(text.encode()).decode()}\x07"
    with vocabulary.leading_client(ctl.instance, endpoint, daemon) as viewer:
        # Attached once the pane follows: a print into a session still taking the viewer finds it busy (zmx).
        ctl.wait_tree(
            lambda tree: (
                next(s for s in find_session(tree, sid)["surfaces"] if s["kind"] == "left").get("lead") == "follower"
            )
        )
        env = _env(local, daemon, ctl.instance.live_dir)
        result = local.local("clipboard", "set", text, env=env, input="")
        assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
        wait_until(lambda: sequence in viewer.output())
    assert_ok(ctl.request("session.close", sid))


# -- terminfo install


def _account_terminfo():
    """The real account's own terminfo directories, which the install must leave alone."""
    if sys.platform == "win32":
        home = Path(os.environ["USERPROFILE"])
    else:
        import pwd

        home = Path(pwd.getpwuid(os.getuid()).pw_dir)
    return [home / ".terminfo", home / ".local/share/terminfo"]


def _snapshot(directories):
    state = {}
    for directory in directories:
        if not directory.exists():
            state[directory] = None
            continue
        state[directory] = sorted(
            (os.fspath(path.relative_to(directory)), path.stat().st_mtime_ns, path.stat().st_size)
            for path in directory.rglob("*")
        )
    return state


# The Windows stand-in's body: what the POSIX script below does, with the remote account's shell Git for Windows'.
_WINDOWS_FAKE_SSH = """#!python
import os
import subprocess
import sys

with open({record!r}, "w", encoding="utf-8", newline="\\n") as out:
    out.writelines(arg + "\\n" for arg in sys.argv[1:])
mode = os.environ.get("FAKE_SSH", "")
if mode.startswith("exit"):
    sys.exit(int(mode[len("exit"):]))
# HOME and PATH alone, as `env -i` gives the POSIX shell (and SYSTEMROOT, which any Windows process needs)
remote = {{"HOME": {home!r}, "PATH": {path!r}, "SYSTEMROOT": os.environ.get("SYSTEMROOT", r"C:\\Windows")}}
sys.exit(subprocess.run([{shell!r}, "-c", sys.argv[-1]], env=remote).returncode)
"""


def _windows_fake_ssh(folder, record, home):
    """`ssh.exe` in `folder`: Windows finds a program by `.exe` alone, so the stand-in is distlib's script launcher (as
    pip makes console scripts) over a Python script."""
    from distlib.scripts import ScriptMaker

    shell = posix_shell()
    git = os.path.dirname(os.path.dirname(shell))
    path = os.pathsep.join([os.path.join(git, "usr", "bin"), os.path.join(git, "bin")])
    source = folder / "source"
    source.mkdir()
    body = _WINDOWS_FAKE_SSH.format(record=os.fspath(record), home=os.fspath(home), path=path, shell=shell)
    (source / "ssh.py").write_text(body, encoding="utf-8")
    maker = ScriptMaker(os.fspath(source), os.fspath(folder))
    maker.executable = sys.executable
    maker.make("ssh.py")
    assert (folder / "ssh.exe").is_file(), sorted(os.listdir(folder))


@pytest.fixture
def fake_ssh(cli):
    """A stand-in `ssh` first on PATH: it records its argv and runs the remote command line the way sshd hands it to
    the account's shell, with HOME the instance's isolated HOME and nothing else inherited (TERMINFO included)."""
    root = cli.instance.root
    folder = root / f"fake-ssh-{uuid.uuid4().hex[:8]}"
    folder.mkdir()
    record = folder / "argv"
    if sys.platform == "win32":
        _windows_fake_ssh(folder, record, cli.instance.home)
    else:
        script = folder / "ssh"
        script.write_text(
            "#!/bin/sh\n"
            f"for arg; do printf '%s\\n' \"$arg\"; done > {shlex.quote(os.fspath(record))}\n"
            'case "$FAKE_SSH" in exit*) exit "${FAKE_SSH#exit}" ;; signal) kill -TERM $$ ;; esac\n'
            "for last; do :; done\n"
            f"exec /usr/bin/env -i HOME={shlex.quote(os.fspath(cli.instance.home))} PATH=/usr/bin:/bin "
            '/bin/sh -c "$last"\n'
        )
        script.chmod(script.stat().st_mode | stat.S_IXUSR)

    def env(**extra):
        return _env(cli, PATH=f"{folder}{os.pathsep}{cli.instance.env['PATH']}", **extra)

    env.record = record
    yield env
    shutil.rmtree(cli.instance.home / ".terminfo", ignore_errors=True)


@pytest.mark.covers("cli:terminfo install")
def test_terminfo_install_compiles_the_entry_into_the_remote_home(cli, fake_ssh):
    account = _account_terminfo()
    before = _snapshot(account)
    args = ("terminfo", "install", "-p", "2222", "-i", "id_one", "-i", "id_two", "-J", "jump", "-F", "cfg", "box")
    result = cli.local(*args, env=fake_ssh(), input="")
    # The remote tic's own warnings come through on stderr, as ssh passes them on.
    assert (result.returncode, result.stdout) == (0, f"installed {ENTRY} on box\n"), result.stderr
    installed = [cli.instance.home / ".terminfo" / letter / ENTRY for letter in ("x", "78")]
    assert any(path.is_file() and path.stat().st_size > 0 for path in installed), installed
    assert _snapshot(account) == before

    argv = fake_ssh.record.read_text().splitlines()
    # The connection options pass through; the remote command line comes last, after the destination.
    assert argv[-2] == "box" and shlex.split(argv[-1])[:2] == ["/bin/sh", "-c"]
    options = argv[:-2]
    for pair in (["-p", "2222"], ["-i", "id_one"], ["-i", "id_two"], ["-J", "jump"], ["-F", "cfg"]):
        assert any(options[i : i + 2] == pair for i in range(len(options) - 1)), (pair, options)
    assert "-T" in options


@pytest.mark.covers("cli:terminfo install")
def test_terminfo_install_reports_how_ssh_ended(cli, fake_ssh):
    failed = cli.local("terminfo", "install", "box", env=fake_ssh(FAKE_SSH="exit255"), input="")
    assert (failed.returncode, failed.stdout, failed.stderr) == (
        255,
        "",
        f"ssh exited 255; {ENTRY} was not installed\n",
    )
    # Windows has no signals: ssh ends there with an exit status, which the half above covers
    if sys.platform != "win32":
        killed = cli.local("terminfo", "install", "box", env=fake_ssh(FAKE_SSH="signal"), input="")
        message = f"ssh was killed by signal 15; {ENTRY} was not installed\n"
        assert (killed.returncode, killed.stdout, killed.stderr) == (128 + 15, "", message)
    assert not (cli.instance.home / ".terminfo").exists()


@pytest.mark.covers("cli:terminfo install")
@pytest.mark.parametrize("destination", ["-oProxyCommand=x", "two words", ""])
def test_terminfo_install_refuses_a_destination_ssh_would_misread(cli, fake_ssh, destination):
    result = cli.local("terminfo", "install", "--", destination, env=fake_ssh(), input="")
    _usage(result, f"destination must be a host, user@host or an ssh alias, not '{destination}'")
    assert not fake_ssh.record.exists()


# -- session wait

WAIT = ("session", "wait")
TIMEOUT_REFUSAL = "--timeout must be a positive number of seconds"


def _read_line(connection):
    data = b""
    while not data.endswith(b"\n"):
        chunk = connection.recv(65536)
        if not chunk:
            break
        data += chunk
    return data


class _Relay:
    """A socket in the instance root that passes each connection's request line to the instance and the reply back,
    noting every request as it arrives."""

    def __init__(self, cli):
        self._client = cli.instance.client()
        path = cli.instance.root / f"r{uuid.uuid4().hex[:6]}.sock"
        self.cli = cli.on(dataclasses.replace(cli.instance, socket=path))
        self.requests = []
        self._lock = threading.Lock()
        # an AF_UNIX socket, or on Windows the named pipe its path names
        self._server = transport.Listener(path)
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            try:
                connection = self._server.accept()
            except OSError:
                return  # closed
            threading.Thread(target=self._relay, args=(connection,), daemon=True).start()

    def _relay(self, connection):
        with connection:
            line = _read_line(connection)
            if not line:
                return
            with self._lock:
                self.requests.append(json.loads(line))
            # Through the instance's own client, which checks the root; the reply goes back re-encoded.
            reply = self._client.send_line(line.rstrip(b"\n"))
            connection.sendall(json.dumps(reply.raw, separators=(",", ":")).encode() + b"\n")

    def polling(self):
        """Whether the wait has read the tree and gone on to poll events from its cursor."""
        with self._lock:
            commands = [(request["cmd"], (request.get("args") or {}).get("after")) for request in self.requests]
        trees = [index for index, (cmd, _) in enumerate(commands) if cmd == "tree"]
        return bool(trees) and any(cmd == "events.read" and after for cmd, after in commands[trees[-1] + 1 :])

    def close(self):
        self._server.close()


@contextmanager
def _relay(cli):
    relay = _Relay(cli)
    try:
        yield relay
    finally:
        relay.close()


def _new_session(ctl):
    return assert_ok(ctl.request("session.new"))["id"]


@pytest.mark.covers("cli:session wait")
def test_session_wait_returns_at_once_when_the_status_already_matches(cli, ctl):
    sid = _new_session(ctl)
    assert_ok(ctl.request("session.status", sid, status="completed"))
    result = cli(*WAIT, "--until", "completed", "--target", sid)
    assert (result.returncode, result.stdout, result.stderr) == (0, "completed\n", ""), result.argv
    result = cli(*WAIT, "--until", "active", "--until", "completed", "--target", sid, "--json")
    expected = json.dumps({"id": sid, "status": "completed"}, separators=(",", ":"))
    assert (result.returncode, result.stdout, result.stderr) == (0, f"{expected}\n", ""), result.argv
    assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:session wait")
def test_session_wait_sees_a_later_status_event(cli, ctl):
    worker, other = _new_session(ctl), _new_session(ctl)
    assert_ok(ctl.request("session.status", worker, status="active"))
    with _relay(cli) as relay:
        args = (*WAIT, "--until", "completed", "--until", "blocked", "--target", worker)
        with relay.cli.spawn(*args) as waiting:
            wait_until(relay.polling)
            # Another session's match and the worker's status outside the set leave it waiting; the other session
            # takes a status the worker never reaches, so a wait that matched it would print `completed`.
            assert_ok(ctl.request("session.status", other, status="completed"))
            assert_ok(ctl.request("session.status", worker, status="active", blink=True))
            assert waiting.process.poll() is None
            assert_ok(ctl.request("session.status", worker, status="blocked"))
            result = waiting.finish()
    assert (result.returncode, result.stdout, result.stderr) == (0, "blocked\n", ""), result.argv
    for sid in (worker, other):
        assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:session wait")
def test_session_wait_exits_2_when_the_session_closes(cli, ctl):
    sid = _new_session(ctl)
    assert_ok(ctl.request("session.status", sid, status="active"))
    with _relay(cli) as relay:
        with relay.cli.spawn(*WAIT, "--until", "completed", "--target", sid) as waiting:
            wait_until(relay.polling)
            assert_ok(ctl.request("session.close", sid))
            result = waiting.finish()
    assert (result.returncode, result.stdout, result.stderr) == (2, "", "session closed\n"), result.argv


@pytest.mark.covers("cli:session wait")
def test_session_wait_times_out_with_124(cli, ctl):
    sid = _new_session(ctl)
    started = time.monotonic()
    result = cli(*WAIT, "--until", "completed", "--timeout", "0.5", "--target", sid)
    assert time.monotonic() - started >= 0.5
    assert (result.returncode, result.stdout, result.stderr) == (124, "", "timed out\n"), result.argv
    assert_ok(ctl.request("session.close", sid))


@pytest.mark.covers("cli:session wait")
@pytest.mark.parametrize(
    ("args", "code", "message"),
    [
        pytest.param([], 64, "provide at least one --until status", id="no-until"),
        pytest.param(
            ["--until", "done"], 64, "invalid status: done (expected idle, active, completed or blocked)", id="status"
        ),
        pytest.param(["--until", "completed", "--timeout", "0"], 64, TIMEOUT_REFUSAL, id="zero"),
        # The argument parsers differ on a value that looks like an option: only the usage error is shared.
        pytest.param(["--until", "completed", "--timeout", "-1"], 64, None, id="negative"),
        pytest.param(["--until", "completed", "--timeout", "inf"], 64, TIMEOUT_REFUSAL, id="infinite"),
        pytest.param(["--until", "completed", "--target", "zzzz"], 1, "no such session: zzzz", id="target"),
    ],
)
def test_session_wait_refuses_bad_arguments(cli, args, code, message):
    result = cli(*WAIT, *args)
    if code == 64:
        _usage(result, message)
    else:
        _failed(result, message, code)


# -- session overlay open --companion

COMPANION_FULL_SIZE = "--companion is full size: it takes no --pane, --size-percent, --width or --height"


@pytest.mark.since("0.39.1")
@pytest.mark.covers("cli:session overlay open")
@pytest.mark.parametrize(
    ("args", "message"),
    [
        pytest.param(["top"], "--companion requires --html or --url", id="program"),
        pytest.param(["--html", "/tmp/p.html", "--pane", "right"], COMPANION_FULL_SIZE, id="pane"),
        pytest.param(["--html", "/tmp/p.html", "--size-percent", "50"], COMPANION_FULL_SIZE, id="size"),
        pytest.param(["--html", "/tmp/p.html", "--width", "300"], COMPANION_FULL_SIZE, id="width"),
    ],
)
def test_overlay_open_companion_validation(cli, args, message):
    """The page companion's usage errors come before any connection: it is a page, and full size."""
    _usage(cli("session", "overlay", "open", *args, "--companion"), message)
