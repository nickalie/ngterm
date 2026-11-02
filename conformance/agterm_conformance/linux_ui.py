"""Linux UI test resources: a run-owned accessibility bus and per-instance Xvfb.

GI is imported only after checking ownership; libatspi binds one bus per process.
Neither helper inherits the desktop's display, session bus or accessibility address.
"""
from __future__ import annotations

import ast
import contextlib
import os
import selectors
import shutil
import signal
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from agterm_conformance.impls import base

LIBEXEC_DIRS = ('/usr/libexec', '/usr/lib', '/usr/lib/at-spi2-core', '/usr/lib64')
_active_run: A11yRun | None = None


def program(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    for directory in LIBEXEC_DIRS:
        candidate = Path(directory) / name
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    raise base.LaunchError(f'Linux UI setup: missing {name}')


def atspi_bindings() -> Any:
    try:
        import gi
        gi.require_version('Atspi', '2.0')
        from gi.repository import Atspi
        return Atspi
    except (ImportError, ValueError) as error:
        raise base.LaunchError(f'Linux UI setup: PyGObject/Atspi-2.0 typelib: {error}') from error


def check_setup(*, check_bindings: bool = True) -> None:
    for name in ('Xvfb', 'gdbus', 'dbus-daemon', 'at-spi-bus-launcher', 'at-spi2-registryd'):
        program(name)
    if check_bindings:
        atspi_bindings()


def _call(env: dict[str, str], address: str, destination: str, path: str, method: str, *args: str) -> str:
    command = [program('gdbus'), 'call', '--address', address, '--dest', destination,
               '--object-path', path, '--method', method, *args]
    try:
        result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=5)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise base.LaunchError(f'Linux UI setup: {method}: {error}') from error
    if result.returncode:
        raise base.LaunchError(f'Linux UI setup: {method}: {result.stderr.strip()}')
    return result.stdout.strip()


def _wait_call(process: subprocess.Popen, timeout: float, call) -> str:
    deadline = time.monotonic() + timeout
    while True:
        try:
            return call()
        except base.LaunchError as error:
            if process.poll() is not None or time.monotonic() >= deadline:
                raise error
            try:
                process.wait(timeout=base.POLL_INTERVAL)
            except subprocess.TimeoutExpired:
                pass


class A11yRun:
    def __init__(self) -> None:
        self.root: Path | None = None
        self.address = ''
        self.active = False
        self.session_bus = None
        self.launcher = None
        self.registry = None
        self.atspi = None
        self.env: dict[str, str] = {}

    def start(self, timeout: float = base.LAUNCH_TIMEOUT) -> A11yRun:
        global _active_run
        if _active_run is not None or os.environ.get('AT_SPI_BUS_ADDRESS'):
            raise base.LaunchRefused('Linux UI requires its own run a11y bus; refusing an inherited address')
        check_setup()
        atspi = atspi_bindings()
        if atspi.is_initialized():
            raise base.LaunchRefused('Atspi was initialized before the run owned its a11y bus')
        self.root = base.make_root()
        try:
            home = self.root / 'home'
            home.mkdir()
            runtime = self.root / base.RUNTIME_DIR_NAME
            runtime.mkdir(mode=0o700)
            self.env = base.scrubbed_environ()
            for key in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY', 'AT_SPI_BUS_ADDRESS', 'DBUS_SESSION_BUS_ADDRESS'):
                self.env.pop(key, None)
            self.env.update(HOME=str(home), XDG_RUNTIME_DIR=str(runtime), GSETTINGS_BACKEND='memory',
                            DBUS_SESSION_BUS_ADDRESS=f'unix:path={runtime / base.DBUS_SOCKET}')
            for name in base.XDG_HOMES:
                self.env[name] = str(home / name.lower())
            self.session_bus = base.start_session_bus(self.root, self.env, timeout)
            self.launcher = base._spawn([program('at-spi-bus-launcher'), '--launch-immediately'],
                                       self.env, self.root / 'a11y-launcher.log', 'wb')
            result = _wait_call(self.launcher, timeout, lambda: _call(
                self.env, self.env['DBUS_SESSION_BUS_ADDRESS'], 'org.a11y.Bus', '/org/a11y/bus',
                'org.a11y.Bus.GetAddress'))
            _call(self.env, self.env['DBUS_SESSION_BUS_ADDRESS'], 'org.a11y.Bus',
                  '/org/a11y/bus', 'org.freedesktop.DBus.Properties.Set',
                  'org.a11y.Status', 'IsEnabled', '<true>')
            self.address = ast.literal_eval(result)[0]
            if not isinstance(self.address, str) or not self.address.startswith('unix:'):
                raise base.LaunchError(f'Linux UI setup: invalid owned a11y address {self.address!r}')
            self.env['AT_SPI_BUS_ADDRESS'] = self.address
            # Explicit ownership rather than relying on D-Bus service activation and its detached child.
            self.registry = base._spawn([program('at-spi2-registryd')], self.env,
                                       self.root / 'a11y-registry.log', 'wb')
            _wait_call(self.registry, timeout, lambda: _call(
                self.env, self.address, 'org.freedesktop.DBus', '/org/freedesktop/DBus',
                'org.freedesktop.DBus.GetNameOwner', 'org.a11y.atspi.Registry'))
            self.active = True
            _active_run = self
            os.environ['AT_SPI_BUS_ADDRESS'] = self.address
            self.init_atspi(atspi)
            return self
        except BaseException as error:
            for log in self.root.glob("*.log"):
                error.add_note(f"{log.name}: {base._tail(log)}")
            self.close()
            raise

    def check(self) -> None:
        if not self.active or _active_run is not self:
            raise base.LaunchRefused('Linux UI a11y bus is not owned by the active run')
        for name, process in (('session bus', self.session_bus), ('launcher', self.launcher), ('registry', self.registry)):
            if process is not None and process.poll() is not None:
                raise base.LaunchError(f'Linux UI {name} exited with {process.returncode}')

    def init_atspi(self, atspi=None):
        self.check()
        if os.environ.get('AT_SPI_BUS_ADDRESS') != self.address:
            raise base.LaunchRefused('Atspi requires the active run a11y bus address')
        atspi = atspi if atspi is not None else atspi_bindings()
        if self.atspi is None:
            if atspi.is_initialized():
                raise base.LaunchRefused('Atspi was initialized outside this run')
            if atspi.init() != 0:
                raise base.LaunchError('Linux UI setup: Atspi.init failed')
            self.atspi = atspi
        return self.atspi

    def close(self) -> None:
        global _active_run
        if self.atspi is not None:
            self.atspi.exit()
            self.atspi = None
        if _active_run is self:
            _active_run = None
            if os.environ.get('AT_SPI_BUS_ADDRESS') == self.address:
                os.environ.pop('AT_SPI_BUS_ADDRESS', None)
        self.active = False
        for process in (self.registry, self.launcher, self.session_bus):
            if process is not None:
                # The launcher starts its a11y dbus-daemon in the same private process group.
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGTERM)
                base._terminate(process)
                with contextlib.suppress(ProcessLookupError):
                    os.killpg(process.pid, signal.SIGKILL)
        self.registry = self.launcher = self.session_bus = None
        if self.root is not None:
            shutil.rmtree(self.root, ignore_errors=True)


