#!/usr/bin/env python3
"""Hand release files from job to job in .github/workflows/release.yml, checked by digest at every step.

A job that uploads an artifact describes it in a manifest, one line of JSON it puts in a job output:
{"artifact_id": "...", "files": {"<name>": "<sha256>", ...}, "assets": ["<name>", ...]}. An artifact is a flat set of
regular files; `assets` are the ones that become release assets (the rest, such as a build tarball or the upgrade
test MSI, never do). A job that receives artifacts downloads them by id and checks them against the manifests before
it uses anything. An empty manifest argument is a job that did not run (a platform a dry run left out) and is skipped.

  manifest DIR ARTIFACT_ID [--not-asset NAME]...  print DIR's manifest
  ids MANIFEST...                                 print the artifact ids, comma-separated (download-artifact's input)
  check DIR MANIFEST...                           DIR holds exactly the manifests' files, with their digests
  assets MANIFEST...                              print the release asset names, one per line
  sums DIR MANIFEST...                            print SHA256SUMS over the release assets, after `check`
  extract TARBALL DEST                            unpack a build tarball, refusing absolute paths, `..`, links that
                                                  point outside DEST, and device files
  --self-test

Usage: scripts/release-artifacts.py COMMAND ...
"""

import hashlib
import io
import json
import sys
import tarfile
import tempfile
from pathlib import Path, PurePosixPath


