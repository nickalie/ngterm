#!/usr/bin/env python3
"""A stand-in Claude Code for the mobile e2e test (headless_e2e.rs): it runs in a pane's foreground as the real TUI
would, so the origin's agent record, `ngtermctl agent chat` and the phone's chat actions meet a pane that holds an agent.

- At start it writes the transcript (a copy of the backlog the test made from Task 1's fixture) and pipes the
  SessionStart payload to `ngtermctl agent hook claude SessionStart`, the pane and the private control socket named
  on the command line and no NGTERM_* in its environment, as the restore adapter does for the real agent.
- It draws Claude Code 2.1.317's idle screen by replaying Task 1's PTY capture recorded at the pane's grid
  (tests/fixtures/agent_screens/claude-<cols>x<rows>.bin up to its `idle` mark), again on every resize, so the phone's
  screen matcher reads the screen it was written against. A grid with no capture shows a line saying so until the
  next resize.
- It takes input as the TUI does: a bracketed paste or typed text goes into the composer, drawn where the capture
  parks the cursor (the composer's first column) in a plain style over the faint placeholder; Enter submits the
  composer: the prompt is appended to the transcript as Claude writes a user line, and shown above the prompt box as
  `> text`. Backspace drops a character. The replies to the terminal queries the capture makes, focus and mouse
  reports are read and dropped.
- `/exit` submitted ends it: the screen modes are restored and it exits 0, with no SessionEnd, so the pane's
  foreground goes back to the shell and the origin clears the record on its own.
- The test moves it between states by writing the state's name to the `control` file and sending it SIGUSR1 (its pid
  is in the `pid` file): `idle`; `working`, a turn in progress (the capture's `working` screen), where a submitted
  prompt is absorbed into the turn as Claude Code 2.1.319+ writes it (a `queue-operation` and a `queued_command`
  attachment, no user line); `question`, the `AskUserQuestion` picker (the capture's `question` screen) of the
  `question` input, whose call Claude Code writes only once it is answered: a digit picks that option of its first
  question, writes the call and its result, and goes back to idle.

- With `agent` `codex` it stands in for Codex 0.167.2 instead: it reports through `agent hook codex`, draws the
  recorded `codex-questions` capture, and appends the recorded rollout's line `groups` as Codex wrote them:
  `permission` shows the approval dialog, where `y` (Yes, proceed) appends `approve` and goes back to idle;
  `question` appends `ask` (the `request_user_input` call) and shows its picker, where `2` (Green) appends `answer`.
  `exit` ends it, as Claude's `/exit` does.

Usage: fake_agent.py CONFIG.json (the keys: agent, capture, transcript, backlog, payload, user_template, screens,
ngtermctl, socket, target, pane_id, control, pid, question, groups).
"""

import codecs
import copy
import datetime
import json
import os
import re
import select
import shutil
import signal
import subprocess
import sys
import termios
import tty
import uuid

