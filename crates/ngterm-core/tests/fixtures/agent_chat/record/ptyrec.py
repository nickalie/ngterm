#!/usr/bin/env python3
"""Run a TUI under a PTY, record every byte it writes, and drive it over a control socket.

Used to record the agent_chat fixtures (README.md next to this directory says how). Needs `pyte`
(`pip install pyte`) for the screen model that answers the terminal queries and that `screen`
and `wait` read.

    ptyrec.py serve --ctl SOCK --log OUT.bin --cols C --rows R [--env K=V ...] -- CMD ARGS...
    ptyrec.py ctl SOCK screen            # the visible screen as text
    ptyrec.py ctl SOCK send TEXT         # TEXT is unescaped: \\r \\x1b \\x03 ...
    ptyrec.py ctl SOCK paste TEXT        # TEXT inside a bracketed paste
    ptyrec.py ctl SOCK wait REGEX [SECS] # wait until the screen matches (default 60 s)
    ptyrec.py ctl SOCK quiet [MS] [SECS] # wait until no output for MS (default 1500)
    ptyrec.py ctl SOCK mark LABEL        # record the log's byte offset under LABEL in OUT.marks.json
    ptyrec.py ctl SOCK kill

The child's environment is exactly the --env pairs: nothing is inherited.
"""

import argparse
import codecs
import json
import os
import pty
import re
import select
import signal
import socket
import struct
import sys
import termios
import fcntl
import threading
import time

import pyte


