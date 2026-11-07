#!/usr/bin/env python3
"""Check the structural invariants .github/workflows/release.yml keeps around its keys (docs/release-signing.md;
docs/plans/completed/20261105-release-signing.md, Overview and Task 9). Each check reads the parsed workflow, not its
text, so a line can't slip from one job to another:

- builders: a job that builds (cargo, Gradle, Xcode, a packaging or staging script, a build cache) uses no
  environment, so no build step and no `build.rs` ever runs where a secret is;
- signers: a job that names the `release` environment needs `approve`, and its condition is a plain conjunction
  with `approve` succeeding; or, only when its environment is exactly `signed_run == 'true' && 'release' || ''` (so an
  unsigned run gets none), with `approve` skipped on an unsigned run (the shape `macos-sign` and `android-sign` use).
  It restores no cache (the cache actions, or a `setup-*` action's cache) and builds nothing;
- secrets: none at workflow or job level (`secrets.X` in any case, `secrets['X']`, `toJSON(secrets)`, a job's
  `secrets:`), only in a step's `env:`;
- permissions: read-only at the workflow level; only `publish` widens them (and `windows-sign` and `play-upload`,
  which other plans add), each to exactly its listed scopes;
- publishing only: a step that writes a GitHub release (`gh release`, `gh api .../releases`, a release action) or
  attests, and every channel or store job (`aur`, `homebrew`, `winget`, `play-upload`), has `publishing == 'true'`
  as a top-level conjunct of its own condition or its job's, so no `||` lets a rehearsal or a dry run through;
- the release: `publish` passes every platform's manifest (Linux both arches, Windows, macOS, iOS, Android) and
  `SHA256SUMS` to its download ids and digest check, every platform's to its asset list, and its `gh release upload`
  sends the assets, `SHA256SUMS` and the signature.

What it guards against: an honest edit that breaks one of these, caught by the lint CI. It doesn't parse the shell a
job runs, so that a signing job never starts a program it downloaded is left to review (user, 2026-11-07); and
whoever can change release.yml can change this file too, so it is no defense against a hostile committer.

Usage: scripts/check-release-workflow.py [WORKFLOW]   check it (default .github/workflows/release.yml)
       scripts/check-release-workflow.py --self-test  check the real workflow, and that each check catches a broken
                                                      copy of it
"""

import copy
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    sys.exit("check-release-workflow.py: needs PyYAML (python3-yaml)")

ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github/workflows/release.yml"

BUILDS = re.compile(
    r"\bcargo\b|build-android\.sh|gradlew|build-ios\.sh|xcodebuild|package-release\.sh|stage\.sh|build-msi\.sh"
    r"|build-ghostty-vt\.sh|build-release-wpe\.sh"
)
CACHE_ACTIONS = ("actions/cache", "Swatinem/rust-cache", "gradle/actions/setup-gradle")
RELEASE_ACTIONS = ("actions/attest", "softprops/action-gh-release", "ncipollo/release-action")
# GitHub resolves context names regardless of case
SECRETS = re.compile(r"\bsecrets\b", re.IGNORECASE)
PERMISSIONS = {
    "publish": {"contents": "write", "id-token": "write", "attestations": "write"},
    "windows-sign": {"contents": "read", "actions": "read"},
    "play-upload": {"contents": "read", "id-token": "write"},
}
CHANNELS = ("aur", "homebrew", "winget", "play-upload")
PLATFORMS = ("LINUX_X86_64", "LINUX_AARCH64", "WINDOWS", "MACOS", "IOS", "ANDROID")
PUBLISHING = "needs.version.outputs.publishing == 'true'"
APPROVED = "needs.approve.result == 'success'"
# the environment of a signer that may also run unsigned: only a signed run reaches `release`
SIGNED_ONLY = "needs.version.outputs.signed_run == 'true' && 'release' || ''"
APPROVED_OR_DRY = (
    "(needs.approve.result == 'success' || (needs.version.outputs.signed_run != 'true' "
    "&& needs.approve.result == 'skipped'))"
)


def steps(job):
    return job.get("steps") or []


def runs(job):
    """A job's run blocks, with each command continued over lines (a trailing backslash) read as one line."""
    return re.sub(r"\\\n", " ", "\n".join(step.get("run", "") for step in steps(job)))


def environment(job):
    """A job's environment, lowercased: GitHub matches environment names regardless of case."""
    env = job.get("environment")
    if isinstance(env, dict):
        env = env.get("name", "")
    return str(env or "").lower()


