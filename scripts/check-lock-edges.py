#!/usr/bin/env python3
"""Check that every dependency edge in Cargo.lock points at the highest locked version its requirement allows, and
repair the ones that don't.

Why: `cargo update -p CRATE` (Dependabot's way too) unlocks CRATE and its whole dependency subtree. A crate outside
that subtree whose requirement spans several versions (`windows-sys = ">=0.52, <0.62"`) and whose edge pointed into
it is then locked to the lowest other version of that dependency still in the lock, not to the one it had (cargo's
`lock()` in src/cargo/core/registry.rs: "query all known locked packages"). The lock stays valid under `--locked`, so
nothing notices, but the edge has moved: gpu-allocator on an older `windows` than wgpu-hal's does not build on
Windows. No manifest setting stops it while the lock holds two versions inside one such range.

A resolve that starts from the manifests (a new or changed dependency) gives each such edge the highest locked
version, so that is the state this check holds the lock to, and `--fix` restores it after an update.

Usage: scripts/check-lock-edges.py [--fix]   check Cargo.lock; with --fix, re-point the edges and let cargo rewrite it
       scripts/check-lock-edges.py --self-test
Needs cargo and Python 3.11+.
"""

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCK = ROOT / "Cargo.lock"
VERSION = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")
COMPARATOR = re.compile(r"^(=|>=|>|<=|<|~|\^)?\s*(\d+|[*xX])(?:\.(\d+|[*xX]))?(?:\.(\d+|[*xX]))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")
RELEASE = (1, ())


def pre_key(pre):
    if pre is None:
        return RELEASE
    return (0, tuple((0, int(part), "") if part.isdigit() else (1, 0, part) for part in pre.split(".")))


def parse_version(text):
    match = VERSION.match(text)
    if not match:
        raise ValueError(f"not a version: {text!r}")
    major, minor, patch, pre = match.groups()
    return (int(major), int(minor), int(patch), pre_key(pre))


def comparator_bounds(text):
    """One comparator as (bounds, the release it names a pre-release of or None); a bound is (op, version)."""
    match = COMPARATOR.match(text.strip())
    if not match:
        raise ValueError(f"not a version requirement: {text!r}")
    op, major, minor, patch, pre = match.groups()
    parts = []
    for part in (major, minor, patch):
        if part is None or part in "*xX":
            break
        parts.append(int(part))
    wildcard = any(part is not None and part in "*xX" for part in (major, minor, patch))
    if not parts:
        return [], None
    if wildcard:
        if op not in (None, "="):
            raise ValueError(f"wildcard with an operator: {text!r}")
        op = "="
    op = op or "^"
    low = tuple(parts + [0] * (3 - len(parts)))
    full = len(parts) == 3

    def bump(index):
        return tuple(low[:index]) + (low[index] + 1,) + (0,) * (2 - index) + (RELEASE,)

    floor = low + (pre_key(pre if full else None),)
    last = len(parts) - 1
    if op == "=":
        bounds = [("==", floor)] if full else [(">=", floor), ("<", bump(last))]
    elif op == ">":
        bounds = [(">", floor)] if full else [(">=", bump(last))]
    elif op == ">=":
        bounds = [(">=", floor)]
    elif op == "<":
        bounds = [("<", floor)]
    elif op == "<=":
        bounds = [("<=", floor)] if full else [("<", bump(last))]
    elif op == "~":
        bounds = [(">=", floor), ("<", bump(min(last, 1)))]
    else:
        significant = next((index for index in range(len(parts)) if parts[index] != 0), last)
        bounds = [(">=", floor), ("<", bump(significant))]
    return bounds, (low if full and pre is not None else None)


def matches(req, version):
    """Whether the version (text) satisfies the requirement, as the semver crate cargo uses decides it."""
    parsed = parse_version(version)
    pre_bases = []
    for comparator in req.split(","):
        bounds, pre_base = comparator_bounds(comparator)
        if pre_base is not None:
            pre_bases.append(pre_base)
        for op, bound in bounds:
            ok = {
                "==": parsed == bound,
                ">": parsed > bound,
                ">=": parsed >= bound,
                "<": parsed < bound,
                "<=": parsed <= bound,
            }[op]
            if not ok:
                return False
    return parsed[3] == RELEASE or parsed[:3] in pre_bases


