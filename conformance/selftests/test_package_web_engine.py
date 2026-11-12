"""The runtime packager on tiny ELF fixtures, with no WPE build or GUI."""

from __future__ import annotations

import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

import pytest


REPO = Path(__file__).resolve().parents[2]


def run(*args: str | Path, **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run([str(arg) for arg in args], text=True, capture_output=True, **kwargs)


def shared(path: Path, source: str = "int fixture(void) { return 7; }", *flags: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    result = run("cc", "-shared", "-fPIC", "-x", "c", "-", "-o", path, *flags, input=source)
    assert result.returncode == 0, result.stderr


@pytest.fixture
def checkout(tmp_path: Path) -> Path:
    for tool in ("cc", "ldd", "patchelf", "zstd"):
        if not shutil.which(tool):
            pytest.skip(f"ELF fixture tests require {tool}")
    root = tmp_path / "checkout"
    for name in ("scripts/package-web-engine.sh", "scripts/web-engine-closure.py", "scripts/lib/native.sh",
                 "scripts/build-release-wpe.sh", "scripts/release-linux-deps.sh", "build/pins.env",
                 "packaging/release/pins.env", "packaging/release/relocatable-wpe.patch",
                 "packaging/linux/gstreamer-plugins.txt", "packaging/linux/runtime-libraries.txt"):
        target = root / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / name, target)
    (root / "packaging/linux/gstreamer-plugins.txt").write_text("playback\nlibav\n")
    prefix = root / "wpe"
    library = prefix / "lib"
    shared(library / "libWPEWebKit-2.0.so.1.14.0")
    (library / "libWPEWebKit-2.0.so.1").symlink_to("libWPEWebKit-2.0.so.1.14.0")
    (library / "libWPEWebKit-2.0.so").symlink_to("libWPEWebKit-2.0.so.1")
    helpers = library / "wpe-webkit-2.0"
    helpers.mkdir()
    for name in ("WPEWebProcess", "WPENetworkProcess"):
        shutil.copy2("/bin/true", helpers / name)
    shared(helpers / "injected-bundle/libWPEInjectedBundle.so")
    docs = prefix / "share/doc/wpewebkit"
    docs.mkdir(parents=True)
    (docs / "LICENSE").write_text("WPE fixture license\n")
    system = root / "system/usr"
    lib = system / "lib/fixture"
    dependency = lib / "libengine-dependency.so.1.2"
    shared(dependency, "int dependency(void) { return 42; }", "-Wl,-soname,libengine-dependency.so.1")
    (lib / "libengine-dependency.so.1").symlink_to(dependency.name)
    (lib / "libengine-dependency.so").symlink_to(dependency.name)
    for name in ("playback", "libav", "offlist"):
        shared(lib / f"gstreamer-1.0/libgst{name}.so",
               "extern int dependency(void); int plugin(void) { return dependency(); }",
               f"-L{lib}", "-lengine-dependency", f"-Wl,-rpath,{lib}")
    for name in ("libgiognutls", "libgiolibproxy", "libgiognomeproxy", "libdconfsettings"):
        shared(lib / f"gio/modules/{name}.so")
    scanner = lib / "gstreamer1.0/gstreamer-1.0/gst-plugin-scanner"
    scanner.parent.mkdir(parents=True)
    shutil.copy2("/bin/true", scanner)
    (system / "share/doc/fixture-package").mkdir(parents=True)
    (system / "share/doc/fixture-package/copyright").write_text("Fixture copyright\n")
    (system / "share/common-licenses").mkdir()
    (system / "share/common-licenses/LGPL-2.1").write_text("Fixture LGPL\n")
    tools = root / "tools"
    tools.mkdir()
    (tools / "dpkg-query").write_text(
        "#!/bin/sh\ncase \"$1\" in\n"
        "-S) printf 'fixture-package: %s\\n' \"$2\" ;;\n"
        "-W) printf 'fixture-package\\t%s\\n' \"${FIXTURE_PACKAGE_VERSION:-1.0}\" ;;\nesac\n"
    )
    (tools / "dpkg-query").chmod(0o755)
    key = run("bash", root / "scripts/build-release-wpe.sh", "--print-key")
    assert key.returncode == 0, key.stderr
    (prefix / ".release-pins").write_text(key.stdout)
    return root


def package(root: Path, *args: str | Path, **environment: str) -> subprocess.CompletedProcess:
    env = {key: value for key, value in os.environ.items() if not key.startswith("NGTERM_")}
    env.update(NGTERM_RELEASE_WPE=str(root / "wpe"), PATH=f"{root / 'tools'}:{env['PATH']}")
    env.update(environment)
    return run("bash", root / "scripts/package-web-engine.sh", "--system-root", root / "system",
               "--multiarch", "fixture", *args, env=env)


def test_archive_has_only_selected_plugins_and_one_dependency_copy(checkout: Path) -> None:
    patchelf = checkout / "tools/patchelf"
    patchelf.write_text("#!/bin/sh\nexit 1\n")
    patchelf.chmod(0o755)
    before = package(checkout, "--print-key")
    assert before.returncode == 0, before.stderr
    assert len(before.stdout.strip()) == 64
    patchelf.unlink()
    result = package(checkout, checkout / "dist")
    assert result.returncode == 0, result.stderr
    archive = next((checkout / "dist").glob("*.tar.zst"))
    assert archive.name == f"ngterm-web-engine-{before.stdout.strip()}-linux-{os.uname().machine}.tar.zst"
    assert f"size={archive.stat().st_size}" in result.stdout
    assert f"sha256={hashlib.sha256(archive.read_bytes()).hexdigest()}" in result.stdout
    tar = checkout / "runtime.tar"
    with tar.open("wb") as output:
        subprocess.run(["zstd", "-dc", str(archive)], stdout=output, check=True)
    extracted = checkout / "extracted"
    with tarfile.open(tar) as contents:
        contents.extractall(extracted, filter="data")
    plugins = extracted / "lib/gstreamer-1.0"
    assert {path.name for path in plugins.iterdir()} == {"libgstplayback.so", "libgstlibav.so"}
    dependency = extracted / "lib/libengine-dependency.so.1"
    assert dependency.is_symlink() and dependency.is_file()
    assert len([path for path in extracted.rglob("libengine-dependency*") if not path.is_symlink()]) == 1
    assert (extracted / "lib/libWPEWebKit-2.0.so.1").is_file()
    assert len([path for path in extracted.rglob("libWPEWebKit*") if not path.is_symlink()]) == 1
    assert (extracted / "lib/wpe-webkit-2.0/injected-bundle/libWPEInjectedBundle.so").is_file()
    assert (extracted / "libexec/gstreamer-1.0/gst-plugin-scanner").is_file()
    assert len(list((extracted / "lib/gio/modules").glob("*.so"))) == 4
    assert (extracted / "share/doc/fixture-package/copyright").read_text() == "Fixture copyright\n"
    assert (extracted / "share/doc/wpewebkit/LICENSE").is_file()
    assert (extracted / "share/common-licenses/LGPL-2.1").is_file()
    assert (extracted / "PACKAGES").read_text() == "fixture-package\t1.0\n"
    assert "libc.so.6" in (extracted / "HOST-LIBRARIES").read_text()
    assert f"key={before.stdout.strip()}\n" in (extracted / "MANIFEST").read_text()
    assert "wpe_version=" in (extracted / "MANIFEST").read_text()
    # The original plugin search path can disappear: the bundled closure is used.
    shutil.rmtree(checkout / "system/usr/lib")
    env = {key: value for key, value in os.environ.items() if key != "LD_LIBRARY_PATH"}
    found = run("ldd", plugins / "libgstplayback.so", env=env)
    assert found.returncode == 0, found.stdout
    resolved = re.search(r"libengine-dependency.so.1 => (\S+)", found.stdout)[1]
    assert Path(resolved).resolve() == dependency.resolve()
    assert run("patchelf", "--print-rpath", plugins / "libgstplayback.so").stdout.strip() == "$ORIGIN:$ORIGIN/.."


def test_a_host_librarys_own_dependencies_stay_on_the_host(checkout: Path) -> None:
    # libz.so.1 is a declared host library; its fixture needs libhostdep.so.1, which no list names. Bundling that
    # would hand the host's libz, and every host plugin sharing it, an older copy than they were built against.
    # It also needs libengine-dependency.so.1, which the playback plugin needs itself: that one stays bundled, since
    # another distribution's libz need not link it, or ship it under that name (Debian's libbz2.so.1.0).
    lib = checkout / "system/usr/lib/fixture"
    shared(lib / "libhostdep.so.1", "int hostdep(void) { return 3; }", "-Wl,-soname,libhostdep.so.1")
    shared(lib / "libz.so.1",
           "extern int hostdep(void); extern int dependency(void);"
           " int zfixture(void) { return hostdep() + dependency(); }",
           "-Wl,-soname,libz.so.1", f"-L{lib}", "-l:libhostdep.so.1", "-lengine-dependency", f"-Wl,-rpath,{lib}")
    shared(lib / "gstreamer-1.0/libgstlibav.so", "extern int zfixture(void); int plugin(void) { return zfixture(); }",
           f"-L{lib}", "-l:libz.so.1", f"-Wl,-rpath,{lib}")
    result = package(checkout, checkout / "dist")
    assert result.returncode == 0, result.stderr
    archive = next((checkout / "dist").glob("*.tar.zst"))
    listing = run("tar", "--zstd", "-tf", archive)
    assert listing.returncode == 0, listing.stderr
    names = {Path(entry).name for entry in listing.stdout.split()}
    assert "libgstlibav.so" in names
    assert not {name for name in names if name.startswith(("libz.", "libhostdep."))}
    assert "libengine-dependency.so.1" in names


@pytest.mark.parametrize("changed", ["allowlist", "hostlibs", "wrapper", "closure", "arch", "package"])
def test_key_tracks_runtime_inputs(checkout: Path, changed: str) -> None:
    old = package(checkout, "--print-key")
    assert old.returncode == 0, old.stderr
    args, env = [], {}
    if changed in ("allowlist", "hostlibs", "wrapper", "closure"):
        name = {"allowlist": "packaging/linux/gstreamer-plugins.txt",
                "hostlibs": "packaging/linux/runtime-libraries.txt",
                "wrapper": "scripts/package-web-engine.sh", "closure": "scripts/web-engine-closure.py"}[changed]
        path = checkout / name
        path.write_text(path.read_text() + ("offlist\n" if changed == "allowlist" else "\n# changed runtime recipe\n"))
    elif changed == "arch":
        args = ["--arch", "aarch64" if os.uname().machine == "x86_64" else "x86_64"]
    else:
        env = {"FIXTURE_PACKAGE_VERSION": "1.1-security-update"}
    new = package(checkout, "--print-key", *args, **env)
    assert new.returncode == 0, new.stderr
    assert old.stdout != new.stdout


def test_archive_rejects_a_foreign_architecture(checkout: Path) -> None:
    arch = "aarch64" if os.uname().machine == "x86_64" else "x86_64"
    output = checkout / "dist"
    result = package(checkout, "--arch", arch, output)
    assert result.returncode != 0
    assert f"cannot package {arch} runtime on {os.uname().machine} host" in result.stderr
    assert not output.exists()


def test_missing_allowlisted_plugin_is_an_error(checkout: Path) -> None:
    (checkout / "system/usr/lib/fixture/gstreamer-1.0/libgstplayback.so").unlink()
    result = package(checkout, "--print-key")
    assert result.returncode != 0 and "missing runtime input" in result.stderr
    assert "libgstplayback.so" in result.stderr


def test_stale_wpe_prefix_is_an_error(checkout: Path) -> None:
    (checkout / "wpe/.release-pins").write_text("old-key\n")
    result = package(checkout, "--print-key")
    assert result.returncode != 0 and "release WPE prefix is stale" in result.stderr


def test_shared_host_check_rejects_an_undeclared_dependency(checkout: Path) -> None:
    tree = checkout / "bundle"
    tree.mkdir()
    plugin = checkout / "system/usr/lib/fixture/gstreamer-1.0/libgstplayback.so"
    shutil.copy2(plugin, tree / "plugin.so")
    # A bare host-path RUNPATH is overwritten, so this library becomes missing.
    result = run("python3", checkout / "scripts/web-engine-closure.py", "check", tree, tree / "lib",
                 checkout / "packaging/linux/runtime-libraries.txt")
    assert result.returncode != 0 and "unresolved dependencies" in result.stderr
    # A host path supplied through the loader cache (modeled by ldd output) is
    # resolved but still fails the declared-host-library check.
    ldd = checkout / "tools/ldd"
    ldd.write_text(f"#!/bin/sh\nprintf '\\tlibengine-dependency.so.1 => {plugin.parent.parent}/libengine-dependency.so.1 (0x123)\\n'\n")
    ldd.chmod(0o755)
    env = dict(os.environ, PATH=f"{ldd.parent}:{os.environ['PATH']}")
    result = run("python3", checkout / "scripts/web-engine-closure.py", "check", tree, tree / "lib",
                 checkout / "packaging/linux/runtime-libraries.txt", env=env)
    assert result.returncode != 0 and "undeclared host library: libengine-dependency.so.1" in result.stderr


def test_allowlist_contains_required_media_plugins() -> None:
    plugins = {line for line in (REPO / "packaging/linux/gstreamer-plugins.txt").read_text().splitlines()
               if line and not line.startswith("#")}
    assert len(plugins) == 71
    assert {"opengl", "encoding", "gio", "alsa", "libav", "va", "nvcodec", "v4l2codecs"} <= plugins
    assert not {"webrtc", "vulkan", "v4l2", "hls", "dash", "adaptivedemux2"} & plugins