@dataclass(frozen=True)
class X11Context:
    run: A11yRun
    display: str
    root: Path
    status: Any = None

    def close(self) -> None:
        if self.status is not None:
            base._terminate(self.status)

    def apply(self, env: dict[str, str]) -> None:
        self.run.check()
        env.pop('WAYLAND_DISPLAY', None)
        env.pop('XAUTHORITY', None)
        env.update(DISPLAY=self.display, AT_SPI_BUS_ADDRESS=self.run.address,
                   GDK_BACKEND='x11', GSETTINGS_BACKEND='memory')

    def check(self, env, root) -> None:
        self.run.check()
        if self.status is not None and self.status.poll() is not None:
            raise base.LaunchError(f'Linux UI status helper exited: {base._tail(self.root / "a11y-status.log")}')
        if Path(root) != self.root:
            raise base.LaunchRefused('Xvfb does not belong to the instance root')
        for key, value in {'DISPLAY': self.display, 'AT_SPI_BUS_ADDRESS': self.run.address,
                           'GDK_BACKEND': 'x11', 'GSETTINGS_BACKEND': 'memory'}.items():
            if env.get(key) != value:
                raise base.LaunchRefused(f'{key} is not the instance/run-owned UI value')
        if 'WAYLAND_DISPLAY' in env or 'XAUTHORITY' in env:
            raise base.LaunchRefused('X11 UI launch carries another display or authority')


def start_status(root: Path, env: dict[str, str], timeout: float):
    process = base._spawn([sys.executable, str(Path(__file__).with_name('a11y_status.py').resolve())],
                          env, root / 'a11y-status.log', 'wb')
    try:
        _wait_call(process, timeout, lambda: _call(
            env, env['DBUS_SESSION_BUS_ADDRESS'], 'org.a11y.Bus', '/org/a11y/bus',
            'org.freedesktop.DBus.Properties.GetAll', 'org.a11y.Status'))
        return process
    except BaseException as error:
        error.add_note(base._tail(root / 'a11y-status.log'))
        base._terminate(process)
        raise


def start_xvfb(root: Path, env: dict[str, str], run: A11yRun, timeout: float):
    run.check()
    read_fd, write_fd = os.pipe()
    process = None
    try:
        with open(root / 'xvfb.log', 'wb') as log:
            width, height = base.WESTON_SIZE
            process = subprocess.Popen([program('Xvfb'), '-displayfd', str(write_fd), '-nolisten', 'tcp',
                                        '-screen', '0', f'{width}x{height}x24'], env={
                                            k: v for k, v in env.items() if k not in ('DISPLAY', 'WAYLAND_DISPLAY', 'XAUTHORITY')},
                                       stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                                       pass_fds=(write_fd,), start_new_session=True)
        os.close(write_fd)
        write_fd = -1
        with selectors.DefaultSelector() as selector:
            selector.register(read_fd, selectors.EVENT_READ)
            if not selector.select(timeout):
                raise base.LaunchError('Linux UI setup: Xvfb displayfd timed out')
            value = os.read(read_fd, 128).decode().strip()
        if not value.isdecimal():
            raise base.LaunchError(f'Linux UI setup: Xvfb invalid displayfd {value!r}: {base._tail(root / "xvfb.log")}')
        context = X11Context(run, ':' + value, root)
        context.apply(env)
        status = start_status(root, env, timeout)
        return process, X11Context(run, context.display, root, status)
    except BaseException:
        if process is not None:
            base._terminate(process)
        raise
    finally:
        os.close(read_fd)
        if write_fd != -1:
            os.close(write_fd)


if __name__ == '__main__':
    try:
        check_setup()
    except base.LaunchError as error:
        print(error, file=sys.stderr)
        raise SystemExit(2)