def restores_cache(step):
    action = step.get("uses", "")
    if action.startswith(CACHE_ACTIONS):
        return True
    cache = (step.get("with") or {}).get("cache")
    if action.startswith("actions/setup-go"):
        return cache not in (False, "false")
    return action.startswith("actions/setup-") and cache not in (None, False, "", "false")


def condition(text):
    """An `if:` without its `${{ }}` and with its whitespace collapsed."""
    text = " ".join(str(text or "").split())
    if text.startswith("${{") and text.endswith("}}"):
        text = text[3:-2].strip()
    return text


def conjuncts(text):
    """The top-level `&&` parts of a condition, or None when a top-level `||` makes it no plain conjunction."""
    parts, depth, start, i, quoted = [], 0, 0, 0, False
    while i < len(text):
        c = text[i]
        if c == "'":
            quoted = not quoted
        elif not quoted and c == "(":
            depth += 1
        elif not quoted and c == ")":
            depth -= 1
        elif not quoted and depth == 0 and text.startswith("||", i):
            return None
        elif not quoted and depth == 0 and text.startswith("&&", i):
            parts.append(text[start:i].strip())
            start = i + 2
            i += 1
        i += 1
    parts.append(text[start:].strip())
    return [part for part in parts if part]


def guarded(text):
    parts = conjuncts(condition(text))
    return parts is not None and PUBLISHING in parts


def writes_release(step):
    """Whether STEP writes a GitHub release or attests: a `gh` command with a `release` or (with a `/releases` path)
    `api` word after any options and their values, a command continued over lines read as one; or a release or
    attestation action."""
    command = re.sub(r"\\\n", " ", step.get("run", ""))
    return (
        bool(re.search(r"\bgh\b[^\n;|&]*\srelease\b", command))
        or bool(re.search(r"\bgh\b[^\n;|&]*\sapi\b[^\n]*/releases", command))
        or step.get("uses", "").startswith(RELEASE_ACTIONS)
    )


def check_job(name, job):
    problems = []
    env = environment(job)
    builds = bool(BUILDS.search(runs(job))) or any(restores_cache(step) for step in steps(job))
    if builds and env:
        problems.append(f"{name}: builds, yet uses an environment ({env})")
    if any(SECRETS.search(str(value)) for value in (job.get("env") or {}).values()):
        problems.append(f"{name}: references a secret at job level")
    if "secrets" in job:
        problems.append(f"{name}: passes secrets at job level")
    if "'release'" in env or env == "release":
        needs = job.get("needs") or []
        needs = [needs] if isinstance(needs, str) else needs
        if "approve" not in needs:
            problems.append(f"{name}: uses release without needing approve")
        # a skipped approve is safe only where an unsigned run gets no environment at all
        accepted = {APPROVED, APPROVED_OR_DRY} if condition(env) == SIGNED_ONLY else {APPROVED}
        parts = conjuncts(condition(job.get("if")))
        if parts is None or not (accepted & set(parts)):
            problems.append(f"{name}: uses release without a condition that requires approve (or a skipped approve "
                            "on an unsigned run only)")
        if any(restores_cache(step) for step in steps(job)):
            problems.append(f"{name}: uses release and restores a cache")
    if "permissions" in job:
        allowed = PERMISSIONS.get(name)
        if allowed is None:
            problems.append(f"{name}: widens permissions")
        elif job["permissions"] != allowed:
            problems.append(f"{name}: permissions {job['permissions']} aren't {allowed}")
    if name in CHANNELS and not guarded(job.get("if")):
        problems.append(f"{name}: a channel job that doesn't run only when publishing")
    for step in steps(job):
        if writes_release(step) and not (guarded(step.get("if")) or guarded(job.get("if"))):
            problems.append(f"{name}: step '{step.get('name', step.get('uses'))}' writes the release or attests "
                            "without being publishing-only")
    return problems


def check_publish(publish):
    problems = []
    env = publish.get("env") or {}
    lines = runs(publish).splitlines()
    for manifest in (*PLATFORMS, "SUMS"):
        if manifest not in env:
            problems.append(f"publish: no {manifest} manifest")
    for verb in ("ids", "check"):
        line = next((line for line in lines if f"release-artifacts.py {verb}" in line), "")
        missing = [m for m in (*PLATFORMS, "SUMS") if f"${m}" not in line]
        if missing:
            problems.append(f"publish: `release-artifacts.py {verb}` leaves out {', '.join(missing)}")
    assets = next((line for line in lines if "release-artifacts.py assets" in line), "")
    missing = [m for m in PLATFORMS if f"${m}" not in assets]
    if missing:
        problems.append(f"publish: the release asset list leaves out {', '.join(missing)}")
    upload = next((line for line in lines if "gh release upload" in line), "")
    if not upload:
        problems.append("publish: no `gh release upload`")
    else:
        arguments = upload.split()
        if '"${assets[@]}"' not in arguments:
            problems.append("publish: the upload doesn't send the release assets")
        if "SHA256SUMS" not in arguments:
            problems.append("publish: the upload doesn't send SHA256SUMS")
        if '"${signature[@]}"' not in arguments and "SHA256SUMS.minisig" not in arguments:
            problems.append("publish: the upload doesn't send SHA256SUMS.minisig")
    return problems


