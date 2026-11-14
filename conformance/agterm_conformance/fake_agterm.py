"""A stand-in for upstream agterm on a Mac, run on a loopback machine (`agterm_mac.AgtermMac`): its `agtermctl` and its
bundled `zmx`, answering what a viewer runs over ssh the way agterm v0.41.1 does. An outside service the suite may fake,
built from upstream's own sources and fixtures, never from this build's code:

- `agtermctl zmx tree --json`: the ok `ControlResponse` whose `result.remote` is `RemoteTreeMerger.candidates`'
  `ControlRemoteTree` (`RemoteTree.swift`; the fixture shape of `ControlServerRemoteGroupTests.swift`);
- `agtermctl zmx present SESSION`: the presentation stream (`PresentationFrames.swift`, `PresentationHub.swift`): the
  viewer's hello first, answered by hello with the kinds both speak and the snapshot, then pings, acks, the presenter
  grant and whatever the test pushes; stdout carries frames and nothing else, a refused session exits 1 with its
  reason on stderr (`ZmxCommands.swift` `Present`);
- `agtermctl restore mode`, `session new|duplicate|close` and `session overlay run-job`, each `--json` answered as a
  `ControlResponse`;
- `zmx attach DAEMON COMMAND...`: attaches a live daemon, else creates it running COMMAND, as zmx does, under the
  explicit leadership of agterm's zmx (`scripts/zmx-patches/0001-explicit-leadership.patch`, `ZmxLead.swift`). The
  app's own window is attached to every daemon it lists and leads it, so a client that opted in (`ZMX_MANAGED`)
  follows unless it claims (`ZMX_MANAGED_CLAIM`) or no client leads; it is told its role as a
  `zmx-role;<nonce>:<role>:<generation>` title before anything else. The leader gets the pane's shell; a follower's
  terminal is raw and what it sends is dropped, and it is told `unowned` once the leader has left. Short of zmx: each
  leader runs a shell of its own, a leader that lost the lead to a claim is not told, and an unmanaged client leads
  at once (zmx's takes the lead with its first key).

Run as `python -I fake_agterm.py MACHINE_DIR PROGRAM ARGS...`. The machine's `state.json` holds what the app shows;
every invocation is logged to `calls.jsonl`, each stream's received frames to `present/<session>.received.jsonl`, and
frames a test appends to `present/<session>.push.jsonl` are sent on the session's open streams. The listing alone is
printed by the `agtermctl` wrapper from `listing.json` (`save_state`), without starting Python.
"""

from __future__ import annotations

import json
import os
import select
import sys
import threading
import time
import uuid
from pathlib import Path

PRESENTATION_VERSION = 1
# `PresentationHub.supportedKinds`
SUPPORTED_KINDS = [
    "status", "hud", "notify", "context", "info", "layout", "sound", "ask.form", "ask.other", "overlay.points",
    "page.content", "page.document", "status.shapes",
    "page.request", "page.rejected", "page.close", "page.closed", "page.command", "page.state",
]
PING_SECONDS = 5.0
PUSH_POLL = 0.05
# the leader of a daemon no client has claimed: the app's own window
WINDOW = "window"
LEAD_POLL = 0.2
LOGGED_VARIABLES = (
    "ZMX_SESSION", "ZMX_SESSION_PREFIX", "ZMX_NO_DETACH_KEY", "ZMX_DIR", "ZMX_MANAGED", "ZMX_MANAGED_CLAIM", "HOME",
)


def _write_json(path: Path, value) -> None:
    temporary = path.with_suffix(f".{os.getpid()}.tmp")
    temporary.write_text(json.dumps(value, indent=1))
    os.replace(temporary, path)


def _encode(value) -> str:
    # `PresentationCodec.encode`: sorted keys, no escaped slashes, one line
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


class Machine:
    def __init__(self, directory: Path) -> None:
        self.directory = directory
        self.state_path = directory / "state.json"
        self.present = directory / "present"

    def state(self):
        return json.loads(self.state_path.read_text())

    def save(self, state) -> None:
        save_state(self.directory, state)

    def log(self, program: str, argv: list[str]) -> None:
        entry = {
            "program": program,
            "argv": argv,
            "env": {name: os.environ[name] for name in LOGGED_VARIABLES if name in os.environ},
            "ngterm": sorted(name for name in os.environ if name.startswith("NGTERM_")),
        }
        with open(self.directory / "calls.jsonl", "a", encoding="utf-8") as log:
            log.write(json.dumps(entry) + "\n")


def _reply(result=None, error=None) -> int:
    if error is not None:
        print(_encode({"ok": False, "error": error}))
        return 1
    print(_encode({"ok": True, "result": result} if result is not None else {"ok": True}))
    return 0


def _new_session(state, name: str, cwd: str, source=None) -> dict:
    identity = uuid.uuid4()
    session = {
        "id": str(uuid.uuid4()).upper(),
        "name": name,
        "windowID": state["window"]["id"],
        "windowName": state["window"]["name"],
        "workspaceID": state["workspace"]["id"],
        "workspaceName": state["workspace"]["name"],
        "cwd": cwd,
        "panes": [{"pane": "left", "daemon": f"agterm-{identity.hex}"}],
    }
    if source is not None:
        session["duplicateOf"] = source
    state["sessions"].append(session)
    state["daemons"].append(session["panes"][0]["daemon"])
    return session


