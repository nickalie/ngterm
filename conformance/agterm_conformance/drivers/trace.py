"""Per-test diagnostic evidence; collection never replaces the test's own outcome.

The context spans pytest setup through teardown. Snapshots stay in memory until the policy
keeps them, including snapshots taken just before an instance's root is removed.
"""
from __future__ import annotations

import hashlib
import json
import re
import time
from contextvars import ContextVar
from functools import wraps
from pathlib import Path
from typing import Any

ACTIVE: ContextVar[Trace | None] = ContextVar('conformance_trace', default=None)


def slug(value: str) -> str:
    return re.sub(r'[^a-zA-Z0-9_.-]+', '-', value).strip('-')[:100] + '-' + hashlib.sha256(value.encode()).hexdigest()[:10]


def event(channel: str, **data: Any) -> None:
    if record := ACTIVE.get():
        record.record(channel, data)


def remember(instance: Any, driver: Any = None) -> None:
    if record := ACTIVE.get():
        record.remember(instance, driver)


def before_stop(instance: Any) -> None:
    if record := ACTIVE.get():
        record.remember(instance)
        record.capture_one(str(instance.root))


def action(method):
    """Record public driver calls, including failed waits and their original exceptions."""
    @wraps(method)
    def wrapped(self, *args, **kwargs):
        record = ACTIVE.get()
        if record is None or record.mode == "off":
            return method(self, *args, **kwargs)
        entry = dict(action=method.__name__, pid=self.pid, args=[repr(a) for a in args],
                     kwargs={k: repr(v) for k, v in kwargs.items()})
        try:
            result = method(self, *args, **kwargs)
        except Exception as error:
            event('actions', **entry, error=f'{type(error).__name__}: {error}')
            raise
        event('actions', **entry, result=repr(result))
        return result
    return wrapped


class Trace:
    def __init__(self, nodeid: str, directory: Path, mode: str):
        self.path = directory / slug(nodeid)
        self.mode = mode
        self.logs = {name: [] for name in ('actions', 'socket', 'cli')}
        self.instances: dict[str, tuple[Any, Any]] = {}
        self.files: dict[str, bytes] = {}
        self.errors: list[str] = []
        self.collecting = False
        self.frozen = False
        self.cli_processes = []
        self.rerun_pending = False
        self.reset_outcome = False

    def release(self) -> None:
        """Do not retain successful-test evidence through pytest's session-long item list."""
        for process in self.cli_processes:
            process._trace = None
        self.logs = {name: [] for name in self.logs}
        self.instances.clear()
        self.files.clear()
        self.errors.clear()
        self.cli_processes.clear()

    def record(self, channel: str, entry: dict) -> dict | None:
        if self.mode != 'off' and not self.collecting:
            stored = dict(time=time.time(), **entry)
            self.logs[channel].append(stored)
            return stored
        return None

    def reset(self) -> None:
        self.logs = {name: [] for name in self.logs}
        self.files.clear()
        self.errors.clear()
        self.instances.clear()
        self.cli_processes.clear()
        self.frozen = False
        self.rerun_pending = True
        self.reset_outcome = True

    def remember(self, instance: Any, driver: Any = None) -> None:
        if self.mode != 'off':
            key = str(instance.root)
            previous = self.instances.get(key)
            self.instances[key] = (instance, driver or (previous[1] if previous else None))

    def _save(self, name: str, collect) -> None:
        try:
            value = collect()
            self.files[name] = value.encode() if isinstance(value, str) else value
        except Exception as error:
            self.errors.append(f'{name}: {type(error).__name__}: {error}')

    def capture_one(self, key: str) -> None:
        if self.mode == 'off' or self.collecting or self.frozen:
            return
        self.collecting = True
        try:
            instance, driver = self.instances[key]
            prefix = '' if key == next(iter(self.instances)) else f'instances/{slug(key)}/'
            self._save(prefix + 'app.log', lambda: instance.log.read_bytes())
            self._save(prefix + 'panes/index.json', lambda: self._panes(instance, prefix))
            if driver is not None:
                self._save(prefix + 'a11y-tree.txt', lambda: driver.tree_dump())
                self._save(prefix + 'screenshot.png', lambda: driver.screenshot())
        finally:
            self.collecting = False

    def _panes(self, instance, prefix):
        # Use a short diagnostic deadline, never the user's socket or a test command that changes state.
        client = instance.client(timeout=1.0) if hasattr(instance, 'client') else instance
        response = client.request('tree')
        if not response.ok:
            raise RuntimeError(response.error)
        result = response.result or {}
        tree = result.get('tree', result)
        for workspace in tree.get('workspaces', []):
            for session in workspace.get('sessions', []):
                sid = session['id']
                name = re.sub(r'[^a-zA-Z0-9_.-]', '_', str(sid))
                def text(sid=sid):
                    response = client.request('session.text', sid)
                    if not response.ok:
                        raise RuntimeError(response.error)
                    return (response.result or {}).get('text', '')
                self._save(prefix + f'panes/{name}.txt', text)
        return json.dumps(tree, ensure_ascii=False, indent=2)

    def capture(self) -> None:
        for process in self.cli_processes:
            try:
                process._record_trace()
            except Exception as error:
                self.errors.append(f'cli: {type(error).__name__}: {error}')
        for key in list(self.instances):
            self.capture_one(key)

    def finish(self, failed: bool) -> None:
        if self.mode == 'off' or (self.mode == 'on-failure' and not failed):
            return
        try:
            self.path.mkdir(parents=True, exist_ok=True)
            for channel, entries in self.logs.items():
                (self.path / f'{channel}.jsonl').write_text(''.join(json.dumps(e, ensure_ascii=False) + '\n' for e in entries))
            for name, data in self.files.items():
                path = self.path / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            if self.errors:
                (self.path / 'trace-errors.txt').write_text('\n'.join(self.errors) + '\n')
        except Exception as error:
            # A broken artifact directory must not turn a test failure into an INTERNALERROR.
            import sys
            print(f'could not write trace {self.path}: {error}', file=sys.stderr)
