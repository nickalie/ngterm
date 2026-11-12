"""scripts/build-ghostty-vt.sh's protection from its downloads' hosts: the key of the Zig package mirror, and the time
limit on a fetch. Each case runs the real script, or its real functions, over a copy of the scripts, pins and patches;
nothing is fetched."""

import os
import re
import shutil
import stat
import subprocess
import time
from collections.abc import Callable
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
SCRIPT = "scripts/build-ghostty-vt.sh"


@pytest.fixture
def checkout(tmp_path: Path) -> Path:
    """The script, its library, the pins and the patches, as a repository of their own."""
    root = tmp_path / "repo"
    (root / "scripts" / "lib").mkdir(parents=True)
    shutil.copy2(REPO / SCRIPT, root / SCRIPT)
    shutil.copy2(REPO / "scripts/lib/native.sh", root / "scripts/lib/native.sh")
    shutil.copytree(REPO / "build", root / "build")
    return root


def environment(checkout: Path, **env: str) -> dict[str, str]:
    base = {name: value for name, value in os.environ.items() if not name.startswith(("NGTERM_", "AGTERM_"))}
    base.update(NGTERM_VENDOR=str(checkout.parent / "vendor"), **env)
    return base


def printed(checkout: Path, option: str, **env: str) -> str:
    done = subprocess.run(
        ["bash", str(checkout / SCRIPT), option],
        env=environment(checkout, **env),
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert done.returncode == 0, done.stderr
    return done.stdout.strip()


def key(checkout: Path, **env: str) -> str:
    return printed(checkout, "--print-zig-packages-key", **env)


def edit(path: Path, change: Callable[[str], str]) -> None:
    before = path.read_text()
    after = change(before)
    assert after != before, f"the edit changed nothing in {path}"
    path.write_text(after)


def test_the_mirror_key_moves_with_a_ghostty_pin(checkout: Path) -> None:
    first = key(checkout)
    edit(
        checkout / "build/pins.env",
        lambda text: re.sub(r'(?m)^LIBGHOSTTY_VT_REV="[^"]*"', 'LIBGHOSTTY_VT_REV="other"', text),
    )
    assert key(checkout) != first


def test_the_mirror_key_moves_with_a_patch(checkout: Path) -> None:
    first = key(checkout)
    (checkout / "build/patches/ghostty-vt/9999-added.patch").write_text("added\n")
    assert key(checkout) != first


def test_the_mirror_key_moves_with_the_targets(checkout: Path) -> None:
    first = key(checkout)
    edit(
        checkout / SCRIPT,
        lambda text: text.replace("ZIG_TARGETS=(", "ZIG_TARGETS=(riscv64-linux-gnu ", 1),
    )
    assert key(checkout) != first


def test_the_mirror_key_stays_with_another_builds_pin(checkout: Path) -> None:
    first = key(checkout)
    edit(checkout / "build/pins.env", lambda text: re.sub(r'(?m)^WPE_VERSION="[^"]*"', 'WPE_VERSION="other"', text))
    assert key(checkout) == first


def test_the_mirror_key_does_not_depend_on_the_host(checkout: Path, tmp_path: Path) -> None:
    shims = tmp_path / "shims"
    shims.mkdir()
    uname = shims / "uname"
    real = shutil.which("uname")
    uname.write_text(f'#!/bin/sh\n[ "$1" = -m ] && {{ echo riscv64; exit 0; }}\nexec {real} "$@"\n')
    uname.chmod(uname.stat().st_mode | stat.S_IEXEC)
    other_host = {"PATH": f"{shims}{os.pathsep}{os.environ['PATH']}"}
    # the prefix key does read the arch, so this shows the shim works
    assert printed(checkout, "--print-prefix", **other_host) != printed(checkout, "--print-prefix")
    assert key(checkout, **other_host) == key(checkout)


def functions(*names: str) -> str:
    """The real text of the script's functions `names`."""
    text = (REPO / SCRIPT).read_text()
    found = []
    for name in names:
        match = re.search(rf"(?ms)^{name}\(\) \{{\n.*?^\}}\n", text)
        assert match, name
        found.append(match.group(0))
    return "".join(found)


def bash(body: str, timeout: float = 30, **env: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", f"set -euo pipefail\n{body}"],
        env={**os.environ, **env},
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )


def test_run_bounded_stops_a_command_past_its_limit() -> None:
    started = time.monotonic()
    done = bash(functions("run_bounded") + 'status=0; run_bounded 1 sleep 25 || status=$?; echo "$status"')
    assert done.stdout.strip() == "143", done.stderr
    assert time.monotonic() - started < 15
    assert "still running after 1s" in done.stderr


def test_run_bounded_returns_a_commands_own_status_at_once() -> None:
    started = time.monotonic()
    done = bash(
        functions("run_bounded")
        + 'fails() { return 7; }\nstatus=0; run_bounded 25 fails || status=$?; echo "$status"\n'
        + "run_bounded 25 true; echo ok"
    )
    assert done.stdout.split() == ["7", "ok"], done.stderr
    assert time.monotonic() - started < 15


def test_a_stalled_zig_itself_is_stopped(tmp_path: Path) -> None:
    """The signal has to reach zig: a shell left between them would die and leave the stalled zig running."""
    pid_file = tmp_path / "zig.pid"
    zig = tmp_path / "zig"
    zig.write_text(f'#!/bin/sh\necho $$ > "{pid_file}"\nexec sleep 25\n')
    zig.chmod(zig.stat().st_mode | stat.S_IEXEC)
    (tmp_path / "tree").mkdir()
    (tmp_path / "build").mkdir()
    done = bash(
        "native_path() { echo \"$1\"; }\n"
        + functions("run_bounded", "configure_for_target")
        + f'TREE="{tmp_path}/tree"; BUILD_DIR="{tmp_path}/build"; ZIG_CACHE="{tmp_path}/cache"; ZIG_EXE="{zig}"\n'
        + 'status=0; run_bounded 1 configure_for_target x86_64-linux-gnu || status=$?; echo "$status"'
    )
    assert done.stdout.strip() == "143", done.stderr
    pid = int(pid_file.read_text())
    with pytest.raises(ProcessLookupError):
        os.kill(pid, 0)


ZIG_0_17_FETCH = (
    '#!/bin/sh\nname="$(basename "$2" .tar.gz)"\nmkdir -p "$ZIG_GLOBAL_CACHE_DIR/p"\n'
    'echo rewrapped > "$ZIG_GLOBAL_CACHE_DIR/p/$name.tar.gz"\necho "$name"\n'
)


def seed(
    tmp_path: Path, packages: dict[str, bytes], zig_script: str, prelude: str = ""
) -> tuple[subprocess.CompletedProcess[str], Path]:
    """seed_zig_packages over a file:// mirror of `packages`, with `zig_script` as zig and `prelude` run first."""
    stage = tmp_path / "stage"
    stage.mkdir()
    for name, content in packages.items():
        (stage / f"{name}.tar.gz").write_bytes(content)
    release = tmp_path / "releases" / "zig-packages-k"
    release.mkdir(parents=True)
    subprocess.run(["tar", "-cf", str(release / "zig-packages.tar"), "-C", str(stage), "."], check=True)
    zig = tmp_path / "zig"
    zig.write_text(zig_script)
    zig.chmod(zig.stat().st_mode | stat.S_IEXEC)
    for directory in ("tree", "build", "cache"):
        (tmp_path / directory).mkdir()
    cache = tmp_path / "cache"
    done = bash(
        'native_path() { echo "$1"; }\nzig_packages_key() { echo k; }\ndie() { echo "$*" >&2; exit 1; }\n'
        + prelude
        + functions("seed_zig_packages")
        + f'TREE="{tmp_path}/tree"; BUILD_DIR="{tmp_path}/build"; ZIG_CACHE="{cache}"; ZIG_EXE="{zig}"\n'
        + f'ZIG_PACKAGES_RELEASES="file://{tmp_path}/releases"; export ZIG_GLOBAL_CACHE_DIR="{cache}"\n'
        + "seed_zig_packages"
    )
    return done, cache


def test_the_mirror_seeds_the_global_cache_with_its_archives_as_they_are(tmp_path: Path) -> None:
    """Zig 0.17's fetch of a cache archive caches it rewrapped, which the build then hashes wrong."""
    done, cache = seed(tmp_path, {"pkg-1.0.0-AAAA": b"first", "N-V-__8AABBBB": b"second"}, ZIG_0_17_FETCH)
    assert done.returncode == 0, done.stderr
    assert (cache / "p" / "pkg-1.0.0-AAAA.tar.gz").read_bytes() == b"first"
    assert (cache / "p" / "N-V-__8AABBBB.tar.gz").read_bytes() == b"second"
    assert sorted(path.name for path in (cache / "p").iterdir()) == ["N-V-__8AABBBB.tar.gz", "pkg-1.0.0-AAAA.tar.gz"]


def test_a_copy_cut_short_leaves_no_archive_under_the_name_zig_reads(tmp_path: Path) -> None:
    cut_short = 'cp() { printf part > "$2"; return 1; }\n'
    done, cache = seed(tmp_path, {"pkg-1.0.0-AAAA": b"first"}, ZIG_0_17_FETCH, cut_short)
    assert done.returncode != 0
    assert "cannot seed pkg-1.0.0-AAAA" in done.stderr
    assert list((cache / "p").iterdir()) == []


def test_a_mirror_archive_under_another_name_seeds_nothing(tmp_path: Path) -> None:
    done, cache = seed(tmp_path, {"pkg-1.0.0-AAAA": b"first"}, "#!/bin/sh\necho pkg-1.0.0-OTHER\n")
    assert done.returncode != 0
    assert "holds pkg-1.0.0-OTHER under the name pkg-1.0.0-AAAA" in done.stderr
    assert not (cache / "p" / "pkg-1.0.0-AAAA.tar.gz").exists()