class ArtifactError(Exception):
    pass


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as source:
        for block in iter(lambda: source.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def flat_files(directory):
    """The names of DIR's entries, which must all be regular files."""
    names = []
    for path in sorted(Path(directory).iterdir()):
        if path.is_symlink() or not path.is_file():
            raise ArtifactError(f"{path} is not a regular file: an artifact is a flat set of files")
        names.append(path.name)
    return names


def manifest(directory, artifact_id, not_assets):
    if not artifact_id:
        raise ArtifactError("no artifact id: did the upload fail?")
    files = {name: sha256(Path(directory) / name) for name in flat_files(directory)}
    if not files:
        raise ArtifactError(f"{directory} is empty")
    missing = sorted(set(not_assets) - set(files))
    if missing:
        raise ArtifactError(f"--not-asset names files {directory} lacks: {', '.join(missing)}")
    assets = [name for name in files if name not in not_assets]
    return {"artifact_id": str(artifact_id), "files": files, "assets": assets}


def parse(manifests):
    parsed = []
    for text in manifests:
        if not text.strip():
            continue
        try:
            value = json.loads(text)
        except json.JSONDecodeError as error:
            raise ArtifactError(f"a manifest is not JSON: {error}") from None
        if not isinstance(value, dict) or set(value) != {"artifact_id", "files", "assets"}:
            raise ArtifactError("a manifest has artifact_id, files and assets, and nothing else")
        if not set(value["assets"]) <= set(value["files"]):
            raise ArtifactError(f"artifact {value['artifact_id']}: an asset is not one of its files")
        parsed.append(value)
    if not parsed:
        raise ArtifactError("no manifests: no job produced anything")
    return parsed


def union(manifests):
    """Every file of the manifests, name to digest; a name two artifacts share is an error."""
    files = {}
    for value in manifests:
        for name, digest in value["files"].items():
            if name in files:
                raise ArtifactError(f"{name} is in two artifacts")
            files[name] = digest
    return files


def ids(manifests):
    return ",".join(value["artifact_id"] for value in parse(manifests))


def check(directory, manifests):
    expected = union(parse(manifests))
    present = set(flat_files(directory))
    if present != set(expected):
        extra = ", ".join(sorted(present - set(expected))) or "none"
        missing = ", ".join(sorted(set(expected) - present)) or "none"
        raise ArtifactError(f"{directory} does not hold the reported files: unexpected {extra}; missing {missing}")
    for name, digest in sorted(expected.items()):
        actual = sha256(Path(directory) / name)
        if actual != digest:
            raise ArtifactError(f"{name}: sha256 {actual}, but its job reported {digest}")


def assets(manifests):
    parsed = parse(manifests)
    union(parsed)
    return sorted(name for value in parsed for name in value["assets"])


def sums(directory, manifests):
    check(directory, manifests)
    files = union(parse(manifests))
    return "".join(f"{files[name]}  {name}\n" for name in assets(manifests))


def refuse_member(tarball, info):
    """Refuse a member that would land, or point, outside the destination, or that is not a file, directory or
    symlink."""
    path = PurePosixPath(info.name)
    if path.is_absolute() or ".." in path.parts:
        raise ArtifactError(f"{tarball}: {info.name} is absolute or leaves the tree")
    if not (info.isfile() or info.isdir() or info.issym()):
        raise ArtifactError(f"{tarball}: {info.name} is not a file, directory or symlink")
    if info.issym():
        target = PurePosixPath(info.linkname)
        depth = len(path.parent.parts)
        for part in target.parts:
            depth += -1 if part == ".." else 0 if part == "." else 1
            if depth < 0:
                break
        if target.is_absolute() or depth < 0:
            raise ArtifactError(f"{tarball}: {info.name} links to {info.linkname}, outside the tree")


def extract(tarball, destination):
    if not hasattr(tarfile, "data_filter"):
        raise ArtifactError("this Python's tarfile has no extraction filters: use Python 3.12 or later")
    destination = Path(destination)
    if destination.exists() and any(destination.iterdir()):
        raise ArtifactError(f"{destination} is not empty")
    destination.mkdir(parents=True, exist_ok=True)
    with tarfile.open(tarball) as archive:
        for info in archive.getmembers():
            refuse_member(tarball, info)
        try:
            # `data` refuses absolute and `..` paths, links leading outside DEST, and device files, and clears
            # set-id and group/other write bits
            archive.extractall(destination, filter="data")
        except tarfile.FilterError as error:
            raise ArtifactError(f"{tarball}: {error}") from None


def self_test():
    with tempfile.TemporaryDirectory() as scratch:
        root = Path(scratch)

        def expect_error(message, function, *args):
            try:
                function(*args)
            except ArtifactError as error:
                assert message in str(error), (message, str(error))
            else:
                raise AssertionError(f"expected an error containing {message!r}")

        # manifests and checks
        linux = root / "linux"
        linux.mkdir()
        (linux / "a.tar.gz").write_bytes(b"a")
        (linux / "a.deb").write_bytes(b"b")
        windows = root / "windows"
        windows.mkdir()
        (windows / "w.msi").write_bytes(b"c")
        (windows / "upgrade-test.msi").write_bytes(b"d")
        first = json.dumps(manifest(linux, "11", []))
        second = json.dumps(manifest(windows, 22, ["upgrade-test.msi"]))
        assert json.loads(first)["assets"] == ["a.deb", "a.tar.gz"]
        assert json.loads(second)["assets"] == ["w.msi"]
        assert ids([first, "", second]) == "11,22"
        assert assets([first, second, " "]) == ["a.deb", "a.tar.gz", "w.msi"]
        expect_error("no artifact id", manifest, linux, "", [])
        expect_error("--not-asset names", manifest, linux, "1", ["missing.msi"])
        expect_error("no manifests", ids, ["", ""])
        expect_error("not JSON", ids, ["{"])
        expect_error("nothing else", ids, ['{"artifact_id": "1", "files": {}}'])
        expect_error("not one of its files", ids, ['{"artifact_id": "1", "files": {}, "assets": ["x"]}'])
        expect_error("in two artifacts", assets, [first, first])

        merged = root / "merged"
        merged.mkdir()
        for source in (linux, windows):
            for path in source.iterdir():
                (merged / path.name).write_bytes(path.read_bytes())
        check(merged, [first, second])
        listed = sums(merged, [first, second, ""])
        assert listed.splitlines() == [
            f"{hashlib.sha256(b'b').hexdigest()}  a.deb",
            f"{hashlib.sha256(b'a').hexdigest()}  a.tar.gz",
            f"{hashlib.sha256(b'c').hexdigest()}  w.msi",
        ], listed
        expect_error("missing upgrade-test.msi", check, linux, [first, second])
        (merged / "stray").write_bytes(b"")
        expect_error("unexpected stray", check, merged, [first, second])
        (merged / "stray").unlink()
        (merged / "a.deb").write_bytes(b"tampered")
        expect_error("a.deb: sha256", check, merged, [first, second])
        (merged / "a.deb").write_bytes(b"b")
        (merged / "link").symlink_to("a.deb")
        expect_error("not a regular file", check, merged, [first, second])
        (merged / "link").unlink()
        (merged / "sub").mkdir()
        expect_error("not a regular file", check, merged, [first, second])

        # safe extraction: a tree with an executable and an inner symlink unpacks; escapes are refused
        def tarball(name, *members):
            path = root / name
            with tarfile.open(path, "w") as archive:
                for info, data in members:
                    archive.addfile(info, io.BytesIO(data) if data is not None else None)
            return path

        def member(name, data=b"", mode=0o644, kind=tarfile.REGTYPE, link=""):
            info = tarfile.TarInfo(name)
            info.type, info.mode, info.linkname = kind, mode, link
            info.size = len(data) if kind == tarfile.REGTYPE else 0
            return info, data if kind == tarfile.REGTYPE else None

        good = tarball(
            "good.tar",
            member("ngterm.app/Contents/MacOS/ngterm", b"binary", 0o755),
            member("archive/share/doc", kind=tarfile.DIRTYPE, mode=0o755),
            member("archive/share/doc/LICENSE", b"MIT"),
            member("archive/share/doc/COPYING", kind=tarfile.SYMTYPE, link="LICENSE"),
        )
        extract(good, root / "good")
        binary = root / "good/ngterm.app/Contents/MacOS/ngterm"
        assert binary.read_bytes() == b"binary" and binary.stat().st_mode & 0o111
        assert (root / "good/archive/share/doc/COPYING").read_text() == "MIT"
        expect_error("not empty", extract, good, root / "good")
        refused = [
            member("/etc/evil", b"x"),
            member("../evil", b"x"),
            member("archive/key", kind=tarfile.SYMTYPE, link="/tmp/keychain"),
            member("archive/up", kind=tarfile.SYMTYPE, link="../../outside"),
            member("archive/hard", kind=tarfile.LNKTYPE, link="/etc/passwd"),
            member("archive/device", kind=tarfile.CHRTYPE),
        ]
        for index, bad in enumerate(refused):
            path = tarball(f"bad-{index}.tar", bad)
            expect_error(path.name, extract, path, root / f"bad-{index}")
    print("release-artifacts self-test: ok")


def main(argv):
    if argv == ["--self-test"]:
        self_test()
        return
    if not argv:
        raise SystemExit(__doc__)
    command, args = argv[0], argv[1:]
    if command == "manifest" and len(args) >= 2:
        not_assets = []
        rest = args[2:]
        while rest:
            if rest[0] != "--not-asset" or len(rest) < 2:
                raise SystemExit(__doc__)
            not_assets.append(rest[1])
            rest = rest[2:]
        print(json.dumps(manifest(args[0], args[1], not_assets), separators=(",", ":")))
    elif command == "ids" and args:
        print(ids(args))
    elif command == "check" and len(args) >= 2:
        check(args[0], args[1:])
    elif command == "assets" and args:
        print("\n".join(assets(args)))
    elif command == "sums" and len(args) >= 2:
        sys.stdout.write(sums(args[0], args[1:]))
    elif command == "extract" and len(args) == 2:
        extract(args[0], args[1])
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except ArtifactError as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1) from None
