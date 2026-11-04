#!/usr/bin/env python3
"""Strip a recording of personal paths, ids and bulk before it becomes a fixture.

    sanitize.py jsonl IN OUT --map OLD=NEW ... [--user NAME]   # a transcript or rollout
    sanitize.py json IN OUT --map OLD=NEW ... [--user NAME]    # one hook payload
    sanitize.py env IN OUT --map OLD=NEW ... [--user NAME]     # a hook's KEY=VALUE environment
    sanitize.py bin IN OUT --map OLD=NEW ... [--user NAME]     # a PTY capture: same-length swaps only

`--map` pairs apply in order to every string (put the longest paths first; a recording carries the
recorder's time zone too, so map its name to a neutral one of the same length). `--user NAME` then
rewrites the word NAME to `user`. A capture keeps its byte offsets (its marks point into it), so
`bin` takes only same-length pairs and refuses others.

Transcript lines keep their type and the fields a parser reads; bulk the chat never shows (system
prompts, tool schemas, API request logs, encrypted reasoning, signatures, account ids) is replaced
by a short placeholder.
"""

import argparse
import json
import re
import sys

ZERO_UUID = "00000000-0000-0000-0000-000000000000"
REDACTED = "<redacted>"
# Claude Code bookkeeping lines whose bodies are request logs: keep only these keys.
CLAUDE_STUB_TYPES = {"api-request-shape", "api-request-blob", "api-request"}
CLAUDE_STUB_KEEP = ("type", "sessionId", "id", "timestamp", "querySource")
# Keys holding prompts, schemas or account data, anywhere in a line.
REDACT_KEYS = {"base_instructions", "developer_instructions", "instructions", "encrypted_content", "signature"}
ID_KEYS = {"accountUuid", "organizationUuid", "creator_account_id", "account_id", "organization_id"}
USER_ID_KEYS = {"creator_user_id", "user_id", "chatgpt_user_id"}


class Rewriter:
    def __init__(self, pairs, user):
        self.pairs = pairs
        self.user = re.compile(rf"\b{re.escape(user)}\b") if user else None

    def text(self, s):
        for old, new in self.pairs:
            s = s.replace(old, new)
        if self.user:
            s = self.user.sub("user", s)
        return s

    def value(self, v, key=None):
        if key in REDACT_KEYS and isinstance(v, (str, dict, list)) and v:
            return REDACTED
        if key in ID_KEYS and isinstance(v, str):
            return ZERO_UUID
        if key in USER_ID_KEYS and isinstance(v, str):
            return "user-" + "0" * 24
        if isinstance(v, str):
            return elide(self.text(v))
        if isinstance(v, list):
            return [self.value(x) for x in v]
        if isinstance(v, dict):
            return {self.text(k): self.value(x, k) for k, x in v.items()}
        return v


def elide(s, keep=1024, limit=8192):
    """A string over `limit` keeps its head and is padded back to its length with a marker line, so a
    fixture still holds an oversized output without carrying whatever the recording dumped."""
    if len(s) <= limit:
        return s
    pad = "… (elided while sanitizing)\n"
    body = s[:keep] + "\n"
    return body + (pad * ((len(s) - len(body)) // len(pad) + 1))[: len(s) - len(body)]


def stub_claude(d):
    t = d.get("type")
    if t in CLAUDE_STUB_TYPES:
        out = {k: d[k] for k in CLAUDE_STUB_KEEP if k in d}
        out["redacted"] = True
        return out
    if t == "attachment" and isinstance(d.get("attachment"), dict):
        # Attachments are context Claude Code injects (environment, skill lists, prompt snapshots, the org):
        # keep the kind, drop the body and its rendering.
        d = dict(d)
        d["attachment"] = {"type": d["attachment"].get("type"), "redacted": True}
        d.pop("rendered", None)
    return d


def stub_codex(d):
    p = d.get("payload")
    if not isinstance(p, dict):
        return d
    t, pt = d.get("type"), p.get("type")
    p = dict(p)
    if t == "response_item" and pt == "additional_tools":
        p["tools"] = []
    elif t == "response_item" and pt == "message" and p.get("role") == "developer":
        p["content"] = [{"type": "input_text", "text": REDACTED}]
    elif t == "world_state":
        p["state"] = {}
    elif t == "compacted":
        p["replacement_history"] = []
        p.pop("retained_context", None)
    d = dict(d)
    d["payload"] = p
    return d


def jsonl(args, rw):
    out = []
    with open(args.inp, encoding="utf-8", errors="surrogateescape") as f:
        for line in f:
            body = line.rstrip("\n")
            try:
                d = json.loads(body)
            except ValueError:
                out.append(rw.text(body))
                continue
            d = stub_codex(d) if "payload" in d else stub_claude(d)
            out.append(json.dumps(rw.value(d), ensure_ascii=False, separators=(",", ":")))
    with open(args.out, "w", encoding="utf-8", errors="surrogateescape") as f:
        f.write("\n".join(out) + "\n")


def one_json(args, rw):
    with open(args.inp, encoding="utf-8") as f:
        d = json.load(f)
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(rw.value(d), f, ensure_ascii=False, separators=(",", ":"))
        f.write("\n")


def env(args, rw):
    keep = []
    with open(args.inp, encoding="utf-8") as f:
        for line in f:
            k, _, v = line.rstrip("\n").partition("=")
            if k in ("CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_PID", "CLAUDE_ENV_FILE"):
                continue
            keep.append(f"{k}={rw.text(v)}")
    with open(args.out, "w", encoding="utf-8") as f:
        f.write("\n".join(keep) + ("\n" if keep else ""))


def binary(args, rw):
    with open(args.inp, "rb") as f:
        data = f.read()
    pairs = [(o.encode(), n.encode()) for o, n in rw.pairs]
    for old, new in pairs:
        if len(old) != len(new):
            sys.exit(f"bin: {old!r} -> {new!r} changes the length")
        data = data.replace(old, new)
    if rw.user:
        user = rw.user.pattern.replace(r"\b", "").replace("\\", "").encode()
        if len(user) != len(b"user"):
            sys.exit("bin: --user must be four bytes long to keep offsets")
        data = re.sub(rb"\b" + re.escape(user) + rb"\b", b"user", data)
    with open(args.out, "wb") as f:
        f.write(data)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("mode", choices=["jsonl", "json", "env", "bin"])
    p.add_argument("inp")
    p.add_argument("out")
    p.add_argument("--map", action="append", default=[])
    p.add_argument("--user")
    args = p.parse_args()
    pairs = [tuple(m.split("=", 1)) for m in args.map]
    rw = Rewriter(pairs, args.user)
    {"jsonl": jsonl, "json": one_json, "env": env, "bin": binary}[args.mode](args, rw)


if __name__ == "__main__":
    main()
