"""LLVM coverage profiles of an instrumented stage (`run-linux.sh --coverage`, `run-windows.sh --coverage`).

The runner names a profile directory in `CONFORMANCE_COVERAGE_DIR` (the coverage build's target dir, whose
`*.profraw` files `cargo llvm-cov report` reads). Under it every process the harness starts from the build under test
(the app through the adapter, `ngtermctl` through `Cli` and `CliProcess`, `ngterm-session attach` through
`live_client`) gets `LLVM_PROFILE_FILE` there, pooled per binary: `%4m` names one of four files per binary signature,
merged online, so thousands of `ngtermctl` calls leave a handful of files. What those processes start (the session
server) inherits it. Without the variable an inherited `LLVM_PROFILE_FILE` is dropped, so an instance never writes
into a directory the run did not name.

A process that ends past its own exit (a signal death, `TerminateProcess`, a kill) writes no profile. Each one the
harness ends that way is recorded in `lost-profiles.txt` in the profile directory, one line per process, so a
coverage drop can be told from lost data. Only the build's own programs are recorded (`INSTRUMENTED_PROGRAMS`):
nothing else writes a profile.

`python -m agterm_conformance.llvm_profiles BIN_DIR` exits 2 when a staged `bin/` holds no instrumented app.
"""

from __future__ import annotations

import mmap
import os
import subprocess
import sys
import threading
from collections.abc import Iterable, Mapping, MutableMapping
from pathlib import Path
from typing import Any

COVERAGE_DIR_VARIABLE = "CONFORMANCE_COVERAGE_DIR"
PROFILE_VARIABLE = "LLVM_PROFILE_FILE"
# `%4m`: a pool of four files per binary signature, merged online by the profile runtime (clang's
# "Running the instrumented program" docs).
PROFILE_NAME = "conformance-%4m.profraw"
LOST_FILE = "lost-profiles.txt"
# The programs of the build under test, which write profiles when instrumented; on Windows with `.exe`.
INSTRUMENTED_PROGRAMS = frozenset({"ngterm", "ngtermctl", "ngterm-session", "ngterm-headless"})
# The app the stage must hold instrumented.
APP_PROGRAM = "ngterm"
# compiler-rt's profile runtime's message prefix (`PROF_ERR`), in every binary linked with `-C instrument-coverage`.
RUNTIME_MARKER = b"LLVM Profile Error"

_lost_lock = threading.Lock()


class NotInstrumented(Exception):
    """A stage under coverage whose binaries carry no profile runtime: its run would write no profile."""


def profile_dir(environ: Mapping[str, str] | None = None) -> Path | None:
    """The run's profile directory, absolute (against the harness's working directory: the processes given it run from
    their instance homes), or None when the run measures no coverage."""
    value = (os.environ if environ is None else environ).get(COVERAGE_DIR_VARIABLE)
    return Path(os.path.abspath(value)) if value else None


def apply(env: MutableMapping[str, str], environ: Mapping[str, str] | None = None) -> MutableMapping[str, str]:
    """Points `env`'s `LLVM_PROFILE_FILE` at the run's pooled profile, or drops it when the run measures none."""
    env.pop(PROFILE_VARIABLE, None)
    if (directory := profile_dir(environ)) is not None:
        env[PROFILE_VARIABLE] = os.fspath(directory / PROFILE_NAME)
    return env


def with_profile(env: Mapping[str, str], environ: Mapping[str, str] | None = None) -> dict[str, str]:
    """A copy of `env` with `apply`'s profile variable."""
    copy = dict(env)
    apply(copy, environ)
    return copy


def program_name(path: str | os.PathLike[str]) -> str:
    """The program's name without its directory or Windows' `.exe`."""
    name = os.path.basename(os.fspath(path))
    return name[: -len(".exe")] if name.lower().endswith(".exe") else name


def record_lost(
    pid: int | None, program: str | os.PathLike[str], how: str, environ: Mapping[str, str] | None = None
) -> None:
    """Records that the harness ended `program`'s process `pid` (None: not known) past its exit (`how`), under coverage
    only, and only for the build's own programs."""
    directory = profile_dir(environ)
    name = program_name(program)
    if directory is None or name not in INSTRUMENTED_PROGRAMS:
        return
    line = f"{'?' if pid is None else pid} {name}: {how}\n"
    with _lost_lock:
        directory.mkdir(parents=True, exist_ok=True)
        with open(directory / LOST_FILE, "a", encoding="utf-8") as sink:
            sink.write(line)


def terminate(process: subprocess.Popen[Any]) -> bool:
    """`process.terminate()`; whether it reached a running process. Popen skips one that has already exited and takes
    its exit code (on Windows from `TerminateProcess`'s refusal), so a `returncode` set here means the process exited
    by itself, writing its profile."""
    process.terminate()
    return process.returncode is None


def record_if_signalled(
    pid: int, program: str | os.PathLike[str], returncode: int | None, how: str, *, terminated: bool
) -> None:
    """Records a process the harness ended (`terminated`: its signal or `TerminateProcess` reached it running). On
    POSIX only one that died of the signal (a negative `returncode`): one that handled it and exited with a code wrote
    its profile. On Windows every `TerminateProcess` that took effect: recorded."""
    if terminated and (sys.platform == "win32" or (returncode is not None and returncode < 0)):
        record_lost(pid, program, how)


def is_instrumented(binary: Path) -> bool:
    """Whether `binary` carries the profile runtime."""
    with open(binary, "rb") as source:
        if os.fstat(source.fileno()).st_size == 0:
            return False
        with mmap.mmap(source.fileno(), 0, access=mmap.ACCESS_READ) as mapped:
            return mapped.find(RUNTIME_MARKER) != -1


def check_instrumented(binaries: Iterable[Path]) -> None:
    """Refuses a coverage run on binaries of the build that are not instrumented: the app must be among them, and every
    one of the build's programs given must carry the profile runtime."""
    found = [Path(binary) for binary in binaries if program_name(binary) in INSTRUMENTED_PROGRAMS]
    if not any(program_name(binary) == APP_PROGRAM for binary in found):
        raise NotInstrumented(f"no {APP_PROGRAM} binary to measure coverage of")
    plain = [os.fspath(binary) for binary in found if not is_instrumented(binary)]
    if plain:
        raise NotInstrumented(
            f"not built with coverage (no LLVM profile runtime): {', '.join(plain)}; build the stage with "
            "cargo llvm-cov show-env (the runner's --coverage)"
        )


def stage_binaries(bin_dir: Path) -> list[Path]:
    """The build's programs in a staged `bin/`."""
    if not bin_dir.is_dir():
        return []
    return sorted(
        entry for entry in bin_dir.iterdir() if entry.is_file() and program_name(entry) in INSTRUMENTED_PROGRAMS
    )


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: python -m agterm_conformance.llvm_profiles BIN_DIR", file=sys.stderr)
        return 2
    try:
        check_instrumented(stage_binaries(Path(argv[0])))
    except (NotInstrumented, OSError) as error:
        print(f"coverage: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
