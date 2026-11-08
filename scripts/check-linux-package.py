#!/usr/bin/env python3
"""Keep the Linux app package independent of the separately installed web engine.

--prepare patches the app's library closure and removes the worker's build-time
RUNPATH before checking. A plain invocation checks fixture or finished trees
without modifying them and without requiring an installed WPE engine.
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
    worker = root / "usr/bin/ngterm-web"
    rpath = output("patchelf", "--print-rpath", worker)
    if rpath:
        raise ValueError(f"ngterm-web must have no RPATH or RUNPATH: {rpath}")
    for path in root.rglob("*"):
        if engine_library(path.name) or path.name in ("gstreamer-1.0", "wpe-webkit-2.0", "bundled-wpe"):
            raise ValueError(f"web engine payload in app package: {path.relative_to(root)}")


def prepare(root: Path) -> None:
    spec = importlib.util.spec_from_file_location("web_engine_closure", REPO / "scripts/web-engine-closure.py")
    closure = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(closure)
    worker = root / "usr/bin/ngterm-web"
    # The worker's dependencies deliberately need not resolve until an engine
    # is selected. Everything else must resolve inside the app or host policy.
    subprocess.run(["patchelf", "--remove-rpath", str(worker)], check=True)
    closure.patch_and_check(root, root / "usr/lib", REPO / "packaging/linux/runtime-libraries.txt",
                            exclude=(worker,))


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
