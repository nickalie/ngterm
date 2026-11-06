#!/usr/bin/env python3
"""Resolve what a run of .github/workflows/release.yml builds and how strictly it signs.

The `version` job runs it and writes these outputs to $GITHUB_OUTPUT:

- version, commit: the workspace version (Cargo.toml) and the checked-out commit;
- tag: the release tag when publishing, else empty;
- android_version_code: major * 1_000_000 + minor * 1_000 + patch (minor and patch below 1000);
- publishing: "true" for a tag push, or a dispatch with `publish: true` on a tag ref; a dispatch with
  `publish: true` on any other ref is an error, not a dry run. A dispatch with `publish: false` is a dry run;
- signing_mode: "optional" or "required". A dry run is always optional; a publishing run is required when the
  repository variable NGTERM_RELEASE_SIGNING is "required" (empty or "optional" means optional, anything else is
  an error);
- require_minisign, require_developer_id: "true" when publishing and the updater's trust anchor for that component
  (crates/ngterm-update/trust/minisign.pub, apple-team-id) is non-empty, ignoring whitespace; a missing file counts
  as empty. Each forces its own component only;
- platforms: a JSON list out of linux, macos, windows, android, from the `platforms` input ("all", or a comma- or
  space-separated subset); all of them when publishing.

When publishing, the tag must be v<version> and point at the checked-out commit.

It reads GITHUB_EVENT_NAME, GITHUB_REF, GITHUB_REF_TYPE, GITHUB_REF_NAME, RELEASE_PUBLISH and RELEASE_PLATFORMS (the
dispatch inputs) and NGTERM_RELEASE_SIGNING from the environment.

Usage: scripts/release-version.py [--self-test]
"""

import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRUST = ROOT / "crates/ngterm-update/trust"
PLATFORMS = ["linux", "macos", "windows", "android"]
# Google Play's largest versionCode
MAX_VERSION_CODE = 2_100_000_000


class ReleaseError(Exception):
    pass


def android_version_code(version):
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", version)
    if not match:
        raise ReleaseError(f"Cargo.toml's version {version!r} is not MAJOR.MINOR.PATCH")
    major, minor, patch = (int(part) for part in match.groups())
    if minor >= 1000 or patch >= 1000:
        raise ReleaseError(f"version {version}: minor and patch must be below 1000 for the Android version code")
    code = major * 1_000_000 + minor * 1_000 + patch
    if code > MAX_VERSION_CODE:
        raise ReleaseError(f"version {version}: the Android version code {code} exceeds {MAX_VERSION_CODE}")
    return code


def is_publishing(event, ref_type, publish_input):
    if event == "push":
        if ref_type != "tag":
            raise ReleaseError("a push publishes only from a tag")
        return True
    if event == "workflow_dispatch":
        if publish_input not in ("true", "false"):
            raise ReleaseError(f"the publish input must be true or false, not {publish_input!r}")
        if publish_input == "false":
            return False
        if ref_type != "tag":
            raise ReleaseError("publish: true runs on a tag ref: gh workflow run release --ref vX.Y.Z -f publish=true")
        return True
    raise ReleaseError(f"release.yml does not run on {event!r}")


def signing_mode(publishing, variable):
    if variable not in ("", "optional", "required"):
        raise ReleaseError(f"NGTERM_RELEASE_SIGNING must be optional or required, not {variable!r}")
    return "required" if publishing and variable == "required" else "optional"


def platforms(publishing, platforms_input):
    if publishing:
        return list(PLATFORMS)
    requested = [name for name in re.split(r"[\s,]+", platforms_input.strip()) if name]
    if requested in ([], ["all"]):
        return list(PLATFORMS)
    unknown = sorted(set(requested) - set(PLATFORMS))
    if unknown:
        raise ReleaseError(f"unknown platforms {', '.join(unknown)} (choose from all, {', '.join(PLATFORMS)})")
    return [name for name in PLATFORMS if name in requested]


def resolve(*, event, ref_type, ref_name, publish_input, platforms_input, signing_variable, version, anchors):
    """Every output but the commit, from the run's context; anchors maps a trust file's name to its contents."""
    publishing = is_publishing(event, ref_type, publish_input)
    code = android_version_code(version)
    if publishing and ref_name != f"v{version}":
        raise ReleaseError(f"tag {ref_name!r} must match Cargo.toml: v{version}")

    def anchored(name):
        return publishing and bool(anchors.get(name, "").strip())

    return {
        "version": version,
        "tag": ref_name if publishing else "",
        "android_version_code": str(code),
        "publishing": str(publishing).lower(),
        "signing_mode": signing_mode(publishing, signing_variable),
        "require_minisign": str(anchored("minisign.pub")).lower(),
        "require_developer_id": str(anchored("apple-team-id")).lower(),
        "platforms": json.dumps(platforms(publishing, platforms_input), separators=(",", ":")),
    }


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def main():
    env = os.environ
    with open(ROOT / "Cargo.toml", "rb") as source:
        version = tomllib.load(source)["workspace"]["package"]["version"]
    anchors = {}
    for name in ("minisign.pub", "apple-team-id"):
        path = TRUST / name
        anchors[name] = path.read_text() if path.is_file() else ""
    outputs = resolve(
        event=env.get("GITHUB_EVENT_NAME", ""),
        ref_type=env.get("GITHUB_REF_TYPE", ""),
        ref_name=env.get("GITHUB_REF_NAME", ""),
        publish_input=env.get("RELEASE_PUBLISH") or "false",
        platforms_input=env.get("RELEASE_PLATFORMS", ""),
        signing_variable=env.get("NGTERM_RELEASE_SIGNING", ""),
        version=version,
        anchors=anchors,
    )
    outputs["commit"] = git("rev-parse", "HEAD")
    if outputs["publishing"] == "true":
        tagged = git("rev-parse", f"refs/tags/{outputs['tag']}^{{commit}}")
        if tagged != outputs["commit"]:
            raise ReleaseError(f"the checkout {outputs['commit']} is not the commit of {outputs['tag']} ({tagged})")
    lines = "".join(f"{key}={value}\n" for key, value in outputs.items())
    sys.stdout.write(lines)
    if "GITHUB_OUTPUT" in env:
        with open(env["GITHUB_OUTPUT"], "a") as output:
            output.write(lines)


