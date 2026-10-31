"""The Rust build: it reports its own parity level and capabilities on `version`; its GPUI frontend renders through
Vulkan, on lavapipe only, and needs a seat on its compositor. On Windows it renders through DirectX on the machine's
adapter (a runner's or a VM's, WARP without one), serves its control socket on a named pipe and quits on the session's
end sent to its hidden window (base.py's module docs)."""

from __future__ import annotations

import os
import shutil
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from agterm_conformance.impls import base
from agterm_conformance.impls.base import (
    RUST_PREFIX,
    FAKE_SEAT_ARG,
    LAUNCH_TIMEOUT,
    NO_PROFILE,
    Adapter,
    Instance,
    LaunchError,
    Profile,
    lavapipe_icd,
    software_rendering_env,
    version_tuple,
)
from agterm_conformance.vocabulary import LIVE

# `version` -> result.conformance = {"parityLevel": "0.38.1", "capabilities": ["posix-shell", ...]}.
# Optional: a build without it, like a run without a binary, is below every level.
CONFORMANCE_FIELD = "conformance"
# The staged layout's installed-build identity, relative to the stage root (scripts/stage.sh,
# crates/ngterm-core/src/identity.rs): a version line, then a commit line.
INSTALLED_FILE = Path("share/ngterm/app-BUILD")
# Where an instance's private install lies in its root.
PRIVATE_INSTALL_DIR = "install"

class ProfileError(ValueError):
    pass


def profile_from_version(result: Mapping[str, Any] | None) -> Profile:
    block = (result or {}).get(CONFORMANCE_FIELD)
    if block is None:
        return NO_PROFILE
    if not isinstance(block, Mapping):
        raise ProfileError(f"version.{CONFORMANCE_FIELD} is not an object: {block!r}")
    level = block.get("parityLevel")
    capabilities = block.get("capabilities", [])
    if level is not None:
        if not isinstance(level, str):
            raise ProfileError(f"parityLevel is not a string: {level!r}")
        try:
            version_tuple(level)
        except ValueError as error:
            raise ProfileError(str(error)) from None
    if not isinstance(capabilities, list) or not all(isinstance(item, str) for item in capabilities):
        raise ProfileError(f"capabilities is not a list of strings: {capabilities!r}")
    return Profile(parity_level=level, capabilities=frozenset(capabilities))