def lock_edges(lock):
    """Each edge of the parsed lock as (package, dependency), both (name, version, source)."""
    packages = [(p["name"], p["version"], p.get("source")) for p in lock.get("package", [])]
    by_name = {}
    for package in packages:
        by_name.setdefault(package[0], []).append(package)
    for entry, package in zip(lock.get("package", []), packages):
        for dep in entry.get("dependencies", []):
            name, _, rest = dep.partition(" ")
            version, _, source = rest.partition(" ")
            found = [
                candidate
                for candidate in by_name.get(name, [])
                if (not version or candidate[1] == version) and (not source or f"({candidate[2]})" == source)
            ]
            if len(found) != 1:
                raise ValueError(f"{package[0]} {package[1]}: dependency {dep!r} names {len(found)} lock packages")
            yield package, found[0]


def drifted(lock, requirements):
    """The edges not at the highest locked version their requirement allows: (package, dependency, version wanted).

    `requirements` maps a package to {dependency name: [version requirements]}, from its manifest."""
    versions = {}
    for entry in lock.get("package", []):
        versions.setdefault((entry["name"], entry.get("source")), []).append(entry["version"])
    found = []
    for package, dep in lock_edges(lock):
        candidates = versions[(dep[0], dep[2])]
        if len(candidates) == 1:
            continue
        reqs = requirements.get(package, {}).get(dep[0], [])
        allowing = [req for req in reqs if matches(req, dep[1])]
        if not allowing:
            raise ValueError(f"{package[0]} {package[1]}: no requirement on {dep[0]} among {reqs} allows {dep[1]}")
        best = [
            max((v for v in candidates if matches(req, v)), key=parse_version)
            for req in allowing
        ]
        if dep[1] not in best:
            found.append((package, dep, max(best, key=parse_version)))
    return found


def repoint(text, drift):
    """The lock text with each drifted edge moved to the version it should have."""
    blocks = text.split("\n[[package]]\n")
    for package, dep, wanted in drift:
        head = f'name = "{package[0]}"\nversion = "{package[1]}"\n'
        old = re.compile(rf'^ "{re.escape(dep[0])} {re.escape(dep[1])}((?: \(.*\))?)",$', re.MULTILINE)
        for index, block in enumerate(blocks):
            if not block.startswith(head):
                continue
            if package[2] is not None and f'source = "{package[2]}"' not in block:
                continue
            blocks[index], count = old.subn(rf' "{dep[0]} {wanted}\1",', block)
            if count != 1:
                raise ValueError(f"{package[0]} {package[1]}: {count} lock lines for {dep[0]} {dep[1]}")
            break
        else:
            raise ValueError(f"no lock block for {package[0]} {package[1]}")
    return "\n[[package]]\n".join(blocks)


def manifest_requirements(locked):
    command = ["cargo", "metadata", "--format-version", "1"] + (["--locked"] if locked else [])
    metadata = json.loads(subprocess.run(command, cwd=ROOT, check=True, stdout=subprocess.PIPE).stdout)
    requirements = {}
    for package in metadata["packages"]:
        deps = requirements.setdefault((package["name"], package["version"], package["source"]), {})
        for dep in package["dependencies"]:
            deps.setdefault(dep["name"], []).append(dep["req"])
    return requirements


def describe(drift):
    return "\n".join(
        f"  {package[0]} {package[1]} -> {dep[0]} {dep[1]} (highest locked version it allows: {wanted})"
        for package, dep, wanted in drift
    )


def check(fix):
    for _ in range(5):
        drift = drifted(tomllib.loads(LOCK.read_text()), manifest_requirements(locked=not fix))
        if not drift:
            print("Cargo.lock: every edge is at the highest locked version its requirement allows")
            return 0
        if not fix:
            print(
                "Cargo.lock: a `cargo update` moved these edges to an older locked version"
                " (scripts/check-lock-edges.py --fix restores them; its header says why):",
                file=sys.stderr,
            )
            print(describe(drift), file=sys.stderr)
            return 1
        print("re-pointing:\n" + describe(drift))
        LOCK.write_text(repoint(LOCK.read_text(), drift))
    print("Cargo.lock: edges still move after 5 passes", file=sys.stderr)
    return 1


