"""App package ELF boundaries, without a release build or an installed WPE engine."""

from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess

import pytest
import yaml


REPO = Path(__file__).resolve().parents[2]
CHECK = REPO / "scripts/check-linux-package.py"
BINARIES = ("ngterm", "ngtermctl", "ngterm-session", "ngterm-headless", "ngterm-web")


def run(*args: str | Path, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run([str(arg) for arg in args], text=True, capture_output=True, **kwargs)


@pytest.fixture
def tree(tmp_path: Path) -> Path:
    for tool in ("cc", "patchelf", "ldd"):
        if not shutil.which(tool):
            pytest.skip(f"ELF fixtures require {tool}")
    root = tmp_path / "app"
    (root / "usr/bin").mkdir(parents=True)
    (root / "usr/lib").mkdir()
    for name in BINARIES:
        shutil.copy2("/bin/true", root / "usr/bin" / name)
    return root


def check(tree: Path, *args: str) -> subprocess.CompletedProcess:
    env = {name: value for name, value in os.environ.items() if not name.startswith("NGTERM_")}
    return run("python3", CHECK, *args, tree, env=env)


def link_engine(tree: Path, binary: str, soname: str) -> Path:
    # Outside the app tree: a DT_NEEDED check must catch even an unbundled engine.
    library = tree.parent / soname
    result = run("cc", "-shared", "-fPIC", "-x", "c", "-", "-o", library,
                 f"-Wl,-soname,{soname}", input="int fixture(void) { return 7; }")
    assert result.returncode == 0, result.stderr
    result = run("cc", "-x", "c", "-", "-o", tree / "usr/bin" / binary,
                 "-Wl,--no-as-needed", "-x", "none", library, input="int main(void) { return 0; }")
    assert result.returncode == 0, result.stderr
    return library


def test_app_with_separate_unresolved_engine_passes(tree: Path) -> None:
    library = link_engine(tree, "ngterm-web", "libWPEWebKit-2.0.so.1")
    library.unlink()
    result = check(tree)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("soname", ["libWPEWebKit-2.0.so.1", "libWPEPlatform-2.0.so.1",
                                   "libgstreamer-1.0.so.0", "libsoup-3.0.so.0",
                                   "libjavascriptcoregtk-4.1.so.0", "libJavaScriptCore-2.0.so.1"])
def test_terminal_must_not_link_the_engine(tree: Path, soname: str) -> None:
    link_engine(tree, "ngterm", soname)
    result = check(tree)
    assert result.returncode != 0
    assert f"ngterm needs web engine library: {soname}" in result.stderr


@pytest.mark.parametrize("binary", BINARIES)
@pytest.mark.parametrize("legacy", [False, True])
@pytest.mark.parametrize("rpath", ["$ORIGIN/../lib", "/opt/ngterm/usr/lib", "/build/wpe/lib"])
def test_binaries_must_not_have_a_library_search_path(tree: Path, binary: str, rpath: str, legacy: bool) -> None:
    args = ["patchelf", "--set-rpath", rpath, tree / "usr/bin" / binary]
    if legacy:
        args.insert(1, "--force-rpath")
    assert run(*args).returncode == 0
    result = check(tree)
    assert result.returncode != 0
    assert f"{binary} must have no RPATH or RUNPATH: {rpath}" in result.stderr


@pytest.mark.parametrize("bundled", ["usr/lib/libxkbcommon.so.0", "usr/lib/libglib-2.0.so.0", "usr/bin/helper.so"])
def test_a_bundled_library_is_rejected(tree: Path, bundled: str) -> None:
    # The app takes every library from the host: an older bundled copy is what crashed on the compositor's keymap.
    path = tree / bundled
    result = run("cc", "-shared", "-fPIC", "-x", "c", "-", "-o", path, input="int fixture(void) { return 7; }")
    assert result.returncode == 0, result.stderr
    result = check(tree)
    assert result.returncode != 0
    assert f"bundled library in app package: {bundled}" in result.stderr


def test_preparation_strips_build_search_paths_without_resolving_the_worker(tree: Path) -> None:
    worker = tree / "usr/bin/ngterm-web"
    library = link_engine(tree, "ngterm-web", "libWPEWebKit-2.0.so.1")
    library.unlink()
    assert run("patchelf", "--set-rpath", "/build/wpe/lib", worker).returncode == 0
    assert run("patchelf", "--set-rpath", "/build/target/release", tree / "usr/bin/ngterm").returncode == 0
    result = check(tree, "--prepare")
    assert result.returncode == 0, result.stderr
    assert run("patchelf", "--print-rpath", worker).stdout.strip() == ""
    assert run("patchelf", "--print-needed", worker).stdout.splitlines()[0] == "libWPEWebKit-2.0.so.1"
    for name in BINARIES:
        assert run("patchelf", "--print-rpath", tree / "usr/bin" / name).stdout.strip() == ""
    assert check(tree).returncode == 0


@pytest.mark.parametrize("payload", ["usr/lib/libWPEWebKit-2.0.so.1", "usr/lib/libgstapp-1.0.so.0",
                                    "usr/lib/libsoup-3.0.so.0", "usr/lib/gstreamer-1.0/plugin.so",
                                    "usr/lib/wpe-webkit-2.0/WPEWebProcess", "usr/share/ngterm/bundled-wpe"])
def test_engine_payload_cannot_enter_the_app_tree(tree: Path, payload: str) -> None:
    path = tree / payload
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"fixture")
    result = check(tree)
    assert result.returncode != 0
    assert "web engine payload in app package:" in result.stderr


