#!/usr/bin/env python3
"""Compute the LIBGHOSTTY_VT_* and FONTS_* pins of build/pins.env for a new Ghostty revision.

    bump-pins.py [--ghostty DIR] [--apply] NEW_REV

Reads NEW_REV's build.zig.zon from the clone (default <repo>/.upstream/ghostty) and fetches what a pin hashes:

- LIBGHOSTTY_VT_REV and LIBGHOSTTY_VT_SHA256: the commit and the sha256 of its source archive (the archive
  scripts/gen-ghostty-config-keys.sh fetches);
- LIBGHOSTTY_VT_ZIG_VERSION: the revision's `minimum_zig_version`; when it moves, the six
  LIBGHOSTTY_VT_ZIG_SHA256_<OS>_<ARCH> sums from ziglang.org's download/index.json;
- LIBGHOSTTY_VT_THEMES_URL, _SHA256 and LIBGHOSTTY_VT_THEME_COUNT: the `iterm2_themes` package the revision
  declares, its archive's sha256 and the theme files it holds; LIBGHOSTTY_VT_KNOWN_THEMES must all be among them;
- FONTS_JETBRAINS_MONO_* and FONTS_NERD_SYMBOLS_*: the `jetbrains_mono` and `nerd_fonts_symbols_only` packages.

Prints one `NAME: old -> new` line per moved pin and `# ` notes for the hand edits a move needs (the pins.env
comments, scripts/fetch-fonts.sh's layout), then `PINS=moved NAME ...` or `PINS=unchanged`. With --apply it rewrites
the value of each moved pin in build/pins.env; the comments around them stay for the caller to update.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

ZIG_TARGETS = {  # pins.env suffix -> ziglang.org index key
    "LINUX_X86_64": "x86_64-linux", "LINUX_AARCH64": "aarch64-linux",
    "MACOS_X86_64": "x86_64-macos", "MACOS_AARCH64": "aarch64-macos",
    "WINDOWS_X86_64": "x86_64-windows", "WINDOWS_AARCH64": "aarch64-windows",
}
FONT_DEPS = {"jetbrains_mono": "FONTS_JETBRAINS_MONO", "nerd_fonts_symbols_only": "FONTS_NERD_SYMBOLS"}


def fetch(url: str) -> bytes:
    last: Exception | None = None
    for _ in range(3):
        try:
            with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "ngterm-libghostty-sync"}),
                                        timeout=120) as resp:
                return resp.read()
        except OSError as err:
            last = err
    raise SystemExit(f"bump-pins: cannot fetch {url}: {last}")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_pins(path: Path) -> dict[str, str]:
    pins = {}
    for line in path.read_text().splitlines():
        if m := re.match(r'^([A-Z0-9_]+)=(?:"([^"]*)"|(\S+))', line):
            pins[m.group(1)] = m.group(2) if m.group(2) is not None else m.group(3)
    return pins


def zon_dep_url(zon: str, name: str) -> str | None:
    m = re.search(r"\." + re.escape(name) + r"\s*=\s*\.\{(.*?)\}", zon, re.S)
    if not m:
        return None
    url = re.search(r'\.url\s*=\s*"([^"]+)"', m.group(1))
    return url.group(1) if url else None


def theme_files(archive: bytes) -> list[str]:
    """The theme files of the package: regular files one level below its top directory."""
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
        return sorted(m.name.split("/", 1)[1] for m in tar.getmembers()
                      if m.isfile() and m.name.count("/") == 1)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ghostty")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("new")
    args = ap.parse_args()

    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True,
                               text=True).stdout.strip())
    clone = args.ghostty or str(root / ".upstream/ghostty")
    pins_path = root / "build/pins.env"
    pins = read_pins(pins_path)

    def git(*a: str) -> str:
        return subprocess.run(["git", "-C", clone, *a], check=True, capture_output=True, text=True).stdout

    rev = git("rev-parse", f"{args.new}^{{commit}}").strip()
    zon = git("show", f"{rev}:build.zig.zon")
    new: dict[str, str] = {}
    notes: list[str] = []

    if rev != pins["LIBGHOSTTY_VT_REV"]:
        new["LIBGHOSTTY_VT_REV"] = rev
        repo = pins["LIBGHOSTTY_VT_REPO"]
        new["LIBGHOSTTY_VT_SHA256"] = sha256(fetch(f"{repo}/archive/{rev}.tar.gz"))
        notes.append("the LIBGHOSTTY_VT comment block: the new revision's date, its merge and its green "
                     "build-libghostty-vt* jobs")

    zig = re.search(r'\.minimum_zig_version\s*=\s*"([^"]+)"', zon)
    if not zig:
        raise SystemExit("bump-pins: no minimum_zig_version in build.zig.zon")
    if zig.group(1) != pins["LIBGHOSTTY_VT_ZIG_VERSION"]:
        version = zig.group(1)
        index = json.loads(fetch("https://ziglang.org/download/index.json"))
        if version not in index:
            raise SystemExit(f"bump-pins: zig {version} is not in ziglang.org's index.json")
        new["LIBGHOSTTY_VT_ZIG_VERSION"] = version
        for suffix, key in ZIG_TARGETS.items():
            if key not in index[version]:
                raise SystemExit(f"bump-pins: zig {version} has no {key} archive")
            new[f"LIBGHOSTTY_VT_ZIG_SHA256_{suffix}"] = index[version][key]["shasum"]
        notes.append(f"zig {version}: the zig comment's lookup date; every zig-version mention outside pins.env "
                     "(rg -n 'ZIG_VERSION|zig 0\\.' scripts crates README.md .github)")

    themes = zon_dep_url(zon, "iterm2_themes")
    if themes is None:
        raise SystemExit("bump-pins: build.zig.zon declares no iterm2_themes package")
    if themes != pins["LIBGHOSTTY_VT_THEMES_URL"]:
        data = fetch(themes)
        files = theme_files(data)
        known = re.search(r"^LIBGHOSTTY_VT_KNOWN_THEMES=\((.*)\)", pins_path.read_text(), re.M)
        missing = [t for t in re.findall(r'"([^"]+)"', known.group(1) if known else "") if t not in files]
        if missing:
            notes.append(f"LIBGHOSTTY_VT_KNOWN_THEMES lists themes the new package lacks: {missing}")
        new["LIBGHOSTTY_VT_THEMES_URL"] = themes
        new["LIBGHOSTTY_VT_THEMES_SHA256"] = sha256(data)
        new["LIBGHOSTTY_VT_THEME_COUNT"] = str(len(files))
        notes.append("the themes comment: the package's Zig hash in build.zig.zon")

    for dep, prefix in FONT_DEPS.items():
        url = zon_dep_url(zon, dep)
        if url is None:
            notes.append(f"build.zig.zon no longer declares {dep}: decide what ngterm-gpui embeds instead")
            continue
        if url != pins[f"{prefix}_URL"]:
            new[f"{prefix}_URL"] = url
            new[f"{prefix}_SHA256"] = sha256(fetch(url))
            notes.append(f"{dep} moved: the FONTS comment (versions, Zig hash), README's font versions, and "
                         "scripts/fetch-fonts.sh's paths inside the archive (run it)")

    for name, value in new.items():
        print(f"{name}: {pins.get(name, '<none>')} -> {value}")
    for note in notes:
        print(f"# hand edit: {note}")

    if args.apply and new:
        text = pins_path.read_text()
        for name, value in new.items():
            quoted = not re.search(rf"^{name}=\d+\s*$", text, re.M)
            line = f'{name}="{value}"' if quoted else f"{name}={value}"
            text, n = re.subn(rf"^{name}=.*$", line, text, count=1, flags=re.M)
            if n != 1:
                raise SystemExit(f"bump-pins: no {name}= line in {pins_path}")
        pins_path.write_text(text)

    print(f"PINS=moved {' '.join(new)}" if new else "PINS=unchanged")
    return 0


if __name__ == "__main__":
    sys.exit(main())
