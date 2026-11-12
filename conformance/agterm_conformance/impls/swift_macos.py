"""The released upstream macOS app, exec'd directly from its bundle (never through `open`).

Upstream hosts live sessions in zmx, a daemon per session, and names the group after it: `UPSTREAM` maps the shared
tests' ngterm names (agterm_conformance.vocabulary) to upstream's, the one place the suite speaks zmx.
"""

from __future__ import annotations

import os
import plistlib
import shutil
import signal
import subprocess
import sys
import uuid
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import TYPE_CHECKING, Any

from agterm_conformance.client import ClientError, check_under_root
from agterm_conformance.impls.base import (
    MINIMAL_PATH,
    SESSION_HOST,
    STOP_TIMEOUT,
    Adapter,
    Instance,
    LaunchError,
    Profile,
    find_leftovers,
    login_shell_blocker,
    process_identity,
)
from agterm_conformance.vocabulary import GROUP, Vocabulary, fnv1a64, foundation_resolved
from agterm_conformance.wait import WaitTimeout, wait_until

if TYPE_CHECKING:
    from agterm_conformance.pty_process import LiveInstance, PtyProcess

PARITY_LEVEL = "0.41.1"
BASE_CAPABILITIES = frozenset(
    {
        "posix-shell", "posix-command-line", "untitled-panes", "osc-answers", "window-geometry", "window-minimize",
        "sidebar-min-width", "ui-string-values",
    }
)  # fmt: skip
LAUNCH_MODES = ("uitest", "plain")
BUNDLE_DIR = "app"
# The bundle's build metadata keys `version`'s `installed` is read from.
VERSION_KEY = "CFBundleShortVersionString"
COMMIT_KEY = "GitCommit"


# The create-only command of a second client: should the daemon have gone, attach would otherwise create one running a
# shell.
GONE_GUARD = "printf '%s\\n' 'agterm: remote session is gone'; exit 1"
# Upstream's name for the live-session group.
ZMX = "zmx"


def upstream_socket_directory(
    state_dir: str | os.PathLike[str], name: str = "agterm", environ: Mapping[str, str] | None = None
) -> str:
    """`ZmxSupport.socketDirectory`: FNV-1a 64 of the canonical state dir, under `/tmp`, prefixed with the
    implementation's directory name (`Adapter.app_dir_name`). `environ`, the instance's, is not read: upstream has no
    Windows build."""
    del environ
    return f"/tmp/{name}-zmx-{fnv1a64(foundation_resolved(os.path.realpath(state_dir)))}"


def hosts_session_directory(words: Sequence[str], directory: str) -> bool:
    """`agterm-session-host host <directory>`, the argv `ClientConnector.startHost` spawns the macOS host with."""
    words = list(words)
    return any(
        os.path.basename(word) == SESSION_HOST and words[index + 1 : index + 3] == ["host", directory]
        for index, word in enumerate(words)
    )


def leading_client(instance: LiveInstance, endpoint: Mapping[str, Any], daemon: str) -> PtyProcess:
    """`zmx attach <daemon>` claiming the lead, so the pane's own client in `instance` follows: upstream hands an overlay
    to a presenter only while every pane's own client follows (`followsRemotely`), and a pane follows only when another
    client of its daemon claimed the lead. Started the way `RemoteSession.attachCommand` starts the viewer's on the
    origin, minus ssh.

    `endpoint` is the `zmx.tree` read-back; one naming another instance's daemons is refused.
    """
    # Late: POSIX-only, and this module loads on Windows too.
    from agterm_conformance.pty_process import PtyProcess

    executable, directory = endpoint["executable"], endpoint["socketDirectory"]
    if directory != instance.live_dir:
        raise ValueError(f"endpoint socket directory {directory!r} is not this instance's {instance.live_dir!r}")
    if not os.path.isabs(executable):
        raise ValueError(f"endpoint executable {executable!r} is not absolute")
    env = {
        "PATH": MINIMAL_PATH,
        "HOME": os.fspath(instance.home),
        "TERM": "xterm-256color",
        # The variables the origin's own pane sets (`ZmxSupport`), empty meaning unset to zmx.
        "ZMX_SESSION": "",
        "ZMX_SESSION_PREFIX": "",
        "ZMX_NO_DETACH_KEY": "1",
        "ZMX_DIR": directory,
        "ZMX_MANAGED": uuid.uuid4().hex,
        "ZMX_MANAGED_CLAIM": "1",
    }
    return PtyProcess([executable, "attach", daemon, "/bin/sh", "-c", GONE_GUARD], env)


