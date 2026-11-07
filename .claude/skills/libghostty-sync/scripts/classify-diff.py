#!/usr/bin/env python3
"""Sort a Ghostty diff OLD..NEW into the areas ngterm mirrors, and find the ngterm code that cites each change.

    classify-diff.py [--ghostty DIR] OLD_REV NEW_REV [--json]

Reads the clone with git only (no checkout needed) and the ngterm checkout of the current directory. Prints a
markdown report:

- every first-parent commit (one per merged pull request on ghostty main) with the areas its files fall in: the unit
  a plan task grows from;
- per area, its files (+added -deleted), the Zig tests added or renamed (`test "..."`: Ghostty keeps its tests next to
  the code, so they hold the cases a Rust port asserts), and the ngterm files that cite each file;
- the C API delta of include/ghostty/: functions, types and constants added, removed or changed, each with the
  ngterm files that use it (the generated bindings excluded);
- the review checklist: every ngterm file citing a changed Ghostty file, with what it cites;
- the uncited changes in the areas ngterm mirrors (renderer, font, sprite, input, termio, surface, config, os):
  candidates whose ngterm counterpart cites nothing yet, or which no ngterm code mirrors.

A citation is a token in an ngterm source naming a Ghostty file: its full path, a tail of it (`draw/box.zig`,
`renderer/generic.zig`), or its bare name. A bare name that several Ghostty files share (`main.zig`, `Config.zig`) is
reported as `weak`. Unknown paths land in `review`, never in `no-port`.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path

# (area, regex over the path). First match wins; order matters.
RULES: list[tuple[str, str]] = [
    # libghostty-vt's C API: the headers, their Zig implementation, the library root
    ("vt-api", r"^include/ghostty/vt(\.h$|/)"),
    ("vt-api", r"^src/terminal/c/"),
    ("vt-api", r"^src/(lib_vt|main_c)\.zig$"),
    ("vt-api", r"^src/lib/"),
    ("no-port", r"^include/ghostty\.h$"),  # the full libghostty's embedding API, which ngterm does not link
    ("sprite-goldens", r"^src/font/sprite/testdata/"),
    ("sprite", r"^src/font/sprite/"),
    ("font", r"^src/font/"),
    ("renderer", r"^src/renderer/"),
    # the VT itself: arrives with the bump, ngterm never re-implements it
    ("vt-core", r"^src/terminal/"),
    ("vt-core", r"^src/(unicode|simd|datastruct)/"),
    ("vt-core", r"^src/(quirks|quirks_memset|tripwire|crc32c|fastprint)\.zig$"),
    ("termio", r"^src/termio/"),
    ("termio", r"^src/(pty|Command)\.zig$"),
    ("surface", r"^src/(Surface|App)\.zig$"),
    ("surface", r"^src/apprt/(embedded|surface|action|structs|runtime|ipc)\.zig$"),
    ("input", r"^src/input(/|\.zig$)"),
    ("config", r"^src/config(/|\.zig$)"),
    ("resources", r"^src/(shell-integration|terminfo)/"),
    ("os", r"^src/os/"),
    ("cli", r"^src/cli/"),
    ("build", r"^build\.zig(\.zon)?$"),
    ("build", r"^src/build/"),
    ("build", r"^src/build_config\.zig$"),
    ("build", r"^pkg/(wuffs|simdutf|highway|utfcpp|translate-c|android-ndk|apple-sdk)/"),
    # Ghostty's own app shells: the reference for what an embedder does around the terminal (selection, IME,
    # scrolling, clipboard, links, focus) on macOS and Linux; their window chrome is agterm's domain, not Ghostty's
    ("apprt-macos", r"^macos/"),
    ("apprt-gtk", r"^src/apprt/gtk/"),
    ("apprt-gtk", r"^src/apprt/"),
    ("no-port", r"^src/(benchmark|synthetic|inspector|extra|bench|helpgen|main_ghostty|main_wasm|main_bench|global)"),
    ("no-port", r"^(\.github|nix|flatpak|snap|po|dist|example|deps|test|images|vendor|website)/"),
    ("no-port", r"^pkg/"),
    ("no-port", r"^[^/]+\.(md|nix|lock|toml|xml|yml|yaml|txt)$"),
    ("no-port", r"^(Doxyfile|CMakeLists\.txt|CODEOWNERS|VOUCHED.*|Makefile|\.[^/]+)$"),
]
COMPILED = [(area, re.compile(rx)) for area, rx in RULES]

AREA_TITLES = {
    "vt-api": "libghostty-vt C API (`ngterm-ghostty-vt-sys` bindings, `ngterm-ghostty-vt` wrapper)",
    "vt-core": "The VT itself (arrives with the bump: ngterm-term, the session server's snapshot, every wrapper test)",
    "renderer": "Renderer (`ngterm-gpui` terminal element, colors, cursor, kitty images, background; the mobile renderers)",
    "font": "Font: metrics, shaping, discovery, fallback (`ngterm-gpui` terminal font and metrics)",
    "sprite": "Sprite font: box drawing, blocks, powerline, braille, legacy computing (`ngterm-gpui` terminal/sprite)",
    "sprite-goldens": "Sprite test images (`crates/ngterm-gpui/tests/golden/`)",
    "termio": "Termio: what the embedder does between PTY and VT (`ngterm-term`, `ngterm-pty`, `ngterm-session`)",
    "surface": "Surface: mouse, selection, links, scrolling, binding actions (`ngterm-term`, `ngterm-gpui` terminal)",
    "input": "Input: key encoding tables, keycodes, bindings, mouse (`ngterm-gpui` keyboard/keycodes/mouse, `ngterm-core` keymap)",
    "config": "Config keys and their semantics (`ngterm-core` ghostty_config; gen-ghostty-config-keys.sh)",
    "resources": "Resources: shell integration, terminfo (vendored by build-ghostty-vt.sh; `ngterm-term` launch)",
    "os": "OS helpers: paths, xdg, hostname, open, locale (`ngterm-core`, `ngterm-term`)",
    "cli": "Ghostty's CLI: +list-themes, +ssh terminfo and the like (`ngterm-ctl` counterparts)",
    "build": "Build: Zig version, package deps, fonts and themes, lib-vt build (`build/pins.env`, `ngterm-ghostty-vt-sys`)",
    "apprt-macos": "Ghostty's macOS app (reference for the terminal surface's behavior on macOS)",
    "apprt-gtk": "Ghostty's GTK app and apprt (reference for the terminal surface's behavior on Linux)",
    "review": "Review: paths the table does not know",
    "no-port": "No port: CI, packaging, i18n, docs, examples, tests of other builds, other platforms' deps",
}
AREA_ORDER = list(AREA_TITLES)
# areas whose ngterm counterpart is code ngterm wrote to mirror Ghostty's: an uncited change there is a question
MIRROR_AREAS = {"renderer", "font", "sprite", "termio", "surface", "input", "config", "os", "cli", "resources"}

TEST_RE = re.compile(r'^[+-]\s*test\s+"([^"]+)"')
CITE_RE = re.compile(r"[\w./+-]*[\w+-]\.(?:zig|h|glsl|metal|c|cpp|swift|frag|vert|py|sh)\b")
CITE_GLOBS = ("crates", "scripts", "build", "mobile", "conformance/docs", "resources", "README.md", "AGENTS.md")
BINDINGS = "crates/ngterm-ghostty-vt-sys/src/bindings.rs"
TEXT_SUFFIXES = {".rs", ".md", ".sh", ".py", ".toml", ".kt", ".kts", ".swift", ".env", ".txt", ".h", ".m", ".yml"}


def git(clone: str, *args: str) -> str:
    return subprocess.run(["git", "-C", clone, *args], check=True, capture_output=True, text=True).stdout


def area_of(path: str) -> str:
    for area, rx in COMPILED:
        if rx.search(path):
            return area
    return "review"


@dataclass
class FileChange:
    path: str
    added: int
    deleted: int
    area: str
    tests: list[str] = field(default_factory=list)
    cited_by: list[tuple[str, str]] = field(default_factory=list)  # (ngterm file, strength)


@dataclass
class Commit:
    sha: str
    subject: str
    pr: str | None
    files: list[str]
    areas: list[str]


def numstat(clone: str, old: str, new: str) -> dict[str, tuple[int, int]]:
    out: dict[str, tuple[int, int]] = {}
    for line in git(clone, "diff", "--numstat", "-M", old, new).splitlines():
        added, deleted, path = line.split("\t", 2)
        if " => " in path:  # a rename: keep the new name
            path = re.sub(r"\{[^}]* => ([^}]*)\}", r"\1", path) if "{" in path else path.split(" => ")[1]
            path = path.replace("//", "/")
        out[path] = (int(added) if added != "-" else 0, int(deleted) if deleted != "-" else 0)
    return out


def tests_by_file(clone: str, old: str, new: str) -> dict[str, list[str]]:
    """Zig test names added in each .zig file (renamed tests show up as added)."""
    out: dict[str, list[str]] = defaultdict(list)
    current = None
    removed: dict[str, set[str]] = defaultdict(set)
    old_path = None
    for line in git(clone, "diff", "-U0", old, new, "--", "*.zig").splitlines():
        if line.startswith("--- "):
            old_path = line[6:] if line.startswith("--- a/") else None
            continue
        if line.startswith("+++ "):
            # a deleted file's hunks belong to its old path, never to the file diffed before it
            current = line[6:] if line.startswith("+++ b/") else old_path
            continue
        if current is None:
            continue
        m = TEST_RE.match(line)
        if m:
            if line.startswith("+"):
                out[current].append(m.group(1))
            else:
                removed[current].add(m.group(1))
    return {path: [t for t in names if t not in removed[path]] for path, names in out.items()}


def commits(clone: str, old: str, new: str) -> list[Commit]:
    result = []
    log = git(clone, "log", "--first-parent", "--reverse", "--format=%H%x09%s", f"{old}..{new}")
    for line in log.splitlines():
        sha, subject = line.split("\t", 1)
        files = git(clone, "diff", "--name-only", f"{sha}^1", sha).splitlines()
        pr = m.group(1) if (m := re.search(r"\(#(\d+)\)\s*$", subject) or re.search(r"pull request #(\d+)", subject)) else None
        areas = sorted({area_of(f) for f in files}, key=AREA_ORDER.index)
        result.append(Commit(sha[:9], subject, pr, files, areas))
    return result


def ngterm_files(root: Path) -> list[str]:
    out = subprocess.run(["git", "-C", str(root), "ls-files", "--", *CITE_GLOBS], check=True, capture_output=True,
                         text=True).stdout.splitlines()
    return [p for p in out if p != BINDINGS and (Path(p).suffix in TEXT_SUFFIXES or Path(p).name == "pins.env")]


def citation_index(root: Path, files: list[str]) -> dict[str, set[str]]:
    """Each cited token (a path or bare name) -> the ngterm files that mention it."""
    index: dict[str, set[str]] = defaultdict(set)
    for rel in files:
        try:
            text = (root / rel).read_text(errors="replace")
        except (OSError, UnicodeDecodeError):
            continue
        for token in set(CITE_RE.findall(text)):
            index[token.lstrip("./")].add(rel)
    return index


def cite(path: str, index: dict[str, set[str]], basename_count: dict[str, int]) -> list[tuple[str, str]]:
    found: dict[str, str] = {}
    parts = path.split("/")
    for i in range(len(parts)):
        tail = "/".join(parts[i:])
        strength = "path" if i < len(parts) - 1 else ("weak" if basename_count.get(tail, 0) > 1 else "name")
        for rel in index.get(tail, ()):
            found.setdefault(rel, strength)
    return sorted(found.items())


# ---- the C API delta -------------------------------------------------------------------------------------------

# A function declaration: everything since the previous statement (export macro, return type) plus the name and the
# parameters, so a changed return type is a changed declaration.
FUNC_RE = re.compile(r"([^;{}]*?)\b(ghostty_\w+)\s*\(([^;{}]*)\)\s*;")
ALIAS_RE = re.compile(r"typedef\s+([^;{}()]+?)\s+(\w+)\s*;")
FNPTR_RE = re.compile(r"typedef\s+([^;{}]+?)\(\s*\*\s*(\w+)\s*\)\s*\(([^;]*)\)\s*;")
# a #define and its value on the same line only: a bodiless one (a header guard) has an empty value
DEFINE_RE = re.compile(r"^[ \t]*#[ \t]*define[ \t]+(GHOSTTY_\w+)\b[ \t]*([^\n]*)$", re.M)
# the start of a body: `typedef struct|enum|union [Tag] {` or `struct|enum|union Tag {`
BODY_RE = re.compile(r"(typedef\s+)?\b(struct|enum|union)\s+(\w+\s*)?\{")
EXPORT_RE = re.compile(r"\bGHOSTTY_API\b")


def norm(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    text = re.sub(r"//[^\n]*", " ", text)
    return re.sub(r"\s+", " ", text).strip()


def bodies(text: str) -> tuple[dict[str, tuple[str, str]], str]:
    """Every struct, enum and union body (nested braces matched), by its typedef name or its tag, as
    (keyword, body); and the text with those bodies blanked, so the other patterns never read inside them."""
    found: dict[str, tuple[str, str]] = {}
    out, pos = [], 0
    while m := BODY_RE.search(text, pos):
        depth, i = 1, m.end()
        while i < len(text) and depth:
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        body = text[m.end():i - 1].strip()
        tail = re.match(r"\s*(\w*)\s*;", text[i:])
        name = (tail.group(1) if m.group(1) and tail and tail.group(1) else (m.group(3) or "").strip())
        if name:
            found[name] = (m.group(2), body)
        end = i + (tail.end() if tail else 0)
        out.append(text[pos:m.start()] + " ; ")
        pos = end
    out.append(text[pos:])
    return found, "".join(out)


def enum_members(body: str) -> dict[str, str]:
    """An enum body's members with their values, implicit ones counted on from the last literal."""
    members: dict[str, str] = {}
    last: int | None = -1
    for item in (x.strip() for x in body.split(",")):
        if not (m := re.match(r"(\w+)\s*(?:=\s*(.+))?$", item)):
            continue
        expr = m.group(2)
        if expr is None:
            last = last + 1 if last is not None else None
            members[m.group(1)] = str(last) if last is not None else "?"
            continue
        try:
            last = int(expr, 0)
            members[m.group(1)] = str(last)
        except ValueError:
            last = None
            members[m.group(1)] = expr.strip()
    return members