# The row the submitted prompts start at: under the capture's banner at every recorded grid, above its prompt box.
HISTORY_TOP = 10
# The most submitted prompts shown.
HISTORY_ROWS = 6
# What the capture turns on, turned off again on exit.
RESTORE = b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2031l\x1b[?2004l\x1b[0m\x1b[?1049l"
PASTE_START = "\x1b[200~"
PASTE_END = "\x1b[201~"


def load_captures(directory, prefix):
    """{(cols, rows): {state: (the capture's bytes up to the state's mark, the cursor's (row, col) where it parks it)}}
    of the captures named `<prefix>-<cols>x<rows>`."""
    captures = {}
    for name in sorted(os.listdir(directory)):
        grid = name[len(prefix) + 1 : -len(".marks.json")] if name.endswith(".marks.json") else ""
        if not (name.startswith(prefix + "-") and re.fullmatch(r"\d+x\d+", grid)):
            continue
        with open(os.path.join(directory, name), encoding="utf-8") as file:
            marks = json.load(file)
        with open(os.path.join(directory, name[: -len(".marks.json")] + ".bin"), "rb") as file:
            recorded = file.read()
        screens = {}
        for state, mark in marks["marks"].items():
            drawn = recorded[:mark]
            row, col = re.findall(rb"\x1b\[(\d+);(\d+)H", drawn)[-1]
            screens[state] = (drawn, (int(row), int(col)))
        captures[(marks["cols"], marks["rows"])] = screens
    return captures


def report(config):
    """Pipes the SessionStart payload to `ngtermctl agent hook`, addressed and aimed at the socket explicitly."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("NGTERM_")}
    command = [
        config["ngtermctl"], "agent", "hook", config["agent"], "SessionStart",
        "--target", config["target"], "--pane", "primary", "--pane-id", config["pane_id"],
        "--socket", config["socket"],
    ]
    with open(config["payload"], "rb") as payload:
        done = subprocess.run(command, stdin=payload, capture_output=True, env=env, check=False)
    if done.returncode != 0:
        sys.stderr.write(f"fake agent: agent hook failed ({done.returncode}): {done.stdout!r} {done.stderr!r}\r\n")
        sys.exit(1)


class Input:
    """The terminal's input as the TUI reads it: text, pastes and Enter; escape sequences (query replies, focus and
    mouse reports) dropped."""

    def __init__(self):
        self.decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        self.pending = ""

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        actions = []
        while self.pending:
            taken = self.take(actions)
            if taken == 0:
                break  # an escape sequence or a paste not all read yet
            self.pending = self.pending[taken:]
        return actions

    def take(self, actions):
        """Reads one item at the start of the input into `actions`: how much it took, 0 when it is cut short."""
        text = self.pending
        if text.startswith(PASTE_START):
            end = text.find(PASTE_END)
            if end < 0:
                return 0
            actions.append(("paste", text[len(PASTE_START) : end].replace("\r\n", "\n").replace("\r", "\n")))
            return end + len(PASTE_END)
        if text[0] == "\x1b":
            return escape_length(text)
        if text[0] in "\r\n":
            actions.append(("enter", None))
        elif text[0] in "\x7f\x08":
            actions.append(("backspace", None))
        elif text[0] >= " ":
            actions.append(("text", text[0]))
        return 1


def escape_length(text):
    """The length of the escape sequence `text` starts with, 0 when it is cut short."""
    if len(text) < 2:
        return 0
    kind = text[1]
    if kind == "[":
        for index in range(2, len(text)):
            if "\x40" <= text[index] <= "\x7e":
                return index + 1
        return 0
    if kind in "]P_^":
        # an OSC ends with BEL or ST, a DCS, APC or PM with ST
        bell = text.find("\x07", 2) if kind == "]" else -1
        st = text.find("\x1b\\", 2)
        ends = [end + 1 for end in [bell] if end >= 0] + [end + 2 for end in [st] if end >= 0]
        return min(ends) if ends else 0
    return 2


class Agent:
    def __init__(self, config, captures):
        self.config = config
        self.captures = captures
        self.composer = ""
        self.history = []
        self.parent = None
        self.state = "idle"
        # what the control file said when last read: a resize's wake reads it again, and only a new word moves
        self.asked = ""

    def size(self):
        size = os.get_terminal_size(sys.stdout.fileno())
        return size.columns, size.lines

    def draw(self):
        cols, rows = self.size()
        capture = self.captures.get((cols, rows), {}).get(self.state)
        if capture is None:
            out(f"\x1b[?1049h\x1b[2J\x1b[Hfake agent: no recorded {self.state} screen at {cols}x{rows}".encode())
            return
        drawn, (row, col) = capture
        frame = bytearray(drawn)
        for index, line in enumerate(self.history[-HISTORY_ROWS:]):
            frame += f"\x1b[{HISTORY_TOP + index};1H\x1b[0m{f'> {line}'[:cols]}".encode()
        if self.composer:
            frame += f"\x1b[{row};{col}H\x1b[0m\x1b[K{self.composer}".encode()
        out(bytes(frame))

    def submit(self):
        text, self.composer = self.composer, ""
        if text.strip() == "/exit":
            return False
        if self.state == "working":
            self.absorb(text)
        else:
            self.write_prompt(text)
        self.history.append(" ".join(text.split()))
        return True

    def write_prompt(self, text):
        """Appends `text` to the transcript as Claude Code writes a typed prompt."""
        line = copy.deepcopy(self.config["user_template"])
        line["uuid"] = str(uuid.uuid4())
        line["parentUuid"] = self.parent
        line["promptId"] = str(uuid.uuid4())
        line["message"]["content"] = text
        line["timestamp"] = now()
        self.parent = line["uuid"]
        self.append([line])

    def stamped(self, line):
        line["timestamp"] = now()
        line["sessionId"] = self.config["user_template"]["sessionId"]
        return line

    def append(self, lines):
        with open(self.config["transcript"], "a", encoding="utf-8") as file:
            file.write("".join(json.dumps(line, separators=(",", ":")) + "\n" for line in lines))

    def absorb(self, text):
        """Takes `text` into the running turn, as Claude Code does with a prompt sent while it works."""
        session = self.config["user_template"]["sessionId"]
        source = str(uuid.uuid4())
        enqueue = {"type": "queue-operation", "operation": "enqueue", "timestamp": now(), "sessionId": session,
                   "content": text}
        remove = {"type": "queue-operation", "operation": "remove", "timestamp": now(), "sessionId": session,
                  "content": text, "reason": "absorbed_mid_turn", "commandUuid": source}
        attachment = self.stamped({
            "parentUuid": self.parent, "isSidechain": False, "type": "attachment", "uuid": str(uuid.uuid4()),
            "attachment": {"type": "queued_command", "prompt": text, "source_uuid": source, "commandMode": "prompt",
                           "origin": {"kind": "human"}, "timestamp": now(), "humanTurn": True},
        })
        self.parent = attachment["uuid"]
        self.append([enqueue, remove, attachment])

    def answer(self, digit):
        """Picks option `digit` of the question's first question: the call and its result, written together."""
        question = self.config["question"]["questions"][0]
        if not 1 <= digit <= len(question["options"]):
            return
        label = question["options"][digit - 1]["label"]
        call = self.stamped(copy.deepcopy(self.config["assistant_template"]))
        call["uuid"], call["parentUuid"] = str(uuid.uuid4()), self.parent
        call["message"]["id"] = "msg_e2e_question"
        call["message"]["content"] = [{"type": "tool_use", "id": QUESTION_CALL, "name": "AskUserQuestion",
                                       "input": self.config["question"]}]
        result = self.stamped(copy.deepcopy(self.config["user_template"]))
        result["uuid"], result["parentUuid"] = str(uuid.uuid4()), call["uuid"]
        result["message"]["content"] = [{"type": "tool_result", "tool_use_id": QUESTION_CALL,
                                         "content": f'User has answered your questions: "{question["question"]}"="{label}".'}]
        self.parent = result["uuid"]
        self.append([call, result])
        self.state = "idle"

    def group(self, name):
        """Appends the recorded rollout lines of group `name`."""
        with open(self.config["transcript"], "a", encoding="utf-8") as file:
            file.write("".join(line + "\n" for line in self.config["groups"][name]))

    def codex(self, action, value):
        """Codex's dialogs: the approval's `y`, the picker's `2`; anything else is left to the screen as it is."""
        if action == "text" and (self.state, value) in (("permission", "y"), ("question", "2")):
            self.group("approve" if self.state == "permission" else "answer")
            self.state = "idle"
        self.draw()
        return True

    def handle(self, action, value):
        """Applies one input action; False once the agent should exit."""
        if self.config["agent"] == "codex":
            return self.codex(action, value)
        if self.state == "question":
            if action == "text" and value.isdigit():
                self.answer(int(value))
            self.draw()
            return True
        if action == "enter":
            if self.composer.strip() and not self.submit():
                return False
        elif action == "backspace":
            self.composer = self.composer[:-1]
        else:
            self.composer += value
        self.draw()
        return True

    def run(self, wake):
        stdin = sys.stdin.fileno()
        reader = Input()
        self.draw()
        while True:
            ready, _, _ = select.select([stdin, wake], [], [])
            if wake in ready:
                os.read(wake, 512)
                if not self.take_control():
                    return
                self.draw()
            if stdin in ready:
                data = os.read(stdin, 65536)
                if not data:
                    return
                for action, value in reader.feed(data):
                    if not self.handle(action, value):
                        return

    def take_control(self):
        """Moves to the state the test asked for, if it names one; False when it asked the program to exit."""
        try:
            with open(self.config["control"], encoding="utf-8") as file:
                state = file.read().strip()
        except FileNotFoundError:
            return True
        if state == self.asked:
            return True
        self.asked = state
        if state == "exit":
            return False
        if state == "question" and self.config["agent"] == "codex":
            self.group("ask")
        if state:
            self.state = state
        return True


QUESTION_CALL = "toolu_e2e_question"


def now():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"


def out(data):
    view = memoryview(data)
    while view:
        view = view[os.write(sys.stdout.fileno(), view) :]


def main():
    with open(sys.argv[1], encoding="utf-8") as file:
        config = json.load(file)
    captures = load_captures(config["screens"], config["capture"])
    os.makedirs(os.path.dirname(config["transcript"]), exist_ok=True)
    shutil.copyfile(config["backlog"], config["transcript"])
    report(config)
    # SIGWINCH wakes the loop through this pipe: a handler alone would not end the select
    wake, waker = os.pipe()
    os.set_blocking(waker, False)
    signal.set_wakeup_fd(waker)
    signal.signal(signal.SIGWINCH, lambda *_: None)
    signal.signal(signal.SIGUSR1, lambda *_: None)
    with open(config["pid"], "w", encoding="utf-8") as file:
        file.write(str(os.getpid()))
    stdin = sys.stdin.fileno()
    saved = termios.tcgetattr(stdin)
    tty.setraw(stdin)
    try:
        Agent(config, captures).run(wake)
    finally:
        out(RESTORE)
        termios.tcsetattr(stdin, termios.TCSADRAIN, saved)


if __name__ == "__main__":
    main()
