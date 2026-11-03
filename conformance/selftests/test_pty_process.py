from pathlib import Path

from agterm_conformance.pty_process import PtyProcess
from agterm_conformance.wait import wait_until


def _script(tmp_path: Path, body: str) -> Path:
    path = tmp_path / "prog"
    path.write_text("#!/bin/sh\n" + body)
    path.chmod(0o755)
    return path


def _printed(process: PtyProcess, text: str) -> None:
    wait_until(lambda: text in process.output() or process.process.poll() is not None, timeout=5)
    assert text in process.output(), process.output()


def test_pty_process_runs_on_a_sized_terminal(tmp_path: Path) -> None:
    program = _script(tmp_path, '[ -t 0 ] && [ -t 1 ] && echo "tty $(stty size)"; exec cat\n')
    with PtyProcess([str(program)], {"PATH": "/usr/bin:/bin"}, columns=100, rows=30) as process:
        _printed(process, "tty 30 100")
        assert process.process.poll() is None
    assert process.process.returncode is not None


def test_pty_process_drains_output_so_the_program_never_blocks(tmp_path: Path) -> None:
    program = _script(tmp_path, "head -c 300000 /dev/zero | tr '\\0' x; echo; echo drained; exec cat\n")
    with PtyProcess([str(program)], {"PATH": "/usr/bin:/bin"}) as process:
        _printed(process, "drained")


def test_pty_process_stop_kills_a_program_that_ignores_sigterm(tmp_path: Path) -> None:
    program = _script(tmp_path, "trap '' TERM; echo armed; read line\n")
    process = PtyProcess([str(program)], {"PATH": "/usr/bin:/bin"})
    _printed(process, "armed")
    process.stop(timeout=0.5)
    assert process.process.returncode == -9
    process.stop()