class Recorder:
    def __init__(self, args):
        self.cols, self.rows = args.cols, args.rows
        self.log = open(args.log, "wb")
        self.marks_path = os.path.splitext(args.log)[0] + ".marks.json"
        self.marks = {"cols": self.cols, "rows": self.rows, "marks": {}}
        self.offset = 0
        self.last_output = time.monotonic()
        self.lock = threading.Lock()
        self.screen = pyte.Screen(self.cols, self.rows)
        self.screen.write_process_input = self.reply
        self.stream = pyte.ByteStream(self.screen)
        env = dict(kv.split("=", 1) for kv in args.env)
        pid, fd = pty.fork()
        if pid == 0:
            os.chdir(args.cwd)
            os.execvpe(args.cmd[0], args.cmd, env)
        self.pid, self.fd = pid, fd
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", self.rows, self.cols, 0, 0))
        os.kill(pid, signal.SIGWINCH)

    def reply(self, data):
        os.write(self.fd, data.encode())

    def pump(self):
        while True:
            try:
                r, _, _ = select.select([self.fd], [], [], 0.2)
            except InterruptedError:
                continue
            if not r:
                continue
            try:
                data = os.read(self.fd, 65536)
            except OSError:
                data = b""
            if not data:
                with self.lock:
                    self.log.flush()
                    self.exited = True
                return
            # OSC 10/11 colour queries: answer like a dark terminal so themes settle at once.
            for q, code in ((b"\x1b]10;?", 10), (b"\x1b]11;?", 11)):
                if q in data:
                    rgb = "ffff/ffff/ffff" if code == 10 else "0000/0000/0000"
                    os.write(self.fd, f"\x1b]{code};rgb:{rgb}\x1b\\".encode())
            with self.lock:
                self.log.write(data)
                self.log.flush()
                self.offset += len(data)
                self.last_output = time.monotonic()
                self.stream.feed(data)

    def text(self):
        with self.lock:
            return "\n".join(line.rstrip() for line in self.screen.display)

    def handle(self, req):
        op = req["op"]
        if op == "screen":
            return self.text()
        if op == "send":
            os.write(self.fd, req["data"].encode())
            return "ok"
        if op == "paste":
            os.write(self.fd, b"\x1b[200~" + req["data"].encode() + b"\x1b[201~")
            return "ok"
        if op == "wait":
            deadline = time.monotonic() + req.get("secs", 60)
            pat = re.compile(req["regex"], re.S)
            while time.monotonic() < deadline:
                if pat.search(self.text()):
                    return "matched"
                time.sleep(0.1)
            return "timeout\n" + self.text()
        if op == "quiet":
            ms, deadline = req.get("ms", 1500) / 1000, time.monotonic() + req.get("secs", 60)
            while time.monotonic() < deadline:
                with self.lock:
                    idle = time.monotonic() - self.last_output
                if idle >= ms:
                    return "quiet"
                time.sleep(0.05)
            return "timeout"
        if op == "mark":
            with self.lock:
                self.marks["marks"][req["label"]] = self.offset
                with open(self.marks_path, "w") as f:
                    json.dump(self.marks, f, indent=2, sort_keys=True)
                    f.write("\n")
            return f"{req['label']}={self.offset}"
        if op == "kill":
            try:
                os.kill(self.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            return "killed"
        return "unknown op"


def bind_short(srv, path):
    """Bind by a relative name: a scratch directory's absolute path can pass the 108-byte AF_UNIX limit."""
    cwd = os.getcwd()
    os.chdir(os.path.dirname(os.path.abspath(path)))
    try:
        srv.bind(os.path.basename(path))
    finally:
        os.chdir(cwd)


def serve(args):
    rec = Recorder(args)
    threading.Thread(target=rec.pump, daemon=True).start()
    args.ctl = os.path.abspath(args.ctl)
    if os.path.exists(args.ctl):
        os.unlink(args.ctl)
    srv = socket.socket(socket.AF_UNIX)
    bind_short(srv, args.ctl)
    srv.listen(4)
    srv.settimeout(0.5)
    while True:
        try:
            pid, _ = os.waitpid(rec.pid, os.WNOHANG)
        except ChildProcessError:
            pid = rec.pid
        if pid:
            time.sleep(0.5)
            break
        try:
            conn, _ = srv.accept()
        except socket.timeout:
            continue
        with conn:
            req = json.loads(conn.makefile().readline())
            conn.sendall(rec.handle(req).encode())
    os.unlink(args.ctl)


def ctl(args):
    req = {"op": args.op}
    rest = args.rest
    if args.op in ("send", "paste"):
        if rest[0].startswith("@"):  # @FILE: the text is the file's contents, verbatim
            with open(rest[0][1:]) as f:
                req["data"] = f.read().rstrip("\n")
        else:
            req["data"] = codecs.decode(rest[0], "unicode_escape")
    elif args.op == "wait":
        req["regex"] = rest[0]
        if len(rest) > 1:
            req["secs"] = float(rest[1])
    elif args.op == "quiet":
        if rest:
            req["ms"] = float(rest[0])
        if len(rest) > 1:
            req["secs"] = float(rest[1])
    elif args.op == "mark":
        req["label"] = rest[0]
    s = socket.socket(socket.AF_UNIX)
    os.chdir(os.path.dirname(os.path.abspath(args.sock)))
    s.connect(os.path.basename(args.sock))
    s.sendall((json.dumps(req) + "\n").encode())
    s.shutdown(socket.SHUT_WR)
    out = b""
    try:
        while chunk := s.recv(65536):
            out += chunk
    except ConnectionResetError:  # the child exited while answering (a quit key)
        out = b"exited"
    text = out.decode()
    print(text)
    sys.exit(1 if text.startswith("timeout") else 0)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="mode", required=True)
    s = sub.add_parser("serve")
    s.add_argument("--ctl", required=True)
    s.add_argument("--log", required=True)
    s.add_argument("--cols", type=int, required=True)
    s.add_argument("--rows", type=int, required=True)
    s.add_argument("--cwd", default=".")
    s.add_argument("--env", action="append", default=[])
    s.add_argument("cmd", nargs=argparse.REMAINDER)
    c = sub.add_parser("ctl")
    c.add_argument("sock")
    c.add_argument("op")
    c.add_argument("rest", nargs="*")
    args = p.parse_args()
    if args.mode == "serve":
        if args.cmd and args.cmd[0] == "--":
            args.cmd = args.cmd[1:]
        serve(args)
    else:
        ctl(args)


if __name__ == "__main__":
    main()