def self_test():
    def run(**overrides):
        context = {
            "event": "workflow_dispatch",
            "ref_type": "branch",
            "ref_name": "main",
            "publish_input": "false",
            "platforms_input": "all",
            "signing_variable": "",
            "version": "1.2.3",
            "anchors": {},
        }
        context.update(overrides)
        return resolve(**context)

    def fails(message, **overrides):
        try:
            run(**overrides)
        except ReleaseError as error:
            assert message in str(error), (message, str(error))
        else:
            raise AssertionError(f"expected an error containing {message!r} for {overrides}")

    tag = {"ref_type": "tag", "ref_name": "v1.2.3"}
    push = {"event": "push", "publish_input": "", **tag}
    both = {"minisign.pub": "untrusted comment: key\nRWQ...\n", "apple-team-id": "ABCDE12345\n"}
    every = '["linux","macos","windows","android"]'

    # the Android version code and its bounds
    assert run()["android_version_code"] == "1002003"
    assert run(version="0.1.0")["android_version_code"] == "1000"
    assert run(version="2100.0.0")["android_version_code"] == "2100000000"
    assert run(version="0.999.999")["android_version_code"] == "999999"
    fails("below 1000", version="0.1000.0")
    fails("below 1000", version="0.0.1000")
    fails("exceeds", version="2100.0.1")
    fails("not MAJOR.MINOR.PATCH", version="1.2")
    fails("not MAJOR.MINOR.PATCH", version="1.2.3-rc.1")

    # what publishes: a tag push, or publish: true on a tag ref; publish: true elsewhere is an error
    assert run(**push)["publishing"] == "true"
    assert run(**push)["tag"] == "v1.2.3"
    assert run(publish_input="true", **tag)["publishing"] == "true"
    assert run()["publishing"] == "false"
    assert run()["tag"] == ""
    assert run(**tag)["publishing"] == "false"
    fails("runs on a tag ref", publish_input="true")
    fails("only from a tag", event="push", publish_input="")
    fails("does not run on", event="schedule")
    fails("true or false", publish_input="yes")

    # the tag is checked only when publishing
    fails("must match Cargo.toml", **{**push, "ref_name": "v1.2.4"})
    fails("must match Cargo.toml", publish_input="true", ref_type="tag", ref_name="1.2.3")
    assert run(ref_type="tag", ref_name="v9.9.9")["publishing"] == "false"

    # signing mode: dry runs are always optional; publishing follows the variable
    for variable in ("", "optional", "required"):
        assert run(signing_variable=variable)["signing_mode"] == "optional"
        assert run(signing_variable=variable, anchors=both)["signing_mode"] == "optional"
    assert run(signing_variable="", **push)["signing_mode"] == "optional"
    assert run(signing_variable="optional", **push)["signing_mode"] == "optional"
    assert run(signing_variable="required", **push)["signing_mode"] == "required"
    assert run(signing_variable="required", publish_input="true", **tag)["signing_mode"] == "required"
    fails("optional or required", signing_variable="Required")
    fails("optional or required", signing_variable="required ", **push)

    # each trust anchor forces only its own component, only when publishing; blank or missing counts as empty
    def required(**overrides):
        outputs = run(**overrides)
        return outputs["require_minisign"], outputs["require_developer_id"], outputs["signing_mode"]

    assert required(**push) == ("false", "false", "optional")
    assert required(anchors={"minisign.pub": both["minisign.pub"]}, **push) == ("true", "false", "optional")
    assert required(anchors={"apple-team-id": both["apple-team-id"]}, **push) == ("false", "true", "optional")
    assert required(anchors=both, **push) == ("true", "true", "optional")
    assert required(anchors=both, signing_variable="required", **push) == ("true", "true", "required")
    assert required(anchors={"minisign.pub": " \n", "apple-team-id": ""}, **push) == ("false", "false", "optional")
    assert required(anchors=both) == ("false", "false", "optional")
    assert required(anchors=both, signing_variable="required") == ("false", "false", "optional")

    # platforms: a dry run's subset, in a fixed order; publishing builds all
    assert run()["platforms"] == every
    assert run(platforms_input="")["platforms"] == every
    assert run(platforms_input="linux")["platforms"] == '["linux"]'
    assert run(platforms_input="windows, macos")["platforms"] == '["macos","windows"]'
    assert run(platforms_input="android linux linux")["platforms"] == '["linux","android"]'
    assert run(platforms_input="linux", **push)["platforms"] == every
    assert run(platforms_input="macos", publish_input="true", **tag)["platforms"] == every
    fails("unknown platforms ios", platforms_input="linux,ios")
    fails("unknown platforms all", platforms_input="all,linux")
    print("release-version self-test: ok")


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--self-test"]:
            self_test()
        elif sys.argv[1:]:
            raise SystemExit("usage: scripts/release-version.py [--self-test]")
        else:
            main()
    except ReleaseError as error:
        print(f"::error::{error}", file=sys.stderr)
        raise SystemExit(1) from None
