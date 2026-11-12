"""A Windows viewer attaching a Mac that runs upstream agterm (`agterm-peer`): its panes run ssh.exe through `ngtermctl
viewer-run`, with no `/bin/sh` and no zmx on this machine, and hand the Mac the same attach line a Unix viewer does.

Windows runs no unprivileged sshd, so the far side is an `ssh.exe` stand-in first on the instance's PATH (distlib's
script launcher over Python, as `tests/cli/test_cli_only.py`'s): it records each argv and answers as agterm v0.41.1's
Mac does over ssh: the listing under its marker line (`fake_agterm`'s shape), a presentation stream that waits for the
viewer to hang up, and an attach that prints the daemon it reached and waits for input."""

import json
import os
import re
import shlex
import sys
import uuid

import pytest
from agterm_conformance.match import find_session
from agterm_conformance.plugin import Cli, Ctl, launch_options
from agterm_conformance.wait import wait_text, wait_until

pytestmark = [
    pytest.mark.requires(
        "agterm-peer",
        reason="attaching an upstream agterm Mac is the Rust build's extension; agterm attaches only agterm",
    ),
    # Windows alone claims it, so elsewhere the test skips as the build's divergence, which `--must-pass agterm-peer`
    # excuses (a plain platform skip would fail it)
    pytest.mark.requires(
        "cmd-command-line", reason="a Unix viewer runs a remote pane's attach as a /bin/sh line, not through viewer-run"
    ),
]

GONE_GUARD = "printf '%s\\n' 'ngterm: remote session is gone'; exit 1"
ZMX = "/Applications/agterm.app/Contents/MacOS/zmx"
ZMX_DIR = "/tmp/agterm-zmx-501"

_STAND_IN = r"""
import json, sys
record, listing = {record!r}, {listing!r}
argv = sys.argv[1:]
with open(record, "a", encoding="utf-8") as out:
    out.write(json.dumps(argv) + "\n")
command = argv[-1] if argv else ""
if "zmx tree" in command:
    sys.stdout.write("ngterm-peer: agterm\n" + listing + "\n")
    sys.exit(0)
if "zmx present" in command:
    for line in sys.stdin:
        pass
    sys.exit(0)
if " attach " in command:
    daemon = command.split(" attach ")[1].split()[0].strip("'")
    sys.stdout.write("attached " + daemon + "\r\n")
    sys.stdout.flush()
    sys.stdin.readline()
    sys.exit(0)
sys.exit(255)
"""


@pytest.fixture
def agterm_ssh(tmp_path):
    from distlib.scripts import ScriptMaker

    session = str(uuid.uuid4()).upper()
    daemon = f"agterm-{uuid.uuid4().hex}"
    listing = json.dumps(
        {
            "ok": True,
            "result": {
                "remote": {
                    "endpoint": {"executable": ZMX, "socketDirectory": ZMX_DIR},
                    "presentation": 1,
                    "sessions": [
                        {
                            "id": session,
                            "name": "build",
                            "windowID": str(uuid.uuid4()).upper(),
                            "windowName": "Main",
                            "workspaceID": str(uuid.uuid4()).upper(),
                            "workspaceName": "work",
                            "cwd": "/Users/me/repo",
                            "panes": [{"pane": "left", "daemon": daemon}],
                        }
                    ],
                }
            },
        },
        separators=(",", ":"),
    )
    folder = tmp_path / "ssh"
    source = tmp_path / "source"
    folder.mkdir()
    source.mkdir()
    record = tmp_path / "argv.jsonl"
    (source / "ssh.py").write_text(_STAND_IN.format(record=os.fspath(record), listing=listing), encoding="utf-8")
    maker = ScriptMaker(os.fspath(source), os.fspath(folder))
    maker.executable = sys.executable
    maker.make("ssh.py")
    assert (folder / "ssh.exe").is_file(), sorted(os.listdir(folder))

    def calls():
        try:
            return [json.loads(line) for line in record.read_text(encoding="utf-8").splitlines()]
        except FileNotFoundError:
            return []

    return folder, session, daemon, calls


def test_a_windows_viewer_attaches_an_agterm_pane_through_ssh_exe(adapter, request, agterm_ssh):
    folder, session, daemon, calls = agterm_ssh
    instance = adapter.launch(path_prefix=[folder], **launch_options(request))
    try:
        ctl = Ctl(instance)
        cli = Cli(adapter.ctl_path, instance, adapter.ctl_name)
        listed = cli("live", "tree", "agterm:studio", "--json")
        assert listed.returncode == 0, listed.stderr
        assert listed.json()["result"]["remote"]["endpoint"]["peer"] == "agterm"

        attached = cli("live", "attach", "agterm:studio", session)
        assert attached.returncode == 0, attached.stderr
        local = attached.stdout.strip()
        assert find_session(ctl.tree(), local)["remoteHost"] == "agterm:studio"
        wait_text(ctl, local, re.escape(f"attached {daemon}"), timeout=30)

        attach = wait_until(lambda: next((argv for argv in calls() if f" attach {daemon} " in argv[-1]), None))
        assert attach[0] == "-tt" and "studio" in attach and "agterm:studio" not in attach, attach
        assert "LogLevel=ERROR" in attach, attach
        words = shlex.split(attach[-1])
        assert words[0] == "/usr/bin/env", words
        assignments = dict(word.split("=", 1) for word in words[1:] if "=" in word and not word.startswith("/"))
        assert assignments["ZMX_SESSION"] == "" and assignments["ZMX_SESSION_PREFIX"] == "", words
        assert assignments["ZMX_NO_DETACH_KEY"] == "1" and assignments["ZMX_DIR"] == ZMX_DIR, words
        assert assignments.get("ZMX_MANAGED"), words
        assert not any(name.startswith("NGTERM_") for name in assignments), words
        assert words[-6:] == [ZMX, "attach", daemon, "/bin/sh", "-c", GONE_GUARD], words

        present = wait_until(lambda: next((argv for argv in calls() if "zmx present" in argv[-1]), None))
        assert present[0] == "-T" and "agtermctl" in present[-1] and session in present[-1], present
    finally:
        adapter.stop(instance)