# ngterm's texts upstream words in zmx's, each as the shared tests template it; the full list of changed strings is in
# conformance/docs/rust-build.md.
ZMX_TEXTS: Mapping[str, str] = {
    f"{GROUP}.kill requires an explicit --target": "zmx.kill requires an explicit --target",
    f"{GROUP}.kill requires --pane left|right": "zmx.kill requires --pane left|right",
    f"{GROUP}.kill requires --force": "zmx.kill requires --force",
    f"{GROUP}.kill needs a session id; 'active' is not accepted": "zmx.kill needs a session id; 'active' is not accepted",
    f"{GROUP}.kill needs a window id; 'active' is not accepted": "zmx.kill needs a window id; 'active' is not accepted",
    f"{GROUP}.screen requires a daemon name": "zmx.screen requires a daemon name",
    f"{GROUP}.reset requires --force": "zmx.reset requires --force",
    f"{GROUP}.present requires a session": "zmx.present requires a session",
    f"{GROUP}.attach requires a host": "zmx.attach requires a host",
    f"{GROUP}.attach requires a remote session": "zmx.attach requires a remote session",
    f"{GROUP}.attach.group requires a host": "zmx.attach.group requires a host",
    f"{GROUP}.attach.group requires a remote session": "zmx.attach.group requires a remote session",
    "could not read the live session screen of {name}": "could not read the zmx screen of {name}",
    "could not read the live session list": "could not read the zmx session list",
    "live sessions are unavailable in this instance": "zmx is unavailable in this instance",
    "this pane has no live session; run it in a main or split pane started under Live sessions": (
        "this pane has no zmx daemon; run it in a main or split pane started under Live sessions"
    ),
}
# ngterm's texts that name the app, in upstream's name.
NAME_TEXTS: Mapping[str, str] = {
    "outside ngterm": "outside agterm",
    "incomplete pane identity: NGTERM_SESSION_ID and NGTERM_PANE_ID must both be set; pass --as SESSION:PANE-ID": (
        "incomplete pane identity: AGTERM_SESSION_ID and AGTERM_PANE_ID must both be set; pass --as SESSION:PANE-ID"
    ),
    "restart ngterm to apply the configured mode": "restart agterm to apply the configured mode",
}

# Platform-neutral replica ownership wording, where upstream names its two Macs.
PLATFORM_TEXTS: Mapping[str, str] = {
    "--width/--height are not supported for an overlay this computer shows for another computer; resize it on that computer": (
        "--width/--height are not supported for an overlay this Mac shows for another Mac; resize it on that Mac"
    ),
}

UPSTREAM = Vocabulary(
    name="upstream",
    group=ZMX,
    result_key="zmx",
    backed_key="backedByZmx",
    ui_test_opt_in="AGTERM_UITEST_ENABLE_ZMX",
    session_variable="ZMX_SESSION",
    directory_variable="ZMX_DIR",
    variable_prefix="ZMX_",
    # `ZmxSupport.namePrefix`
    daemon_prefix="agterm-",
    texts={**ZMX_TEXTS, **NAME_TEXTS, **PLATFORM_TEXTS},
    socket_directory=upstream_socket_directory,
    leading_client=leading_client,
    # zmx's daemons and the macOS session host
    daemon_names=frozenset({ZMX, SESSION_HOST}),
    hosts=hosts_session_directory,
)


