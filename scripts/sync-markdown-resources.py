#!/usr/bin/env python3
"""Copy the parity reference's Markdown renderer and licenses reproducibly."""

import argparse
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    upstream = root / ".upstream/agterm/agterm/Resources/markdown"
    destination = root / "resources/markdown"
    failures = []
    for source in sorted(upstream.iterdir()):
        if not source.is_file():
            continue
        contents = source.read_bytes()
        if source.name in {"render.js", "page.css"}:
            contents = contents.replace(b"agterm", b"ngterm")
        target = destination / source.name
        if args.check:
            if not target.exists() or target.read_bytes() != contents:
                failures.append(str(target.relative_to(root)))
        else:
            destination.mkdir(parents=True, exist_ok=True)
            target.write_bytes(contents)
    if failures:
        parser.exit(1, "Markdown resources differ: " + ", ".join(failures) + "\n")


if __name__ == "__main__":
    main()