def test_missing_worker_is_rejected(tree: Path) -> None:
    (tree / "usr/bin/ngterm-web").unlink()
    result = check(tree)
    assert result.returncode != 0 and "missing app binary: ngterm-web" in result.stderr


def test_native_packages_install_host_tools_and_offer_system_playback() -> None:
    packages = yaml.safe_load((REPO / "packaging/linux/nfpm.yaml").read_text())["overrides"]
    for package in packages.values():
        assert {"bubblewrap", "xdg-dbus-proxy", "ca-certificates"} <= set(package["depends"])
    deb = packages["deb"]
    assert set(deb["recommends"]) == {"libwpewebkit-2.0-1 (>= 2.54)", "gstreamer1.0-plugins-base",
                                      "gstreamer1.0-plugins-good", "gstreamer1.0-plugins-bad",
                                      "gstreamer1.0-libav", "gstreamer1.0-gl", "gstreamer1.0-alsa",
                                      "gstreamer1.0-pulseaudio"}
    rpm = packages["rpm"]
    assert set(rpm["recommends"]) == {"libWPEWebKit-2_0-1 >= 2.54", "gstreamer-plugins-base",
                                      "gstreamer-plugins-good", "gstreamer-plugins-bad", "gstreamer-plugins-libav"}
    for package in (deb, rpm):
        assert not set(package["depends"]) & set(package["recommends"])
    assert {"wpewebkit>=2.54", "gst-plugins-base", "gst-plugins-good", "gst-plugins-bad", "gst-libav"} <= set(
        packages["archlinux"]["depends"])


def test_rpm_requires_every_host_library() -> None:
    # The rpm names host libraries by SONAME, so it must list each one the app and the runtime take from the host.
    packages = yaml.safe_load((REPO / "packaging/linux/nfpm.yaml").read_text())["overrides"]
    required = packages["rpm"]["depends"]
    hosts = [line for line in (REPO / "packaging/linux/runtime-libraries.txt").read_text().splitlines()
             if line and not line.startswith("#")]
    assert {"libxkbcommon.so.0", "libglib-2.0.so.0", "libgio-2.0.so.0"} <= set(hosts)
    missing = [name for name in hosts if not any(entry.startswith(f"{name}(") for entry in required)]
    assert not missing, missing
