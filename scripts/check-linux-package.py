#!/usr/bin/env python3
"""Keep the Linux app package to its own binaries, independent of the web engine.

The app bundles no libraries: everything its binaries link is the host's, the
copy the compositor, the GPU drivers and GIO's modules were built against (an
older bundled libxkbcommon crashed on the compositor's keymap). --prepare
removes every binary's build-time RUNPATH and rejects a host library
packaging/linux/runtime-libraries.txt does not declare. A plain invocation
checks fixture or finished trees without modifying them and without requiring
an installed WPE engine.
"""

from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path
import subprocess


REPO = Path(__file__).resolve().parent.parent
APP_BINARIES = ("ngterm", "ngtermctl", "ngterm-session", "ngterm-headless")
ENGINE_LIBRARIES = ("libwpe", "libgst", "libsoup", "libjavascriptcore")


def output(*args: str | Path) -> str:
    return subprocess.check_output([str(arg) for arg in args], text=True).strip()


def engine_library(name: str) -> bool:
    return name.lower().startswith(ENGINE_LIBRARIES)


def check(root: Path) -> None:
    for name in (*APP_BINARIES, "ngterm-web"):
        if not (root / "usr/bin" / name).is_file():
            raise ValueError(f"missing app binary: {name}")
    for name in APP_BINARIES:
        for library in output("patchelf", "--print-needed", root / "usr/bin" / name).splitlines():
            if engine_library(library):
                raise ValueError(f"{name} needs web engine library: {library}")
    # Reject both RUNPATH and legacy RPATH, including absolute build prefixes.
    for name in (*APP_BINARIES, "ngterm-web"):
        rpath = output("patchelf", "--print-rpath", root / "usr/bin" / name)
        if rpath:
            raise ValueError(f"{name} must have no RPATH or RUNPATH: {rpath}")
    binaries = {root / "usr/bin" / name for name in (*APP_BINARIES, "ngterm-web")}
    for path in sorted(root.rglob("*")):
        if engine_library(path.name) or path.name in ("gstreamer-1.0", "wpe-webkit-2.0", "bundled-wpe"):
            raise ValueError(f"web engine payload in app package: {path.relative_to(root)}")
        if path not in binaries and elf(path):
            raise ValueError(f"bundled library in app package: {path.relative_to(root)}")


def elf(path: Path) -> bool:
    if not path.is_file() or path.is_symlink():
        return False
    with path.open("rb") as source:
        return source.read(4) == b"\x7fELF"


def prepare(root: Path) -> None:
    spec = importlib.util.spec_from_file_location("web_engine_closure", REPO / "scripts/web-engine-closure.py")
    closure = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(closure)
    for name in (*APP_BINARIES, "ngterm-web"):
        subprocess.run(["patchelf", "--remove-rpath", str(root / "usr/bin" / name)], check=True)
    # The worker's dependencies deliberately need not resolve until an engine
    # is selected. Everything else must resolve to declared host libraries.
    closure.check_host(root, REPO / "packaging/linux/runtime-libraries.txt", exclude=(root / "usr/bin/ngterm-web",))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prepare", action="store_true")
    parser.add_argument("root", type=Path)
    args = parser.parse_args()
    try:
        root = args.root.resolve()
        if args.prepare:
            prepare(root)
        check(root)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