def check(workflow):
    """The list of violations, empty when the workflow keeps every invariant."""
    problems = []
    if any(SECRETS.search(str(value)) for value in (workflow.get("env") or {}).values()):
        problems.append("workflow env: references a secret")
    if workflow.get("permissions") != {"contents": "read"}:
        problems.append("workflow permissions: must be exactly contents: read")
    jobs = workflow.get("jobs") or {}
    for name, job in jobs.items():
        problems += check_job(name, job)
    if "publish" in jobs:
        problems += check_publish(jobs["publish"])
    else:
        problems.append("publish: missing")
    return problems


def load(path):
    with open(path) as source:
        return yaml.safe_load(source)


def self_test():
    workflow = load(WORKFLOW)
    problems = check(workflow)
    assert not problems, "release.yml:\n  " + "\n  ".join(problems)

    def caught(expected, change):
        broken = copy.deepcopy(workflow)
        change(broken["jobs"], broken)
        found = check(broken)
        assert any(expected in problem for problem in found), (expected, found)

    def edit_run(job, needle, old, new):
        """Replace OLD with NEW in the one run block of JOB that contains NEEDLE."""
        for step in job["steps"]:
            if needle in step.get("run", ""):
                assert old in step["run"], (needle, old)
                step["run"] = step["run"].replace(old, new)
                return
        raise AssertionError(f"no run block with {needle!r}")

    def add_run(job, command):
        job["steps"].append({"run": command})

    # builders and signers
    caught("builds, yet uses an environment", lambda j, w: j["android-build"].update(environment="release"))
    caught("without needing approve", lambda j, w: j["checksums"].update(environment="Release"))
    caught("without a condition that requires approve", lambda j, w: j["android-sign"].update(environment="release"))
    caught("without a condition that requires approve", lambda j, w: j["macos-sign"].update(environment="release"))
    caught("without a condition that requires approve", lambda j, w: j["android-sign"].update(
        environment="${{ needs.version.outputs.signed_run == 'true' && 'release' || 'release' }}"))
    caught("without needing approve", lambda j, w: j["android-sign"].update(needs=["version", "android-build"]))
    caught("without a condition that requires approve",
           lambda j, w: j["publish"].update({"if": "needs.version.outputs.signed_run == 'true'"}))
    caught("without a condition that requires approve", lambda j, w: j["macos-sign"].update(
        {"if": "needs.approve.result == 'success' || needs.approve.result == 'skipped'"}))
    caught("without a condition that requires approve", lambda j, w: j["macos-sign"].update(
        {"if": "!cancelled() && (needs.approve.result == 'success' || needs.approve.result != 'failure')"}))
    caught("without a condition that requires approve", lambda j, w: j["android-sign"].update(
        {"if": "needs.approve.result == 'success' && needs.android-build.result == 'success' || always()"}))
    caught("restores a cache", lambda j, w: j["android-sign"]["steps"].insert(0, {"uses": "actions/cache@abc"}))
    caught("restores a cache", lambda j, w: j["macos-sign"]["steps"].insert(
        0, {"uses": "actions/setup-python@abc", "with": {"python-version": "3.13", "cache": "pip"}}))
    caught("restores a cache", lambda j, w: j["publish"]["steps"].insert(0, {"uses": "actions/setup-go@abc"}))
    caught("builds, yet uses an environment", lambda j, w: add_run(j["publish"], "cargo build --release"))
    caught("builds, yet uses an environment", lambda j, w: add_run(j["macos-sign"], "scripts/build-ghostty-vt.sh"))
    # secrets
    caught("references a secret at job level",
           lambda j, w: j["checksums"].setdefault("env", {}).update(KEY="${{ secrets.X }}"))
    caught("references a secret at job level",
           lambda j, w: j["checksums"].setdefault("env", {}).update(KEY="${{ secrets['X'] }}"))
    caught("references a secret at job level",
           lambda j, w: j["checksums"].setdefault("env", {}).update(ALL="${{ toJSON(secrets) }}"))
    caught("passes secrets at job level", lambda j, w: j["checksums"].update(secrets="inherit"))
    caught("references a secret at job level",
           lambda j, w: j["publish"].setdefault("env", {}).update(KEY="${{ SECRETS.MINISIGN_SECRET_KEY }}"))
    caught("workflow env: references a secret", lambda j, w: w.setdefault("env", {}).update(K="${{ secrets.X }}"))
    # permissions
    caught("widens permissions", lambda j, w: j["checksums"].update(permissions={"contents": "write"}))
    caught("aren't", lambda j, w: j["publish"].update(permissions={"contents": "write", "packages": "write"}))
    caught("workflow permissions", lambda j, w: w.update(permissions={"contents": "write"}))
    # publishing only
    caught("doesn't run only when publishing", lambda j, w: j.update(
        aur={"needs": ["publish"], "if": "needs.version.outputs.signed_run == 'true'", "steps": [{"run": "true"}]}))
    caught("doesn't run only when publishing", lambda j, w: j.update(
        aur={"needs": ["publish"], "steps": [{"run": "true"}],
             "if": "needs.version.outputs.publishing == 'true' || needs.version.outputs.rehearsal == 'true'"}))

    def release_step_if(new):
        def change(j, w):
            for step in j["publish"]["steps"]:
                if "gh release" in step.get("run", ""):
                    if new is None:
                        step.pop("if", None)
                    else:
                        step["if"] = new
        return change
    caught("writes the release or attests", release_step_if(None))
    caught("writes the release or attests",
           release_step_if(f"{PUBLISHING} || needs.version.outputs.rehearsal == 'true'"))
    caught("writes the release or attests",
           lambda j, w: j["checksums"]["steps"].append({"uses": "softprops/action-gh-release@abc"}))
    caught("writes the release or attests",
           lambda j, w: add_run(j["checksums"], "gh api -X PATCH repos/o/r/releases/1 -f draft=false"))
    caught("writes the release or attests", lambda j, w: add_run(
        j["checksums"], "gh api \\\n  --method PATCH \\\n  /repos/o/r/releases/1 \\\n  -F draft=false"))
    caught("writes the release or attests",
           lambda j, w: add_run(j["checksums"], 'gh \\\n  release create "$RELEASE_TAG" --verify-tag'))
    caught("writes the release or attests",
           lambda j, w: add_run(j["checksums"], 'gh -R "$GH_REPO" release edit "$RELEASE_TAG" --draft=false'))
    caught("writes the release or attests",
           lambda j, w: add_run(j["checksums"], 'gh --repo o/r api -X PATCH /repos/o/r/releases/1'))
    # the release
    caught("publish: missing", lambda j, w: j.pop("publish"))
    caught("no IOS manifest", lambda j, w: j["publish"]["env"].pop("IOS"))
    caught("`release-artifacts.py ids` leaves out SUMS",
           lambda j, w: edit_run(j["publish"], "release-artifacts.py ids", ' "$SUMS"', ""))
    caught("`release-artifacts.py check` leaves out ANDROID",
           lambda j, w: edit_run(j["publish"], "release-artifacts.py check", ' "$ANDROID"', ""))
    caught("asset list leaves out ANDROID",
           lambda j, w: edit_run(j["publish"], "release-artifacts.py assets", ' "$ANDROID"', ""))
    caught("doesn't send SHA256SUMS.minisig",
           lambda j, w: edit_run(j["publish"], "gh release upload", ' "${signature[@]}"', ""))
    caught("doesn't send SHA256SUMS",
           lambda j, w: edit_run(j["publish"], "gh release upload", " SHA256SUMS ", " "))
    caught("doesn't send the release assets",
           lambda j, w: edit_run(j["publish"], "gh release upload", ' "${assets[@]}"', ""))
    print("check-release-workflow self-test: ok")


if __name__ == "__main__":
    if sys.argv[1:] == ["--self-test"]:
        self_test()
    else:
        if len(sys.argv) > 2:
            sys.exit("usage: scripts/check-release-workflow.py [WORKFLOW] | --self-test")
        path = Path(sys.argv[1]) if len(sys.argv) == 2 else WORKFLOW
        found = check(load(path))
        for problem in found:
            print(f"::error::{path.name}: {problem}", file=sys.stderr)
        if found:
            sys.exit(1)
        print(f"{path.name}: every release invariant holds")