def self_test():
    cases = [
        ("^1.2.3", "1.9.0", True), ("^1.2.3", "2.0.0", False), ("^1.2.3", "1.2.2", False),
        ("^0.2.3", "0.2.9", True), ("^0.2.3", "0.3.0", False), ("^0.0.3", "0.0.3", True), ("^0.0.3", "0.0.4", False),
        ("^0.0", "0.0.9", True), ("^0.0", "0.1.0", False), ("^0", "0.9.1", True), ("^1", "1.0.0", True),
        ("0.61", "0.61.2", True), ("0.61", "0.62.0", False),
        ("~1.2.3", "1.2.9", True), ("~1.2.3", "1.3.0", False), ("~1.2", "1.2.0", True), ("~1", "1.9.9", True),
        ("~1", "2.0.0", False),
        ("=1.2.3", "1.2.3", True), ("=1.2.3", "1.2.4", False), ("=1.2", "1.2.9", True), ("=1", "1.9.0", True),
        (">=0.52, <0.62", "0.61.2", True), (">=0.52, <0.62", "0.62.0", False), (">=0.52, <0.62", "0.48.0", False),
        (">=0.53, <=0.62", "0.62.2", True), (">=0.53, <=0.62", "0.63.0", False), (">=0.54, <=0.57", "0.57.0", True),
        (">0.5", "0.5.9", False), (">0.5", "0.6.0", True), (">1.2.3", "1.2.4", True), ("<1.2.3", "1.2.2", True),
        ("<=1.2.3", "1.2.3", True), ("<1", "0.9.9", True), ("<1", "1.0.0", False), (">=4, <7", "6.0.0", True),
        (">=4, <7", "7.0.0", False),
        ("*", "3.4.5", True), ("1.*", "1.4.5", True), ("1.*", "2.0.0", False), ("1.2.*", "1.2.5", True),
        ("1.2.x", "1.3.0", False),
        ("^1.2.3", "1.3.0-beta.1", False), ("^1.3.0-alpha.1", "1.3.0-beta.1", True),
        ("^1.3.0-beta.2", "1.3.0-beta.1", False), ("^1.3.0-beta.2", "1.3.0-beta.10", True),
        ("^1.3.0-alpha.1", "1.3.0", True), ("=1.0.0-rc.1", "1.0.0-rc.1", True), ("*", "1.0.0-rc.1", False),
        (">=0.9, <2", "1.1.8+spec-1.1.0", True),
    ]
    failed = [(req, version) for req, version, expected in cases if matches(req, version) != expected]
    assert not failed, f"wrong answer for {failed}"

    registry = "registry+https://github.com/rust-lang/crates.io-index"
    text = "\n".join([
        "version = 4",
        "",
        "[[package]]", 'name = "app"', 'version = "0.1.0"', "dependencies = [", ' "ranged",', ' "strict",', ' "sys 0.48.0",', "]",
        "",
        "[[package]]", 'name = "ranged"', 'version = "1.0.0"', f'source = "{registry}"', "dependencies = [", ' "sys 0.60.2",', "]",
        "",
        "[[package]]", 'name = "strict"', 'version = "1.0.0"', f'source = "{registry}"', "dependencies = [", ' "sys 0.60.2",', "]",
        "",
        "[[package]]", 'name = "sys"', 'version = "0.48.0"', f'source = "{registry}"',
        "",
        "[[package]]", 'name = "sys"', 'version = "0.60.2"', f'source = "{registry}"',
        "",
        "[[package]]", 'name = "sys"', 'version = "0.61.2"', f'source = "{registry}"',
        "",
    ])
    requirements = {
        ("app", "0.1.0", None): {"ranged": ["^1"], "strict": ["^1"], "sys": ["^0.48", "^0.61"]},
        ("ranged", "1.0.0", registry): {"sys": [">=0.52, <0.62"]},
        ("strict", "1.0.0", registry): {"sys": ["^0.60"]},
    }
    drift = drifted(tomllib.loads(text), requirements)
    ranged = ("ranged", "1.0.0", registry)
    assert drift == [(ranged, ("sys", "0.60.2", registry), "0.61.2")], drift
    fixed = repoint(text, drift)
    assert fixed == text.replace(' "sys 0.60.2",\n]\n\n[[package]]\nname = "strict"', ' "sys 0.61.2",\n]\n\n[[package]]\nname = "strict"')
    assert drifted(tomllib.loads(fixed), requirements) == []
    try:
        drifted(tomllib.loads(text), {**requirements, ranged: {"sys": ["^0.61"]}})
    except ValueError:
        pass
    else:
        raise AssertionError("an edge no requirement allows went unreported")
    print("check-lock-edges: self-test passed")
    return 0


def main(argv):
    if argv == ["--self-test"]:
        return self_test()
    if argv not in ([], ["--fix"]):
        print(__doc__.strip().split("\n\n")[-1], file=sys.stderr)
        return 2
    return check(fix=bool(argv))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
