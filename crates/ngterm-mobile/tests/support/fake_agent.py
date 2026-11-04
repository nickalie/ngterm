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

Usage: fake_agent.py CONFIG.json (the keys: transcript, backlog, payload, user_template, screens, ngtermctl, socket,
target, pane_id).
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


def load_captures(directory):
    """{(cols, rows): (the capture's bytes up to its idle mark, the cursor's (row, col) where it parks it)}."""
    captures = {}
    for name in sorted(os.listdir(directory)):
        if not (name.startswith("claude-") and name.endswith(".marks.json")):
            continue
        with open(os.path.join(directory, name), encoding="utf-8") as file:
            marks = json.load(file)
        with open(os.path.join(directory, name[: -len(".marks.json")] + ".bin"), "rb") as file:
            idle = file.read()[: marks["marks"]["idle"]]
        moves = re.findall(rb"\x1b\[(\d+);(\d+)H", idle)
        row, col = moves[-1]
        captures[(marks["cols"], marks["rows"])] = (idle, (int(row), int(col)))
    return captures


def report(config):
    """Pipes the SessionStart payload to `ngtermctl agent hook`, addressed and aimed at the socket explicitly."""
    env = {key: value for key, value in os.environ.items() if not key.startswith("NGTERM_")}
    command = [
        config["ngtermctl"], "agent", "hook", "claude", "SessionStart",
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

    def size(self):
        size = os.get_terminal_size(sys.stdout.fileno())
        return size.columns, size.lines

    def draw(self):
        cols, rows = self.size()
        capture = self.captures.get((cols, rows))
        if capture is None:
            out(f"\x1b[?1049h\x1b[2J\x1b[Hfake agent: no recorded idle screen at {cols}x{rows}".encode())
            return
        idle, (row, col) = capture
        frame = bytearray(idle)
        for index, line in enumerate(self.history[-HISTORY_ROWS:]):
            frame += f"\x1b[{HISTORY_TOP + index};1H\x1b[0m{f'> {line}'[:cols]}".encode()
        if self.composer:
            frame += f"\x1b[{row};{col}H\x1b[0m\x1b[K{self.composer}".encode()
        out(bytes(frame))

    def submit(self):
        text, self.composer = self.composer, ""
        if text.strip() == "/exit":
            return False
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
        line["timestamp"] = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
        self.parent = line["uuid"]
        with open(self.config["transcript"], "a", encoding="utf-8") as file:
            file.write(json.dumps(line, separators=(",", ":")) + "\n")

    def handle(self, action, value):
        """Applies one input action; False once the agent should exit."""
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
                self.draw()
            if stdin in ready:
                data = os.read(stdin, 65536)
                if not data:
                    return
                for action, value in reader.feed(data):
                    if not self.handle(action, value):
                        return


def out(data):
    view = memoryview(data)
    while view:
        view = view[os.write(sys.stdout.fileno(), view) :]


def main():
    with open(sys.argv[1], encoding="utf-8") as file:
        config = json.load(file)
    captures = load_captures(config["screens"])
    os.makedirs(os.path.dirname(config["transcript"]), exist_ok=True)
    shutil.copyfile(config["backlog"], config["transcript"])
    report(config)
    # SIGWINCH wakes the loop through this pipe: a handler alone would not end the select
    wake, waker = os.pipe()
    os.set_blocking(waker, False)
    signal.set_wakeup_fd(waker)
    signal.signal(signal.SIGWINCH, lambda *_: None)
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