def declarations(clone: str, rev: str) -> tuple[dict[str, tuple[str, str]], dict[str, dict[str, str]]]:
    """name -> (kind, normalized declaration) over every header under include/ghostty/vt*, and each enum's
    members (name -> value), whose constants are what bindgen's bindings and the wrapper use."""
    names = [p for p in git(clone, "ls-tree", "-r", "--name-only", rev, "--", "include/ghostty").splitlines()
             if re.match(r"include/ghostty/vt(\.h$|/)", p)]
    decls: dict[str, tuple[str, str]] = {}
    enums: dict[str, dict[str, str]] = {}
    for p in names:
        raw = git(clone, "show", f"{rev}:{p}")
        raw = re.sub(r"/\*.*?\*/", " ", raw, flags=re.S)
        for m in DEFINE_RE.finditer(raw):
            decls[m.group(1)] = ("const", norm(m.group(2)))
        text = norm(re.sub(r"^\s*#.*$", "", raw, flags=re.M))
        found, text = bodies(text)
        for name, (keyword, body) in found.items():
            decls[name] = ("type", f"{keyword} {{{body}}}")
            if keyword == "enum":
                enums[name] = enum_members(body)
        for m in FNPTR_RE.finditer(text):
            decls[m.group(2)] = ("callback", f"{m.group(1).strip()}({m.group(3).strip()})")
        for m in ALIAS_RE.finditer(text):
            decls.setdefault(m.group(2), ("alias", m.group(1).strip()))
        for m in FUNC_RE.finditer(text):
            ret = norm(EXPORT_RE.sub(" ", m.group(1)))
            decls[m.group(2)] = ("function", f"{ret} ({m.group(3).strip()})")
    keep = {k: v for k, v in decls.items() if k.lower().startswith("ghostty")}
    return keep, {k: v for k, v in enums.items() if k in keep}


