"""release-artifacts.py's web engine runtime handling, against a fake `gh` that records its calls."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess

import pytest


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/release-artifacts.py"
KEY = "a" * 64
OTHER_KEY = "b" * 64
FAKE_GH = """#!/usr/bin/env python3
import json, os, pathlib, shutil, sys
args = sys.argv[1:]
state = pathlib.Path(os.environ["FAKE_GH_STATE"])
with (state / "calls").open("a") as log:
    log.write(json.dumps(args) + "\\n")
fail = (state / "fail").read_text() if (state / "fail").exists() else ""
if fail and args[:2] == ["release", "view"]:
    sys.exit(print(fail, file=sys.stderr) or 1)
releases = json.loads((state / "releases").read_text())
if args[:2] == ["release", "view"]:
    if args[2] not in releases:
        sys.exit(print("release not found", file=sys.stderr) or 1)
    print(json.dumps({"assets": [{"name": name, **asset} for name, asset in releases[args[2]].items()]}))
elif args[:2] == ["release", "list"]:
    print("\\n".join(releases))
elif args[:2] == ["release", "download"]:
    name = args[args.index("--pattern") + 1]
    target = pathlib.Path(args[args.index("--dir") + 1]) / name
    target.write_bytes((state / "published").read_bytes())
elif args[:2] == ["release", "delete-asset"]:
    del releases[args[2]][args[3]]
    (state / "releases").write_text(json.dumps(releases))
elif args[:2] == ["release", "upload"]:
    name = pathlib.Path(args[-1]).name
    if name in releases.get(args[2], {}):
        sys.exit(print("asset already exists", file=sys.stderr) or 1)
