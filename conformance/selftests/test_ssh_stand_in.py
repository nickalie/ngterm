"""The `ssh` stand-in a viewer takes first on its PATH (agterm_conformance/ssh_stand_in.py): it notes each call, answers
a remote command from the answers set when the call runs, and fails any other as an unreachable host does."""

import os
import shutil
import subprocess

from agterm_conformance.ssh_stand_in import SshStandIn


def _ssh(stand_in, *words):
    program = shutil.which("ssh", path=os.fspath(stand_in.bin))
    assert program is not None, sorted(os.listdir(stand_in.bin))
    return subprocess.run([program, *words], capture_output=True, text=True, timeout=60, check=False)


def test_a_call_with_no_answer_fails_as_an_unreachable_host_and_is_noted(tmp_path):
    stand_in = SshStandIn(tmp_path)
    result = _ssh(stand_in, "-T", "-o", "BatchMode=yes", "box", "ngtermctl version")
    assert (result.returncode, result.stdout) == (255, "")
    assert result.stderr.strip() == "ssh: connect to host box port 22: Connection refused"
    assert stand_in.invocations() == [["-T", "-o", "BatchMode=yes", "box", "ngtermctl version"]]


def test_the_first_answer_whose_words_the_remote_command_holds_wins(tmp_path):
    stand_in = SshStandIn(tmp_path)
    stand_in.answer(("catalog subscribe", "", 0), ("version", '{"ok": true}\n', 0), ("ngtermctl", "any\n", 3))
    assert _ssh(stand_in, "box", "ngtermctl version").stdout == '{"ok": true}\n'
    assert _ssh(stand_in, "box", "ngtermctl catalog subscribe").returncode == 0
    other = _ssh(stand_in, "box", "ngtermctl tree")
    assert (other.returncode, other.stdout) == (3, "any\n")


def test_answers_set_later_reach_the_next_call(tmp_path):
    stand_in = SshStandIn(tmp_path)
    stand_in.answer(("version", "first\n", 0))
    assert _ssh(stand_in, "box", "version").stdout == "first\n"
    stand_in.answer()
    assert _ssh(stand_in, "box", "version").returncode == 255
    assert len(stand_in.invocations()) == 2
