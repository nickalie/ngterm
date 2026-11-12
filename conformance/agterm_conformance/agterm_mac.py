"""A Mac running upstream agterm, as a viewer reaches it over ssh: a `loopback_ssh.LoopbackMachine` whose HOME's
`.local/bin` holds the stand-in `agtermctl` and whose app bundle holds its `zmx` (`fake_agterm`). The test sets what the
app shows (`add_session`), reads what the viewer ran there (`calls`) and drives the presentation stream (`push`,
`received`). A machine may also have ngterm installed beside it (`install_ngtermctl`), answering for another instance
of the build under test."""

from __future__ import annotations

import json
import os
import shlex
import sys
import uuid
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest
from agterm_conformance import fake_agterm
from agterm_conformance.loopback_ssh import LoopbackMachine, SshClient, require_openssh
from agterm_conformance.wait import wait_until

LISTING_HEADER = "ngterm-peer: agterm"


class AgtermMac:
    def __init__(self, machine: LoopbackMachine) -> None:
        self.machine = machine
        self.directory = machine.directory
        self.app = self.directory / "agterm.app"
        self.app.mkdir()
        self.zmx = self.app / "zmx"
        script = os.fspath(Path(fake_agterm.__file__).resolve())
        self._wrap(self.zmx, script, "zmx")
        self._wrap(machine.bin / "agtermctl", script, "agtermctl", fast_listing=True)
        socket_directory = self.directory / "zmx-sockets"
        socket_directory.mkdir()
        self._save(
            {
                "running": True,
                "restore": "live",
                "home": os.fspath(machine.home),
                "zmx": os.fspath(self.zmx),
                "socketDirectory": os.fspath(socket_directory),
                "window": {"id": str(uuid.uuid4()).upper(), "name": "Main"},
                "workspace": {"id": str(uuid.uuid4()).upper(), "name": "work"},
                "sessions": [],
                "daemons": [],
            }
        )

    def _wrap(self, path: Path, script: str, program: str, fast_listing: bool = False) -> None:
        # the listing is printed from the file the state keeps, so a loaded host answers it within the viewer's
        # deadline; every other command starts the stand-in
        directory = shlex.quote(os.fspath(self.directory))
        listing = (
            'if [ "$*" = "zmx tree --json" ]; then\n'
            f"  printf '%s\\n' '{{\"program\": \"agtermctl\", \"argv\": [\"zmx\", \"tree\", \"--json\"], "
            f"\"env\": {{}}, \"ngterm\": []}}' >> {directory}/calls.jsonl\n"
            f"  exec cat {directory}/listing.json\n"
            "fi\n"
            if fast_listing
            else ""
        )
        path.write_text(
            "#!/bin/sh\n"
            + listing
            + f"exec {shlex.quote(sys.executable)} -I {shlex.quote(script)} {shlex.quote(os.fspath(self.directory))} "
            f'{program} "$@"\n'
        )
        path.chmod(0o755)

    @property
    def endpoint(self) -> dict[str, str]:
        state = self.state()
        return {"executable": state["zmx"], "socketDirectory": state["socketDirectory"]}

    def state(self) -> dict[str, Any]:
        return json.loads((self.directory / "state.json").read_text())

    def _save(self, state: dict[str, Any]) -> None:
        fake_agterm.save_state(self.directory, state)

    def update(self, change: Callable[[dict[str, Any]], None]) -> None:
        state = self.state()
        change(state)
        self._save(state)

    def add_session(
        self,
        name: str,
        cwd: str = "/Users/me/repo",
        split: bool = False,
        foreground: list[str] | None = None,
        context: str | None = None,
        snapshot: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """A live session of the app's, every pane's daemon running; its listing entry."""
        state = self.state()
        panes = [{"pane": "left", "daemon": f"agterm-{uuid.uuid4().hex}"}]
        if foreground:
            panes[0]["foreground"] = foreground
        if split:
            panes.append({"pane": "right", "daemon": f"agterm-{uuid.uuid4().hex}"})
        session: dict[str, Any] = {
            "id": str(uuid.uuid4()).upper(),
            "name": name,
            "windowID": state["window"]["id"],
            "windowName": state["window"]["name"],
            "workspaceID": state["workspace"]["id"],
            "workspaceName": state["workspace"]["name"],
            "cwd": cwd,
            "panes": panes,
        }
        if split:
            session["splitAxis"] = "horizontal"
        if context is not None:
            session["context"] = context
        if snapshot is not None:
            session["snapshot"] = snapshot
        state["sessions"].append(session)
        state["daemons"].extend(pane["daemon"] for pane in panes)
        self._save(state)
        return session

    def session(self, session_id: str) -> dict[str, Any] | None:
        return next((s for s in self.state()["sessions"] if s["id"] == session_id), None)

    def end_daemons(self, session: dict[str, Any]) -> None:
        """The session's daemons exit while the app still lists the session."""
        gone = {pane["daemon"] for pane in session["panes"]}
        self.update(lambda state: state.update(daemons=[d for d in state["daemons"] if d not in gone]))

    def vanish_on_attach(self, session: dict[str, Any]) -> None:
        """The session's daemons are gone by the time a pane's attach reaches zmx, while the app still lists them."""
        gone = [pane["daemon"] for pane in session["panes"]]
        self.update(lambda state: state.update(vanished=[*state.get("vanished", []), *gone]))

    def remove_all(self) -> None:
        self.update(lambda state: state.update(sessions=[], daemons=[], vanished=[]))

    @staticmethod
    def pane_identity(daemon: str) -> str:
        """The origin pane identity a daemon name encodes, as the wire spells a `PresentationPane`."""
        return str(uuid.UUID(daemon.removeprefix("agterm-"))).upper()

    # -- what the viewer ran there

    def calls(self, program: str | None = None) -> list[dict[str, Any]]:
        try:
            lines = (self.directory / "calls.jsonl").read_text().splitlines()
        except FileNotFoundError:
            return []
        entries = [json.loads(line) for line in lines if line.strip()]
        return [entry for entry in entries if program is None or entry["program"] == program]

    def wait_call(self, predicate: Callable[[dict[str, Any]], bool], program: str | None = None, timeout: float = 30):
        def found():
            calls = self.calls(program)
            match = next((call for call in calls if predicate(call)), None)
            assert match is not None, f"no matching {program or 'call'} on {self.machine.alias}: {calls}"
            return match

        return wait_until(found, timeout=timeout)

    # -- the presentation stream

    def received(self, session_id: str) -> list[dict[str, Any]]:
        path = self.directory / "present" / f"{session_id}.received.jsonl"
        try:
            return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
        except FileNotFoundError:
            return []

    def wait_received(self, session_id: str, kind: str, timeout: float = 30) -> dict[str, Any]:
        def found():
            frames = self.received(session_id)
            match = next((frame for frame in frames if frame.get("kind") == kind), None)
            assert match is not None, f"no {kind} frame from the viewer for {session_id}: {frames}"
            return match

        return wait_until(found, timeout=timeout)

    def push(self, session_id: str, kind: str, **payload: Any) -> None:
        """A frame the app sends on the session's open streams (`gen` and `rev` are the stream's)."""
        path = self.directory / "present" / f"{session_id}.push.jsonl"
        path.parent.mkdir(exist_ok=True)
        with open(path, "a", encoding="utf-8") as pending:
            pending.write(json.dumps({"kind": kind, **payload}) + "\n")

    # -- ngterm beside it

    def install_ngtermctl(self, ctl: Path, state_dir: Path) -> None:
        """This build's CLI on the machine, answering for the instance whose state directory is `state_dir`."""
        wrapper = self.machine.bin / "ngtermctl"
        wrapper.write_text(
            "#!/bin/sh\n"
            f"NGTERM_STATE_DIR={shlex.quote(os.fspath(state_dir))} exec {shlex.quote(os.fspath(ctl))} \"$@\"\n"
        )
        wrapper.chmod(0o755)


class AgtermWorld:
    """The viewer's ssh client and the machines it reaches over loopback. `stop` ends every machine's sshd."""

    def __init__(self, root: Path) -> None:
        if sys.platform == "win32":
            pytest.skip("the loopback machine is POSIX only: a Windows host runs no unprivileged sshd")
        self._sshd, ssh, self._keygen = require_openssh()
        self.root = root
        self.client = SshClient(root, ssh, self._keygen)
        self.machines: list[LoopbackMachine] = []

    def machine(self, alias: str) -> LoopbackMachine:
        machine = LoopbackMachine(self.root, alias, self.client.public_key)
        self.machines.append(machine)
        machine.start(self._sshd, self._keygen)
        self.client.add(machine)
        self.client.warm(machine)
        return machine

    def agterm_mac(self, alias: str) -> AgtermMac:
        return AgtermMac(self.machine(alias))

    def stop(self) -> None:
        for machine in self.machines:
            machine.stop()