def listing(state) -> dict:
    """`zmx tree --json`'s answer for `state`: the sessions whose every daemon runs."""
    endpoint = {"executable": state["zmx"], "socketDirectory": state["socketDirectory"]}
    private = {"duplicateOf", "snapshot"}
    alive = set(state["daemons"])
    sessions = [
        {k: v for k, v in s.items() if k not in private}
        for s in state["sessions"]
        if all(pane["daemon"] in alive for pane in s["panes"])
    ]
    remote = {"endpoint": endpoint, "presentation": PRESENTATION_VERSION, "sessions": sessions}
    return {"ok": True, "result": {"remote": remote}}


def save_state(directory: Path, state) -> None:
    """The state, and the listing the `agtermctl` wrapper prints without starting Python (`listing.json`)."""
    _write_json(directory / "state.json", state)
    temporary = directory / f"listing.{os.getpid()}.tmp"
    temporary.write_text(_encode(listing(state)) + "\n")
    os.replace(temporary, directory / "listing.json")


def agtermctl(machine: Machine, argv: list[str]) -> int:
    machine.log("agtermctl", argv)
    words = [word for word in argv if word != "--json"]
    state = machine.state()
    if not state.get("running", True):
        print("Error: agterm is not running", file=sys.stderr)
        return 1
    if words[:2] == ["zmx", "tree"]:
        print(_encode(listing(state)))
        return 0
    if words[:2] == ["zmx", "present"] and len(words) == 3:
        return present(machine, state, words[2])
    if words[:2] == ["restore", "mode"]:
        # `ControlRestoreStatus` (ControlPayloads.swift)
        mode = state.get("restore", "live")
        return _reply(
            {"restore": {"configured": mode, "requestedAtLaunch": mode, "active": mode, "restartRequired": False}}
        )
    if words[:2] == ["session", "new"]:
        session = _new_session(state, f"agterm {len(state['sessions']) + 1}", state["home"])
        machine.save(state)
        return _reply({"id": session["id"]})
    if words[:3] == ["session", "duplicate", "--target"] and len(words) == 4:
        source = next((s for s in state["sessions"] if s["id"] == words[3]), None)
        if source is None:
            return _reply(error="no such session")
        session = _new_session(state, f"{source['name']} copy", source["cwd"], source=source["id"])
        machine.save(state)
        return _reply({"id": session["id"]})
    if words[:3] == ["session", "close", "--target"] and len(words) == 4:
        source = next((s for s in state["sessions"] if s["id"] == words[3]), None)
        if source is None:
            return _reply(error="no such session")
        state["sessions"].remove(source)
        state["daemons"] = [d for d in state["daemons"] if d not in {p["daemon"] for p in source["panes"]}]
        machine.save(state)
        return _reply({"id": source["id"]})
    if words[:3] == ["session", "overlay", "run-job"] and len(words) == 4:
        sys.stdout.write(f"agterm overlay job {words[3]}\n")
        sys.stdout.flush()
        os.execv("/bin/sh", ["/bin/sh", "-c", "read line; printf 'overlay read %s\\n' \"$line\""])
    print(f"Error: the stand-in agterm does not answer {' '.join(words)!r}", file=sys.stderr)
    return 64


def _layout(session) -> dict:
    panes = [str(uuid.UUID(pane["daemon"].removeprefix("agterm-"))).upper() for pane in session["panes"]]
    layout = {"panes": panes, "primary": panes[0], "shown": len(panes) > 1}
    if len(panes) > 1:
        layout["axis"] = session.get("splitAxis", "horizontal")
    return layout