class RustAdapter(Adapter):
    name = "rust"
    app_dir_name = "ngterm"
    ctl_name = "ngtermctl"
    env_prefix = RUST_PREFIX
    # `live.*`, `ngtermctl live`, `result.live`, `backedByLive`: upstream's `zmx` group renamed
    # (conformance/docs/rust-build.md > The `live` group).
    vocabulary = LIVE
    # Windows names its single instance by the app id and the state dir: a unique id keeps it off the user's too.
    needs_app_id = sys.platform.startswith("linux") or sys.platform == "win32"
    needs_runtime_dir = sys.platform.startswith("linux")
    # crates/ngterm-gpui/src/system_window.rs
    quit_window_class = "ngterm-system-window"

    def __init__(self, bin: Path | None, ctl: Path | None = None) -> None:
        super().__init__(bin, ctl)
        self._profile: Profile | None = None
        self._probe_error: Exception | None = None

    def weston_args(self) -> tuple[str, ...]:
        # through the module: the self-tests swap in their fake weston there
        base.require_fake_seat(base.compositor_program())
        return (*super().weston_args(), FAKE_SEAT_ARG)

    def platform_env(self, env: dict[str, str]) -> None:
        super().platform_env(env)
        if sys.platform.startswith("linux"):
            software_rendering_env(env, lavapipe_icd())

    @property
    def profile(self) -> Profile:
        """Resolved once: from the first launched instance, or by a probe launch when none ran yet."""
        if self.bin is None:
            return NO_PROFILE
        if self._probe_error is not None:
            # A fresh exception: re-raising the cached one would grow its traceback with every test.
            raise LaunchError(f"{self.name}: probe launch failed: {self._probe_error}") from self._probe_error
        if self._profile is None:
            # Remembered: a build that cannot launch must not cost every test another launch timeout.
            try:
                instance = self.launch()
            except Exception as error:
                self._probe_error = error
                raise
            self.stop(instance)
        assert self._profile is not None
        return self._profile

    def launch(
        self,
        timeout: float = LAUNCH_TIMEOUT,
        app_id: str | None = None,
        settings: Mapping[str, Any] | None = None,
        first_run: bool = False,
        private_install: bool = False,
    ) -> Instance:
        instance = super().launch(timeout, app_id, settings, first_run, private_install)
        return instance if first_run else self._profiled(instance)

    def serving(self, instance: Instance, timeout: float = LAUNCH_TIMEOUT) -> Instance:
        return self._profiled(super().serving(instance, timeout))

    def _profiled(self, instance: Instance) -> Instance:
        if self._profile is None:
            try:
                self._profile = profile_from_version(instance.version)
            except ProfileError:
                self.stop(instance)
                raise
        return instance

    def default_ctl(self) -> Path | None:
        if self.bin is None:
            return None
        sibling = Path(self.bin).parent / (self.ctl_name + (".exe" if sys.platform == "win32" else ""))
        return sibling if sibling.exists() else None

    def instance_executable(self, root: Path, private_install: bool = False) -> list[str]:
        """A private install is the stage mirrored under the root (`lay_out_private_install`); a relaunch on that root
        runs from it again."""
        install = root / PRIVATE_INSTALL_DIR
        if private_install:
            if self.bin is None:
                raise LaunchError(f"{self.name}: no binary to launch")
            lay_out_private_install(Path(self.bin), install)
        if install.is_dir():
            return [os.fspath(install / "bin" / Path(self.executable()[0]).name)]
        return self.executable()

    def write_installed(self, instance: Instance, version: str | None, commit: str | None = None) -> None:
        install = instance.root / PRIVATE_INSTALL_DIR
        if not install.is_dir():
            raise LaunchError(f"{self.name}: the instance has no private install: launch(private_install=True)")
        file = install / INSTALLED_FILE
        if version is None:
            file.unlink(missing_ok=True)
            return
        file.parent.mkdir(parents=True, exist_ok=True)
        # By a rename, as scripts/stage.sh publishes it: a request never reads it half written.
        temporary = file.with_name(f".{file.name}.tmp")
        temporary.write_text(f"{version}\n{commit or ''}\n")
        os.replace(temporary, file)


def lay_out_private_install(binary: Path, install: Path) -> None:
    """A stage of its own for one instance: `binary` (a staged `bin/ngterm`) copied (hard-linked where it can be) and
    the installed-build identity copied, everything else of the stage linked to it. The app reads the identity beside
    the executable it was started from, so a test rewrites this copy and never the stage under test."""
    if binary.parent.name != "bin":
        raise LaunchError(f"a private install needs a staged binary (bin/<name>), not {binary}")
    _mirror(binary.parent.parent, install, Path(), {Path("bin") / binary.name, INSTALLED_FILE})


def _mirror(stage: Path, install: Path, relative: Path, copied: set[Path]) -> None:
    (install / relative).mkdir(parents=True, exist_ok=True)
    for entry in sorted((stage / relative).iterdir()):
        path = relative / entry.name
        if path == INSTALLED_FILE:
            shutil.copy2(entry, install / path)
        elif path in copied:
            _copy(entry, install / path)
        elif any(path in item.parents for item in copied):
            _mirror(stage, install, path, copied)
        else:
            _link(entry, install / path)


def _link(source: Path, destination: Path) -> None:
    """A symlink; on Windows, where one needs a privilege the account may lack, a copy instead."""
    try:
        os.symlink(source, destination, target_is_directory=source.is_dir())
    except OSError:
        if sys.platform != "win32":
            raise
        if source.is_dir():
            shutil.copytree(source, destination, symlinks=True)
        else:
            shutil.copy2(source, destination)


def _copy(source: Path, destination: Path) -> None:
    """A hard link where the stage and the root share a filesystem: the binary is never written, only replaced."""
    try:
        os.link(source, destination)
    except OSError:
        shutil.copy2(source, destination)