def member_changes(old: dict[str, str], new: dict[str, str]) -> list[str]:
    out = []
    for name in sorted(set(old) | set(new)):
        if name not in new:
            out.append(f"{name} removed")
        elif name not in old:
            out.append(f"{name} added")
        elif old[name] != new[name]:
            out.append(f"{name} {old[name]} -> {new[name]}")
    return out


def api_delta(clone: str, old: str, new: str, root: Path, files: list[str]) -> list[dict]:
    (before, enums_before), (after, enums_after) = declarations(clone, old), declarations(clone, new)
    changed = []
    for name in sorted(set(before) | set(after)):
        if name in before and name in after and before[name] == after[name]:
            continue
        kind = "added" if name not in before else "removed" if name not in after else "changed"
        entry = {"name": name, "change": kind, "kind": (after.get(name) or before[name])[0], "used_by": [],
                 "members": member_changes(enums_before.get(name, {}), enums_after.get(name, {}))}
        if kind == "changed" and entry["kind"] == "function":
            entry["detail"] = f"{before[name][1]} -> {after[name][1]}"
        changed.append(entry)
    if not changed:
        return changed
    # a changed enum is used when ngterm names the type or any of its members, old or new
    owner: dict[str, dict] = {}
    for c in changed:
        owner[c["name"]] = c
        for member in set(enums_before.get(c["name"], {})) | set(enums_after.get(c["name"], {})):
            owner.setdefault(member, c)
    word = re.compile(r"\b(" + "|".join(map(re.escape, sorted(owner, key=len, reverse=True))) + r")\b")
    for rel in files:
        if not rel.endswith((".rs", ".kt", ".swift", ".h", ".m", ".sh", ".py")):
            continue
        try:
            text = (root / rel).read_text(errors="replace")
        except OSError:
            continue
        for c in {id(owner[n]): owner[n] for n in word.findall(text)}.values():
            if rel not in c["used_by"]:
                c["used_by"].append(rel)
    return changed