class SwiftMacosAdapter(Adapter):
    """`uitest` with zmx by default (conformance/docs/spike-macos.md).

    `uitest` sets the XCUITest sentinel: window present (FB11763863), no welcome, no quit prompt, and zmx
    off unless `enable_zmx`. `plain` is a bare exec.
    """

    name = "swift-macos"
    vocabulary = UPSTREAM
    session_host_timeout = STOP_TIMEOUT

    def __init__(
        self, bin: Path | None, ctl: Path | None = None, launch_mode: str = "uitest", enable_zmx: bool = True
    ) -> None:
        if launch_mode not in LAUNCH_MODES:
            raise ValueError(f"launch_mode must be one of {LAUNCH_MODES}, got {launch_mode!r}")
        super().__init__(bin, ctl)
        self.launch_mode = launch_mode
        self.enable_zmx = enable_zmx

    @property
    def profile(self) -> Profile:
        blocker = self.live_sessions_blocker()
        live = frozenset() if blocker else frozenset({"live-sessions"})
        return Profile(PARITY_LEVEL, BASE_CAPABILITIES | live, {"live-sessions": blocker} if blocker else {})

    def live_sessions_blocker(self) -> str | None:
        if self.launch_mode == "uitest" and not self.enable_zmx:
            return f"the XCUITest sentinel turns zmx off without {self.vocabulary.ui_test_opt_in}"
        return login_shell_blocker()

    def _macos_dir(self) -> Path | None:
        if self.bin is None:
            return None
        path = Path(self.bin)
        return path / "Contents" / "MacOS" if path.suffix == ".app" else path.parent

    def executable(self) -> list[str]:
        if self.bin is None:
            raise LaunchError(f"{self.name}: no binary to launch")
        path = Path(self.bin)
        return [os.fspath(path / "Contents" / "MacOS" / "agterm" if path.suffix == ".app" else path)]

    def default_ctl(self) -> Path | None:
        directory = self._macos_dir()
        return directory / "agtermctl" if directory is not None else None

    def platform_env(self, env: dict[str, str]) -> None:
        # Points NSHomeDirectory, and with it the defaults lookup, at the isolated home.
        env["CFFIXED_USER_HOME"] = env["HOME"]
        if self.launch_mode == "uitest":
            env["AGTERM_UITEST_FORCE_SIDEBAR_VISIBLE"] = "1"
            if self.enable_zmx:
                env[self.vocabulary.ui_test_opt_in] = "1"

    def first_run_env(self, env: dict[str, str]) -> None:
        # The sentinel suppresses the welcome; this opts back in (upstream WelcomeUITests).
        if self.launch_mode == "uitest":
            env["AGTERM_UITEST_SHOW_WELCOME"] = "1"

    def bundle(self) -> Path | None:
        if self.bin is None:
            return None
        path = Path(self.bin)
        return next((candidate for candidate in (path, *path.parents) if candidate.suffix == ".app"), None)

    def instance_bundle(self, root: Path) -> Path | None:
        bundle = self.bundle()
        return root / BUNDLE_DIR / bundle.name if bundle is not None else None

    def instance_executable(self, root: Path, private_install: bool = False) -> list[str]:
        """The instance's own copy of the bundle: next to a running instance of the same bundle path, a launch keeps
        no window and binds no socket (conformance/docs/spike-macos.md). That copy is the private install too."""
        clone = self.instance_bundle(root)
        if clone is None:
            if private_install:
                raise LaunchError(f"{self.name}: a private install needs a bundle, not {self.bin}")
            return self.executable()
        if not clone.exists():
            clone.parent.mkdir(parents=True, exist_ok=True)
            source = self.bundle()
            if sys.platform == "darwin":
                # An APFS clone: no data is copied.
                subprocess.run(["cp", "-c", "-R", os.fspath(source), os.fspath(clone)], check=True)
            else:
                shutil.copytree(source, clone, symlinks=True)
        return [os.fspath(clone / "Contents" / "MacOS" / "agterm")]

    def write_installed(self, instance: Instance, version: str | None, commit: str | None = None) -> None:
        """The instance's bundle copy's `Info.plist`, which upstream's `AppIdentity.installed(bundleURL:)` parses on
        every `version`."""
        bundle = self.instance_bundle(instance.root)
        if bundle is None:
            raise LaunchError(f"{self.name}: the instance runs no bundle copy")
        plist = bundle / "Contents" / "Info.plist"
        if version is None:
            plist.unlink(missing_ok=True)
            return
        info = plistlib.loads(plist.read_bytes()) if plist.exists() else {}
        info[VERSION_KEY] = version
        if commit is None:
            info.pop(COMMIT_KEY, None)
        else:
            info[COMMIT_KEY] = commit
        temporary = plist.with_name(f".{plist.name}.tmp")
        temporary.write_bytes(plistlib.dumps(info))
        os.replace(temporary, plist)

    def owns_session_host(self, instance: Instance, pid: int) -> bool:
        """`pid` runs the session host of this instance's own bundle copy for its `ZMX_DIR`, derived from a state dir
        in its root."""
        bundle = self.instance_bundle(instance.root)
        if bundle is None or instance.live_dir != instance.vocabulary.socket_directory(
            instance.state_dir, self.app_dir_name
        ):
            return False
        try:
            check_under_root(instance.state_dir, instance.root)
        except ClientError:
            return False
        identity = process_identity(pid)
        expected = os.path.realpath(bundle / "Contents" / "MacOS" / SESSION_HOST)
        return (
            identity is not None and identity.executable == expected and hosts_session_directory(identity.words, instance.live_dir)
        )

    def stop_helpers(self, instance: Instance) -> str | None:
        """The session host outlives the app by design and takes no stop request from outside its bundle, so the
        harness ends the one its instance started: by exact pid, ownership re-verified before every signal."""
        hosts = [
            pid
            for pid, name, _ in find_leftovers(instance.live_dir, instance.vocabulary)
            if name == SESSION_HOST and self.owns_session_host(instance, pid)
        ]
        killed = [pid for pid in hosts if self._end_session_host(instance, pid)]
        if killed:
            return f"{SESSION_HOST} {', '.join(map(str, killed))} ignored SIGTERM for {self.session_host_timeout}s"
        return None

    def _end_session_host(self, instance: Instance, pid: int) -> bool:
        """True when SIGTERM was not enough; the leftover check reports a host that outlives SIGKILL too."""
        if not self._signal_owned(instance, pid, signal.SIGTERM):
            return False
        try:
            wait_until(lambda: not self.owns_session_host(instance, pid), self.session_host_timeout)
            return False
        except WaitTimeout:
            if self._signal_owned(instance, pid, signal.SIGKILL):
                try:
                    wait_until(lambda: not self.owns_session_host(instance, pid), self.session_host_timeout)
                except WaitTimeout:
                    pass
            return True

    def _signal_owned(self, instance: Instance, pid: int, signum: int) -> bool:
        if not self.owns_session_host(instance, pid):
            return False  # gone, or the pid now names another process
        _send_signal(pid, signum)
        return True


def _send_signal(pid: int, signum: int) -> None:
    try:
        os.kill(pid, signum)
    except ProcessLookupError:
        pass