"""


def archive_name(key: str = KEY, arch: str = "x86_64") -> str:
    return f"ngterm-web-engine-{key}-linux-{arch}.tar.zst"


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


@pytest.fixture
def gh(tmp_path: Path):
    """Run the script with the fake gh first on PATH; `.releases`, `.calls()` and `.published` shape and read it."""
    state = tmp_path / "gh-state"
    state.mkdir()
    (state / "releases").write_text("{}")
    tools = tmp_path / "tools"
    tools.mkdir()
    fake = tools / "gh"
    fake.write_text(FAKE_GH)
    fake.chmod(fake.stat().st_mode | stat.S_IEXEC)

    class Gh:
        directory = state

        @staticmethod
        def releases(value: dict) -> None:
            releases = {
                tag: {name: {"id": f"RA_{index}", "state": "uploaded", "digest": asset}
                      if isinstance(asset, str) else asset for index, (name, asset) in enumerate(assets.items())}
                for tag, assets in value.items()
            }
            (state / "releases").write_text(json.dumps(releases))

        @staticmethod
        def published(data: bytes) -> None:
            (state / "published").write_bytes(data)

        @staticmethod
        def calls() -> list[list[str]]:
            log = state / "calls"
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        @staticmethod
        def run(*args: str) -> subprocess.CompletedProcess:
            env = {name: value for name, value in os.environ.items() if not name.startswith("NGTERM_")}
            env["PATH"] = f"{tools}{os.pathsep}{env['PATH']}"
            env["FAKE_GH_STATE"] = str(state)
            return subprocess.run(["python3", str(SCRIPT), *args], capture_output=True, text=True, env=env)

    return Gh


def manifest_for(gh, directory: Path, name: str, data: bytes) -> str:
    directory.mkdir(exist_ok=True)
    (directory / name).write_bytes(data)
    result = gh.run("manifest", str(directory), "11", "--web-engine", name)
    assert result.returncode == 0, result.stderr
    return result.stdout.strip()


def test_a_runtime_not_published_is_built_and_uploaded(gh, tmp_path: Path) -> None:
    result = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "dist"))
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "upload"
    assert not (tmp_path / "dist").exists()


def test_each_architecture_chooses_for_itself(gh, tmp_path: Path) -> None:
    data = b"published engine"
    gh.releases({f"web-engine-{KEY}": {archive_name(): f"sha256:{digest(data)}"}})
    gh.published(data)
    published = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "x86"))
    other_arch = gh.run("web-engine-fetch", KEY, "aarch64", str(tmp_path / "arm"))
    assert published.stdout.strip() == "reuse", published.stderr
    assert (tmp_path / "x86" / archive_name()).read_bytes() == data
    assert other_arch.stdout.strip() == "upload", other_arch.stderr


def test_a_published_runtime_that_fails_its_digest_is_refused(gh, tmp_path: Path) -> None:
    gh.releases({f"web-engine-{KEY}": {archive_name(): f"sha256:{digest(b'what github hashed')}"}})
    gh.published(b"what arrived")
    result = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "dist"))
    assert result.returncode == 1
    assert "downloaded sha256" in result.stderr


def test_a_failed_lookup_is_not_taken_for_an_unpublished_runtime(gh, tmp_path: Path) -> None:
    (gh.directory / "fail").write_text("HTTP 502: bad gateway")
    result = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "dist"))
    assert result.returncode == 1
    assert "502" in result.stderr
    assert result.stdout == ""


def test_a_new_runtime_gets_a_prerelease_that_is_not_latest(gh, tmp_path: Path) -> None:
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), b"new engine")
    result = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert result.returncode == 0, result.stderr
    calls = gh.calls()
    create = next(call for call in calls if call[:2] == ["release", "create"])
    assert create[2] == f"web-engine-{KEY}"
    assert "--prerelease" in create and "--latest=false" in create
    assert create[create.index("--target") + 1] == "c0ffee"
    upload = next(call for call in calls if call[:2] == ["release", "upload"])
    assert upload[2] == f"web-engine-{KEY}" and upload[-1].endswith(archive_name())
    assert calls.index(create) < calls.index(upload)


def test_an_existing_release_without_the_asset_is_kept_a_prerelease(gh, tmp_path: Path) -> None:
    gh.releases({f"web-engine-{KEY}": {}})
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), b"new engine")
    result = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert result.returncode == 0, result.stderr
    calls = gh.calls()
    assert not any(call[:2] == ["release", "create"] for call in calls)
    edit = next(call for call in calls if call[:2] == ["release", "edit"])
    assert "--prerelease" in edit and "--latest=false" in edit
    assert any(call[:2] == ["release", "upload"] for call in calls)


def test_a_runtime_already_published_is_left_alone(gh, tmp_path: Path) -> None:
    data = b"same engine"
    gh.releases({f"web-engine-{KEY}": {archive_name(): f"sha256:{digest(data)}"}})
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), data)
    result = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert result.returncode == 0, result.stderr
    assert [call[:2] for call in gh.calls()] == [["release", "view"]]


def test_a_different_build_under_a_published_key_never_replaces_it(gh, tmp_path: Path) -> None:
    gh.releases({f"web-engine-{KEY}": {archive_name(): f"sha256:{digest(b'the shipped engine')}"}})
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), b"a rebuilt engine")
    result = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert result.returncode == 1
    assert "shipped apps pin" in result.stderr
    assert [call[:2] for call in gh.calls()] == [["release", "view"]]


def test_both_architectures_reach_their_own_releases(gh, tmp_path: Path) -> None:
    gh.releases({f"web-engine-{KEY}": {archive_name(): f"sha256:{digest(b'x86 engine')}"}})
    x86 = manifest_for(gh, tmp_path / "x86", archive_name(), b"x86 engine")
    arm_name = archive_name(OTHER_KEY, "aarch64")
    arm = manifest_for(gh, tmp_path / "arm", arm_name, b"arm engine")
    (tmp_path / "x86" / arm_name).write_bytes(b"arm engine")
    result = gh.run("web-engine-publish", str(tmp_path / "x86"), "c0ffee", x86, arm, "")
    assert result.returncode == 0, result.stderr
    created = [call[2] for call in gh.calls() if call[:2] == ["release", "create"]]
    uploaded = [call[2] for call in gh.calls() if call[:2] == ["release", "upload"]]
    assert created == uploaded == [f"web-engine-{OTHER_KEY}"]


def test_a_runtime_is_summed_but_never_an_app_release_asset(gh, tmp_path: Path) -> None:
    dist = tmp_path / "dist"
    dist.mkdir()
    (dist / "ngterm.deb").write_bytes(b"deb")
    manifest_text = manifest_for(gh, dist, archive_name(), b"engine")
    assets = gh.run("assets", manifest_text)
    assert assets.stdout.split() == ["ngterm.deb"], assets.stderr
    sums = gh.run("sums", str(dist), manifest_text)
    assert f"{digest(b'engine')}  {archive_name()}" in sums.stdout.splitlines(), sums.stderr


@pytest.mark.parametrize("upload_state", ["starter", "new"])
def test_an_incomplete_upload_is_rebuilt_then_removed_before_upload(gh, tmp_path: Path, upload_state: str) -> None:
    # Even a digest does not turn an incomplete upload into a published asset.
    gh.releases({f"web-engine-{KEY}": {archive_name(): {
        "id": "RA_incomplete", "state": upload_state, "digest": f"sha256:{digest(b'incomplete')}"
    }}})
    fetched = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "dist"))
    assert fetched.returncode == 0, fetched.stderr
    assert fetched.stdout.strip() == "upload"
    assert not (tmp_path / "dist").exists()
    assert [call[:2] for call in gh.calls()] == [["release", "view"]]
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), b"complete engine")
    published = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert published.returncode == 0, published.stderr
    calls = gh.calls()
    delete = ["release", "delete-asset", f"web-engine-{KEY}", archive_name(), "--yes"]
    assert delete in calls
    upload = next(call for call in calls if call[:2] == ["release", "upload"])
    assert calls.index(delete) < calls.index(upload)
    assert "--clobber" not in upload


def test_a_completed_upload_without_a_digest_is_never_replaced(gh, tmp_path: Path) -> None:
    gh.releases({f"web-engine-{KEY}": {archive_name(): {"id": "RA_uploaded", "state": "uploaded"}}})
    fetched = gh.run("web-engine-fetch", KEY, "x86_64", str(tmp_path / "dist"))
    assert fetched.returncode == 1 and "no sha256 digest" in fetched.stderr
    manifest = manifest_for(gh, tmp_path / "dist", archive_name(), b"complete engine")
    published = gh.run("web-engine-publish", str(tmp_path / "dist"), "c0ffee", manifest)
    assert published.returncode == 1 and "published with no digest" in published.stderr
    assert all(call[:2] == ["release", "view"] for call in gh.calls())


@pytest.mark.parametrize("include_valid_runtime", [False, True])
def test_linux_manifests_without_a_runtime_fail_before_any_release_write(
    gh, tmp_path: Path, include_valid_runtime: bool,
) -> None:
    dist = tmp_path / "dist"
    dist.mkdir()
    (dist / "ngterm.deb").write_bytes(b"deb")
    # Dropping --web-engine makes the archive an app asset, which must fail closed.
    (dist / archive_name()).write_bytes(b"engine")
    result = gh.run("manifest", str(dist), "11")
    assert result.returncode == 0, result.stderr
    manifests = [result.stdout.strip()]
    if include_valid_runtime:
        manifests.insert(0, manifest_for(gh, tmp_path / "other", archive_name(OTHER_KEY), b"other engine"))
    published = gh.run("web-engine-publish", str(dist), "c0ffee", *manifests)
    assert published.returncode == 1
    assert "no web engine runtime in Linux manifest" in published.stderr
    assert gh.calls() == []


@pytest.mark.parametrize(
    ("tag", "expected"),
    [("v0.10.0", "v0.9.2"), ("v0.9.2", "v0.1.1"), ("v0.1.1", ""), ("v1.0.0", "v0.10.0")],
)
def test_generated_notes_start_at_the_previous_app_release(gh, tag: str, expected: str) -> None:
    gh.releases({f"web-engine-{KEY}": {}, "v0.1.1": {}, "v0.9.2": {}, "v0.10.0": {}})
    result = gh.run("previous-tag", tag)
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == expected
    assert "--exclude-pre-releases" in gh.calls()[0]


def test_previous_tag_refuses_a_tag_that_is_no_app_release(gh) -> None:
    result = gh.run("previous-tag", f"web-engine-{KEY}")
    assert result.returncode != 0
    assert "is not a vMAJOR.MINOR.PATCH tag" in result.stderr