# ---- the report ------------------------------------------------------------------------------------------------

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ghostty", help="the Ghostty clone (default: <repo>/.upstream/ghostty)")
    ap.add_argument("old")
    ap.add_argument("new")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True,
                               text=True).stdout.strip())
    clone = args.ghostty or str(root / ".upstream/ghostty")
    old, new = (git(clone, "rev-parse", f"{r}^{{commit}}").strip() for r in (args.old, args.new))

    tree = git(clone, "ls-tree", "-r", "--name-only", new).splitlines()
    basename_count: dict[str, int] = defaultdict(int)
    for p in tree:
        basename_count[p.rsplit("/", 1)[-1]] += 1

    files = ngterm_files(root)
    index = citation_index(root, files)
    tests = tests_by_file(clone, old, new)
    changes = []
    for path, (added, deleted) in sorted(numstat(clone, old, new).items()):
        changes.append(FileChange(path, added, deleted, area_of(path), tests.get(path, []),
                                  cite(path, index, basename_count)))
    log = commits(clone, old, new)
    api = api_delta(clone, old, new, root, files)

    if args.json:
        json.dump({"old": old, "new": new, "commits": [c.__dict__ for c in log],
                   "files": [c.__dict__ for c in changes], "api": api}, sys.stdout, indent=1)
        return 0

    by_area: dict[str, list[FileChange]] = defaultdict(list)
    for c in changes:
        by_area[c.area].append(c)

    out = [f"# Ghostty {old[:9]}..{new[:9]}", "",
           f"{len(log)} first-parent commits, {len(changes)} files.", "", "## Commits (one per merged PR)", ""]
    for c in log:
        pr = f" #{c.pr}" if c.pr else ""
        out.append(f"- `{c.sha}`{pr} {c.subject} — {', '.join(c.areas)} ({len(c.files)} files)")

    out += ["", "## Areas", ""]
    for area in AREA_ORDER:
        if area not in by_area:
            continue
        group = by_area[area]
        out += [f"### {area}: {AREA_TITLES[area]}", ""]
        if area == "no-port":
            dirs: dict[str, int] = defaultdict(int)
            for c in group:
                dirs[c.path.split("/", 1)[0] if "/" in c.path else c.path] += 1
            out.append(", ".join(f"`{d}` ({n})" for d, n in sorted(dirs.items())))
            out.append("")
            continue
        for c in group:
            cites = "; cited by " + ", ".join(f"`{f}`" + (f" ({s})" if s != "path" else "") for f, s in c.cited_by) \
                if c.cited_by else ""
            out.append(f"- `{c.path}` +{c.added} -{c.deleted}{cites}")
            for t in c.tests:
                out.append(f"  - test \"{t}\"")
        out.append("")

    out += ["## C API delta (include/ghostty/vt*)", ""]
    used_api = [a for a in api if a["used_by"]]
    if not api:
        out.append("API_BREAK=none: no declaration changed.")
    elif not used_api:
        out.append("API_BREAK=none: no changed declaration is used by ngterm.")
    else:
        out.append(f"API_BREAK=used: {len(used_api)} changed declarations are used by ngterm.")
    out.append("")
    for a in used_api:
        out.append(f"- {a['change']} {a['kind']} `{a['name']}`: " + ", ".join(f"`{u}`" for u in sorted(a["used_by"])))
        if a.get("detail"):
            out.append(f"  - `{a['detail']}`")
        if a["members"]:
            out.append("  - members: " + "; ".join(a["members"]))
    for change in ("added", "changed", "removed"):
        names = [f"`{a['name']}`" for a in api if not a["used_by"] and a["change"] == change]
        if names:
            out.append(f"- {change}, unused by ngterm ({len(names)}): " + ", ".join(names))

    checklist: dict[str, list[str]] = defaultdict(list)
    for c in changes:
        if c.area == "no-port":
            continue
        for rel, strength in c.cited_by:
            checklist[rel].append(f"`{c.path}`" + (" (weak)" if strength == "weak" else ""))
    out += ["", "## Review checklist: ngterm files citing a changed Ghostty file", "",
            "`(weak)`: cited by a bare name several Ghostty files share; confirm which one it means.", ""]
    for rel in sorted(checklist):
        out.append(f"- `{rel}`: " + ", ".join(sorted(checklist[rel])))

    uncited = [c for c in changes if c.area in MIRROR_AREAS and not c.cited_by]
    out += ["", "## Uncited changes in mirrored areas", "",
            "No ngterm file cites these: either ngterm has no counterpart (decide whether it needs one) or its "
            "counterpart cites Ghostty by function name only (find it).", ""]
    for c in uncited:
        out.append(f"- [{c.area}] `{c.path}` +{c.added} -{c.deleted}")
    print("\n".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