def present(machine: Machine, state, session_id: str) -> int:
    session = next((s for s in state["sessions"] if s["id"] == session_id), None)
    if session is None:
        print("Error: no such session", file=sys.stderr)
        return 1
    machine.present.mkdir(exist_ok=True)
    received = machine.present / f"{session_id}.received.jsonl"
    push = machine.present / f"{session_id}.push.jsonl"
    push.touch()
    offset = push.stat().st_size
    counter = machine.present / f"{session_id}.connections"
    generation = int(counter.read_text()) + 1 if counter.exists() else 1
    counter.write_text(str(generation))
    lock = threading.Lock()
    revision = [0]
    done = threading.Event()
    # set once hello and the snapshot are out: agterm's stream sends nothing else before them, and a viewer drops a
    # frame that comes before its hello (a HUD before the snapshot would be cleared by it)
    greeted = threading.Event()

    def send(kind: str, **payload) -> None:
        with lock:
            revision[0] += 1
            frame = {"kind": kind, "gen": generation, "rev": revision[0], **payload}
            try:
                sys.stdout.write(_encode(frame) + "\n")
                sys.stdout.flush()
            except (BrokenPipeError, ValueError):
                done.set()

    def reader() -> None:
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            with open(received, "a", encoding="utf-8") as log:
                log.write(line + "\n")
            frame = json.loads(line)
            kind = frame.get("kind")
            if not greeted.is_set():
                if kind != "hello":
                    break  # `PresentationStream.receive`: a stream that did not open with hello is closed
                hello = frame["hello"]
                kinds = [k for k in SUPPORTED_KINDS if k in hello.get("kinds", [])]
                version = min(PRESENTATION_VERSION, int(hello["version"]))
                send("hello", hello={"version": version, "kinds": kinds, "mode": hello["mode"]})
                current = next((s for s in machine.state()["sessions"] if s["id"] == session_id), session)
                snapshot = dict(current.get("snapshot", {}))
                snapshot.setdefault("layout", _layout(current))
                send("snapshot", snapshot=snapshot)
                greeted.set()
            elif kind == "ping":
                send("ack")
            elif kind == "presenter.acquire":
                send("presenter.granted")
        done.set()

    threading.Thread(target=reader, daemon=True).start()
    last_ping = time.monotonic()
    while not done.wait(PUSH_POLL):
        if not greeted.is_set():
            # what a test pushes meanwhile stays in the file, from `offset`, until the handshake is out
            last_ping = time.monotonic()
            continue
        with open(push, "rb") as pending:
            pending.seek(offset)
            chunk = pending.read()
        complete = chunk[: chunk.rfind(b"\n") + 1]
        offset += len(complete)
        for line in complete.decode().splitlines():
            if line.strip():
                body = json.loads(line)
                send(body.pop("kind"), **body)
        if time.monotonic() - last_ping >= PING_SECONDS:
            send("ping")
            last_ping = time.monotonic()
    return 0


def _alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _lead(machine: Machine, daemon: str, take: bool = False) -> dict:
    """The daemon's lead in `leads.json`, `{"leader": WINDOW | pid | None, "generation": n}`, after a leader that
    exited has left it unowned; with `take`, this process leads it."""
    import fcntl  # POSIX only: the module stays importable on Windows, where the suite collects it and runs none of it

    with open(machine.directory / "leads.lock", "w", encoding="utf-8") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        path = machine.directory / "leads.json"
        leads = json.loads(path.read_text()) if path.exists() else {}
        lead = leads.get(daemon, {"leader": WINDOW, "generation": 1})
        before = dict(lead)
        if isinstance(lead["leader"], int) and not _alive(lead["leader"]):
            lead = {"leader": None, "generation": lead["generation"] + 1}
        if take:
            lead = {"leader": os.getpid(), "generation": lead["generation"] + 1}
        if lead != before or daemon not in leads:
            leads[daemon] = lead
            _write_json(path, leads)
        return lead


def _tell(nonce: str, role: str, generation: int) -> None:
    sys.stdout.write(f"\x1b]2;zmx-role;{nonce}:{role}:{generation}\x1b\\")
    sys.stdout.flush()


def _follow(machine: Machine, daemon: str, nonce: str) -> int:
    """A managed follower: its terminal raw as zmx's client sets it, everything it sends dropped, told when the leader
    leaves; until its viewer hangs up."""
    import tty  # POSIX only, as `fcntl` in `_lead`

    stdin = sys.stdin.fileno()
    if os.isatty(stdin):
        tty.setraw(stdin)
    sys.stdout.write(f"attached {daemon}\r\nagterm$ ")
    sys.stdout.flush()
    told = "follower"
    while True:
        ready, _, _ = select.select([stdin], [], [], LEAD_POLL)
        if ready:
            if not os.read(stdin, 4096):
                return 0
            continue
        lead = _lead(machine, daemon)
        role = "unowned" if lead["leader"] is None else "follower"
        if role != told:
            told = role
            _tell(nonce, role, lead["generation"])


def zmx(machine: Machine, argv: list[str]) -> int:
    machine.log("zmx", argv)
    if len(argv) < 2 or argv[0] != "attach":
        print("zmx: the stand-in answers attach only", file=sys.stderr)
        return 2
    daemon, command = argv[1], argv[2:]
    state = machine.state()
    if daemon not in state["daemons"] or daemon in state.get("vanished", []):
        # zmx creates a missing daemon running the command; the viewer's command is its create-only guard
        os.execv(command[0], command)
    nonce = os.environ.get("ZMX_MANAGED")
    leads = not nonce or "ZMX_MANAGED_CLAIM" in os.environ or _lead(machine, daemon)["leader"] is None
    lead = _lead(machine, daemon, take=leads)
    if nonce:
        _tell(nonce, "leader" if leads else "follower", lead["generation"])
    if not leads:
        return _follow(machine, daemon, nonce)
    sys.stdout.write(f"attached {daemon}\r\n")
    sys.stdout.flush()
    os.execve("/bin/sh", ["/bin/sh", "-i"], {**os.environ, "PS1": "agterm$ ", "ENV": ""})
    return 0


def main() -> int:
    directory, program, *argv = sys.argv[1:]
    machine = Machine(Path(directory))
    return agtermctl(machine, argv) if program == "agtermctl" else zmx(machine, argv)


if __name__ == "__main__":
    sys.exit(main())
