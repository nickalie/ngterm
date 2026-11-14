import json
from pathlib import Path
from types import SimpleNamespace

import pytest
from agterm_conformance.drivers import trace


def instance(tmp_path):
    log = tmp_path / 'app.log'
    log.write_text('app evidence')
    def request(cmd, target=None, **args):
        result = {'tree': {'workspaces': [{'sessions': [{'id': 's1'}]}]}} if cmd == 'tree' else {'text': 'pane evidence'}
        return SimpleNamespace(ok=True, result=result, error=None)
    return SimpleNamespace(root=tmp_path, log=log, pid=123, request=request)


@pytest.mark.parametrize('mode,failed,writes', [('off',True,False), ('on-failure',False,False),
                                             ('on-failure',True,True), ('always',False,True)])
def test_trace_policy(tmp_path, mode, failed, writes):
    record = trace.Trace('tests/protocol/test_x.py::test_x', tmp_path / 'traces', mode)
    record.remember(instance(tmp_path))
    record.record('socket', {'request': {'cmd':'tree'}, 'response': {}})
    record.record('cli', {'argv': ['ngtermctl'], 'returncode': 0, 'stdout':'yes', 'stderr':''})
    record.capture()
    record.finish(failed)
    assert record.path.exists() == writes
    if writes:
        assert (record.path / 'app.log').read_text() == 'app evidence'
        assert (record.path / 'panes/s1.txt').read_text() == 'pane evidence'
        assert json.loads((record.path / 'socket.jsonl').read_text().splitlines()[0])['request']['cmd'] == 'tree'
        assert (record.path / 'cli.jsonl').exists()
        assert not (record.path / 'screenshot.png').exists()
        assert not (record.path / 'a11y-tree.txt').exists()


def test_driver_capture_and_error_isolation(tmp_path):
    record = trace.Trace('ui failure', tmp_path / 'traces', 'always')
    app = instance(tmp_path)
    driver = SimpleNamespace(tree_dump=lambda: 'button save', screenshot=lambda: b'PNG')
    record.remember(app, driver)
    record.capture()
    record.finish(True)
    assert (record.path / 'a11y-tree.txt').read_text() == 'button save'
    assert (record.path / 'screenshot.png').read_bytes() == b'PNG'
    driver.screenshot = lambda: (_ for _ in ()).throw(RuntimeError('grant denied'))
    record.capture()
    record.finish(True)
    assert 'grant denied' in (record.path / 'trace-errors.txt').read_text()


@pytest.mark.parametrize('stage', ['call', 'setup', 'teardown'])
def test_pytest_failure_keeps_original_error(pytester, stage):
    pytester.makeconftest('''
import pytest
from agterm_conformance.drivers import trace
@pytest.fixture
def evidence():
    trace.event('socket', request={'cmd':'ping'})
    if STAGE == 'setup': raise AssertionError('original failure')
    yield
    if STAGE == 'teardown': raise AssertionError('original failure')
'''.replace('STAGE', repr(stage)))
    pytester.makepyfile("def test_failure(evidence):\n" +
                      ("    assert False, 'original failure'\n" if stage == 'call' else '    pass\n'))
    result = pytester.runpytest('--trace-dir', str(pytester.path / 'traces'))
    assert result.ret == 1
    result.stdout.fnmatch_lines(['*original failure*'])
    assert list((pytester.path / 'traces').glob('*/socket.jsonl'))


def test_socket_success_and_failure(tmp_path, monkeypatch):
    from agterm_conformance.client import Client, ClientTimeout
    record = trace.Trace('socket', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    client = Client(tmp_path / 'control.sock', tmp_path)
    monkeypatch.setattr(client, '_exchange', lambda *a: b'{"ok":true,"result":{}}')
    try:
        client.request('tree')
        monkeypatch.setattr(client, '_exchange', lambda *a: (_ for _ in ()).throw(ClientTimeout('timed out')))
        with pytest.raises(ClientTimeout): client.request('tree')
    finally:
        trace.ACTIVE.reset(token)
    assert record.logs['socket'][0]['response']['ok']
    assert 'timed out' in record.logs['socket'][1]['error']


def test_cli_includes_consumed_stream_lines(tmp_path):
    import os
    import sys
    from agterm_conformance.plugin import CliProcess, Cli
    app = instance(tmp_path)
    app.socket, app.home, app.env = tmp_path / 'control.sock', tmp_path, dict(os.environ)
    record = trace.Trace('cli', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        cli = Cli(Path(sys.executable), app)
        result = cli._run([sys.executable, '-c', 'print("one-shot")'], app.env, None, 5)
        assert result.returncode == 0
        with CliProcess([sys.executable, '-c', 'print("first");print("second")'], app.env, tmp_path, None) as process:
            assert process.line() == 'first'
            assert process.finish().stdout == 'second\n'
    finally:
        trace.ACTIVE.reset(token)
    assert record.logs['cli'][0]['stdout'] == 'one-shot\n'
    assert record.logs['cli'][1]['stdout'] == 'first\nsecond\n'
    assert record.logs['cli'][1]['returncode'] == 0


def test_a_held_cli_process_blocks_on_its_output_until_released(tmp_path):
    import os
    import subprocess
    import sys
    from agterm_conformance.plugin import CliProcess
    # after its first line and a go-ahead, far past any OS pipe's buffer, so the writer waits on a reader
    script = ('import sys\nprint("first", flush=True)\nsys.stdin.readline()\n'
              'for n in range(4096): sys.stdout.write(f"{n:04d}" + "x" * 1019 + "\\n")')
    with CliProcess([sys.executable, '-c', script], dict(os.environ), tmp_path, None, interactive=True) as process:
        assert process.line() == 'first'
        process.hold()
        process.send('go\n')
        with pytest.raises(subprocess.TimeoutExpired):
            process.process.wait(timeout=1)
        process.release()
        assert process.line().startswith('0000')
        result = process.finish()
    assert result.returncode == 0
    assert len(result.stdout.splitlines()) == 4095


def test_action_success_failure_and_tree_dump(tmp_path):
    from test_drivers import FakeDriver
    from agterm_conformance.drivers.base import Element, _describe
    record = trace.Trace('actions', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        elements = [Element('add-session', 'button', 'Add'), Element(None, 'text', 'Hint')]
        driver = FakeDriver(elements)
        driver.press('a')
        with pytest.raises(ValueError): driver.press('not-a-key')
        assert all(_describe(el) in driver.tree_dump() for el in elements)
    finally:
        trace.ACTIVE.reset(token)
    assert record.logs['actions'][0]['action'] == 'press'
    assert 'ValueError' in record.logs['actions'][1]['error']


def test_multiple_instances_and_no_trace_cross_talk(tmp_path):
    record = trace.Trace('multiple', tmp_path / 'traces', 'always')
    for n in range(2):
        root = tmp_path / str(n)
        root.mkdir()
        record.remember(instance(root))
    record.capture()
    record.finish(True)
    assert len(list(record.path.rglob('app.log'))) == 2
    assert len(list(record.path.rglob('s1.txt'))) == 2


def test_screenshot_failure_does_not_replace_pytest_failure(pytester):
    pytester.makepyfile('''
from types import SimpleNamespace
from agterm_conformance.drivers import trace

def test_failure(tmp_path):
    log = tmp_path / 'app.log'
    log.write_text('evidence')
    def fail(): raise RuntimeError('screenshot denied')
    app = SimpleNamespace(root=tmp_path, log=log, request=lambda *a: SimpleNamespace(ok=True, result={}))
    trace.remember(app, SimpleNamespace(tree_dump=lambda: 'tree', screenshot=fail))
    assert False, 'original assertion'
''')
    result = pytester.runpytest('--trace-dir', str(pytester.path / 'traces'))
    result.assert_outcomes(failed=1)
    result.stdout.fnmatch_lines(['*original assertion*'])
    errors = next((pytester.path / 'traces').glob('*/trace-errors.txt'))
    assert 'screenshot denied' in errors.read_text()


def test_trace_option_preserves_pytest_debugger(pytester):
    config = pytester.parseconfigure('--trace', '--trace-mode=off')
    assert config.getoption('trace') is True
    assert config.getoption('trace_mode') == 'off'
    config = pytester.parseconfigure()
    assert config.getoption('trace') is False
    assert config.getoption('trace_mode') == 'on-failure'


@pytest.mark.parametrize('mode,writes', [('off',False), ('on-failure',False), ('always',True)])
def test_passing_pytest_trace_modes(pytester, mode, writes):
    pytester.makepyfile('def test_pass(): pass')
    result = pytester.runpytest('--trace-mode', mode, '--trace-dir', str(pytester.path / 'evidence'))
    result.assert_outcomes(passed=1)
    assert bool(list((pytester.path / 'evidence').glob('*/actions.jsonl'))) == writes


def test_linux_screenshot_uses_only_owned_display(tmp_path, monkeypatch):
    import sys
    from test_drivers import FakeDriver
    from agterm_conformance.drivers.base import DriverError
    calls = []
    image = SimpleNamespace(save=lambda stream, format: stream.write(b'PNG'))
    monkeypatch.setitem(sys.modules, 'PIL', SimpleNamespace(ImageGrab=SimpleNamespace(grab=lambda **kw: calls.append(kw) or image)))
    monkeypatch.setattr(sys, 'platform', 'linux')
    driver = FakeDriver([])
    with pytest.raises(DriverError, match='private Xvfb'): driver.screenshot()
    assert calls == []
    checked = []
    driver.instance = SimpleNamespace(root=tmp_path, env={}, ui=SimpleNamespace(display=':987', check=lambda *args: checked.append(args)))
    assert driver.screenshot() == b'PNG'
    assert checked == [({}, tmp_path)]
    assert calls == [{'xdisplay': ':987'}]


def test_before_stop_preserves_removed_logs(tmp_path):
    app = instance(tmp_path)
    record = trace.Trace('stop', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        trace.before_stop(app)
        app.log.unlink()
    finally:
        trace.ACTIVE.reset(token)
    record.finish(True)
    assert (record.path / 'app.log').read_text() == 'app evidence'


def test_successful_rerun_drops_failed_attempt(pytester):
    pytester.makepyfile('''
count = 0
def test_retry():
    global count
    count += 1
    assert count == 2
''')
    result = pytester.runpytest('--reruns=1', '--trace-dir', str(pytester.path / 'evidence'))
    assert result.ret == 0
    assert result.parseoutcomes()["rerun"] == 1
    assert not list((pytester.path / 'evidence').glob('*/actions.jsonl'))


@pytest.mark.parametrize('phase,outcomes', [('call', {'xfailed': 1}), ('setup', {'errors': 1})])
def test_an_upstream_bug_failure_keeps_its_trace(pytester, phase, outcomes):
    bin_path = pytester.path / 'agterm'
    bin_path.write_text('#!/bin/sh\n')
    pytester.makepyfile(f'''
import pytest

@pytest.fixture
def resource():
    assert {phase!r} != 'setup'

@pytest.mark.upstream_bug('lost-output', reason='upstream loses it')
def test_bug(resource, impl_options):
    assert False
''')
    result = pytester.runpytest('--impl', 'swift-macos', '--bin', str(bin_path),
                                '--trace-dir', str(pytester.path / 'evidence'))
    result.assert_outcomes(**outcomes)
    assert list((pytester.path / 'evidence').glob('*/actions.jsonl'))


def test_released_trace_drops_in_memory_evidence(tmp_path):
    record = trace.Trace('released', tmp_path / 'traces', 'on-failure')
    record.remember(instance(tmp_path))
    record.record('socket', {'large': 'payload'})
    record.capture()
    record.release()
    assert record.instances == {} and record.files == {}
    assert record.logs == {'actions': [], 'socket': [], 'cli': []}


def test_reused_trace_directory_removes_old_evidence(tmp_path):
    first = trace.Trace('same', tmp_path, 'on-failure')
    first.files['stale.png'] = b'old'
    first.finish(True)
    second = trace.Trace('same', tmp_path, 'on-failure')
    second.finish(True)
    assert not (second.path / 'stale.png').exists()
    second.finish(False)
    assert not second.path.exists()


def test_write_failure_keeps_other_artifacts_and_utf8(tmp_path, monkeypatch):
    record = trace.Trace('writes', tmp_path, 'always')
    record.record('socket', {'text': '日本語😀'})
    record.files.update({'broken.bin': b'no', 'app.log': b'yes'})
    write = Path.write_bytes
    def fail_one(path, data):
        if path.name == 'broken.bin': raise OSError('write failed')
        return write(path, data)
    monkeypatch.setattr(Path, 'write_bytes', fail_one)
    record.finish(True)
    assert '日本語😀' in (record.path / 'socket.jsonl').read_text(encoding='utf-8')
    assert (record.path / 'app.log').read_bytes() == b'yes'
    assert 'write failed' in (record.path / 'trace-errors.txt').read_text()


def test_remember_preserves_app_socket_but_accepts_relaunch(tmp_path):
    record = trace.Trace('relay', tmp_path, 'always')
    app = instance(tmp_path)
    app.socket = tmp_path / 'real.sock'
    record.remember(app)
    relay = SimpleNamespace(**vars(app))
    relay.socket = tmp_path / 'relay.sock'
    record.remember(relay)
    assert record.instances[str(tmp_path)][0] is app
    relaunched = SimpleNamespace(**vars(relay))
    relaunched.pid += 1
    record.remember(relaunched)
    assert record.instances[str(tmp_path)][0] is relaunched


def test_split_hidden_scratch_and_scrollback_are_captured(tmp_path):
    app = instance(tmp_path)
    calls = []
    def request(cmd, target=None, **args):
        calls.append((cmd, args))
        result = {'tree': {'workspaces': [{'sessions': [{'id': 's', 'scratch': False,
                  'surfaces': [{'kind': k} for k in ['left', 'right', 'scratch', 'overlay']]}]}]}} if cmd == 'tree' else {'text': args.get('pane', 'default')}
        return SimpleNamespace(ok=True, result=result)
    app.request = request
    record = trace.Trace('panes', tmp_path / 'traces', 'always')
    record.remember(app)
    record.capture()
    assert record.files['panes/s-scratch.txt'] == b'scratch'
    assert record.files['panes/s-right.txt'] == b'right'
    assert not record.errors
    assert all(args['all'] for cmd, args in calls if cmd == 'session.text')
    assert not any(args.get('pane') == 'overlay' for _, args in calls)


def test_default_directory_and_cross_test_isolation(pytester):
    pytester.makepyfile('''
from agterm_conformance.drivers import trace
def test_a():
    trace.event('socket', owner='a')
    assert False
def test_b():
    trace.event('socket', owner='b')
    assert False
''')
    pytester.runpytest().assert_outcomes(failed=2)
    paths = list((pytester.path / 'reports/traces').glob('*/socket.jsonl'))
    assert len(paths) == 2
    assert sorted([json.loads(p.read_text())['owner'] for p in paths]) == ['a', 'b']


@pytest.mark.parametrize('pass_second', [False, True])
def test_rerun_retains_only_final_attempt(pytester, pass_second):
    pytester.makepyfile('''
from agterm_conformance.drivers import trace
count = 0
def test_retry(tmp_path):
    global count
    count += 1
    trace.event('socket', attempt=count)
    record = trace.ACTIVE.get()
    record.files['attempt.txt'] = str(count).encode()
    record.errors.append('attempt ' + str(count))
    assert PASS and count == 2
'''.replace('PASS', repr(pass_second)))
    result = pytester.runpytest('--reruns=1', '--trace-mode=always')
    assert result.ret == (0 if pass_second else 1)
    folder = next((pytester.path / 'reports/traces').iterdir())
    assert json.loads((folder / 'socket.jsonl').read_text())['attempt'] == 2
    assert (folder / 'attempt.txt').read_text() == '2'
    assert (folder / 'trace-errors.txt').read_text() == 'attempt 2\n'


def test_plugin_releases_finished_items(pytester):
    pytester.makeconftest('''
from agterm_conformance.plugin import _TRACE_KEY, _TRACE_FAILED
from agterm_conformance.drivers import trace
from types import SimpleNamespace
processes = []
def pytest_runtest_call(item):
    record = trace.ACTIVE.get()
    process = SimpleNamespace(_trace=record, _record_trace=lambda: None)
    record.cli_processes.append(process)
    processes.append(process)
def pytest_sessionfinish(session):
    for item in session.items:
        assert _TRACE_KEY not in item.stash
        assert _TRACE_FAILED not in item.stash
    assert all(process._trace is None for process in processes)
''')
    pytester.makepyfile('def test_one(): pass\ndef test_two(): pass')
    pytester.runpytest('--trace-mode=always').assert_outcomes(passed=2)


@pytest.mark.parametrize('stage', ['setup', 'teardown'])
def test_adapter_stop_preserves_evidence_before_root_removal(pytester, stage):
    pytester.makeconftest('''
import pytest
from types import SimpleNamespace
from agterm_conformance.drivers import trace
from agterm_conformance.impls import base
@pytest.fixture
def app(tmp_path, monkeypatch):
    log = tmp_path / 'app.log'
    log.write_text('before stop')
    app = SimpleNamespace(root=tmp_path, log=log, pid=123, process=SimpleNamespace(poll=lambda: 0),
        app_id='fake', live_dir=tmp_path/'live', vocabulary=None, compositor=None, session_bus=None, ui=None,
        request=lambda *a, **kw: SimpleNamespace(ok=True, result={'tree': {'workspaces':[{'sessions':[{'id':'s'}]}]}} if a[0]=='tree' else {'text':'pane'}))
    for name in ['_terminate_app', '_leftovers_error', 'reap_process_tree', '_release_app_id', 'stop_headless', 'keep_logs', 'remove_live_dir']:
        monkeypatch.setattr(base, name, lambda *a: None)
    trace.remember(app)
    try:
        if STAGE == 'setup': raise AssertionError('setup error')
        yield app
    finally:
        base.Adapter.stop(SimpleNamespace(stop_helpers=lambda app: None), app)
        assert not log.exists()
        if STAGE == 'teardown': raise AssertionError('teardown error')
'''.replace('STAGE', repr(stage)))
    pytester.makepyfile('def test_example(app): pass')
    assert pytester.runpytest().ret == 1
    folder = next((pytester.path / 'reports/traces').iterdir())
    assert (folder / 'app.log').read_text() == 'before stop'
    assert (folder / 'panes/s.txt').read_text() == 'pane'
    assert not (folder / 'trace-errors.txt').exists()


@pytest.mark.parametrize('call_fails', [False, True])
def test_failed_teardown_refreshes_shutdown_log(pytester, call_fails):
    pytester.makeconftest('''
import pytest
from types import SimpleNamespace
from agterm_conformance.drivers import trace
@pytest.fixture
def app(tmp_path):
    log = tmp_path / 'app.log'
    log.write_text('before')
    app = SimpleNamespace(root=tmp_path, log=log, pid=1, request=lambda *a: SimpleNamespace(ok=True, result={}))
    trace.remember(app)
    yield
    trace.before_stop(app)
    log.write_text('after shutdown: thread snapshot')
    raise AssertionError('quit failed')
''')
    pytester.makepyfile('def test_app(app):\n    assert ' + repr(not call_fails))
    assert pytester.runpytest().ret == 1
    folder = next((pytester.path / 'reports/traces').iterdir())
    filename = 'app.teardown.log' if call_fails else 'app.log'
    assert 'thread snapshot' in (folder / filename).read_text()
    if call_fails: assert (folder / 'app.log').read_text() == 'before'


def test_stopped_driver_captured_once_and_live_pass_not_captured(pytester):
    pytester.makeconftest('''
import pytest
from types import SimpleNamespace
from agterm_conformance.drivers import trace
counts = []
@pytest.fixture
def app(tmp_path):
    log = tmp_path / 'app.log'
    log.write_text('log')
    app = SimpleNamespace(root=tmp_path, log=log, pid=1, request=lambda *a: SimpleNamespace(ok=True, result={}))
    driver = SimpleNamespace(tree_dump=lambda: counts.append('tree') or 'tree', screenshot=lambda: counts.append('image') or b'png')
    trace.remember(app, driver)
    yield app
    trace.before_stop(app)
@pytest.fixture(scope='module')
def live(tmp_path_factory):
    root = tmp_path_factory.mktemp('live')
    log = root / 'app.log'; log.write_text('log')
    app = SimpleNamespace(root=root, log=log, pid=2, request=lambda *a: SimpleNamespace(ok=True, result={}))
    driver = SimpleNamespace(tree_dump=lambda: counts.append('live') or 'tree', screenshot=lambda: b'png')
    return app, driver
def pytest_sessionfinish(session):
    assert counts == ['tree', 'image']
''')
    pytester.makepyfile('''
from agterm_conformance.drivers import trace
def test_stopped(app): pass
def test_live(live): trace.remember(*live)
''')
    pytester.runpytest().assert_outcomes(passed=2)


def test_cli_timeout_retains_partial_output(tmp_path):
    import os
    import subprocess
    import sys
    from agterm_conformance.plugin import Cli
    app = instance(tmp_path)
    app.socket, app.home, app.env = tmp_path / 'control.sock', tmp_path, dict(os.environ)
    record = trace.Trace('timeout', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        cli = Cli(Path(sys.executable), app)
        with pytest.raises(subprocess.TimeoutExpired):
            cli._run([sys.executable, '-u', '-c', 'import sys,time; print("out"); print("err",file=sys.stderr); time.sleep(10)'], app.env, None, .2)
    finally:
        trace.ACTIVE.reset(token)
    entry, = record.logs['cli']
    assert entry['stdout'] == 'out\n' and entry['stderr'] == 'err\n'
    assert 'TimeoutExpired' in entry['error']


@pytest.mark.parametrize('failure', ['spawn', 'send'])
def test_cli_startup_failure_records_original_error(tmp_path, monkeypatch, failure):
    import os
    import sys
    from agterm_conformance import plugin
    record = trace.Trace('cli failure', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    def fail(*a, **kw): raise OSError('original error')
    if failure == 'spawn': monkeypatch.setattr(plugin.subprocess, 'Popen', fail)
    else: monkeypatch.setattr(plugin.CliProcess, 'send', fail)
    try:
        with pytest.raises(OSError, match='original error'):
            plugin.CliProcess([sys.executable, '-c', 'pass'], os.environ, tmp_path, 'input')
        record.capture()
    finally:
        trace.ACTIVE.reset(token)
    assert any('original error' in entry.get('error', '') for entry in record.logs['cli'])
    assert record.errors == []
    assert all(process.process.poll() is not None for process in record.cli_processes)


def test_stream_logs_success_timeout_connect_and_send_errors(tmp_path, monkeypatch):
    from agterm_conformance import client as module
    chunks = [b'{"ok":true}\n', b'{"frame":1}\n']
    def recv(size):
        if chunks: return chunks.pop(0)
        raise TimeoutError
    sock = SimpleNamespace(sendall=lambda data: None, settimeout=lambda timeout: None,
                           recv=recv, close=lambda: None)
    monkeypatch.setattr(module.transport, 'connect', lambda *a: sock)
    record = trace.Trace('stream', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        client = module.Client(tmp_path / 'socket', tmp_path)
        with client.open_stream('events') as stream:
            stream.send({'input': 1})
            assert stream.read() == {'frame': 1}
            with pytest.raises(module.ClientTimeout): stream.read_line(.01)
            def fail(data): raise BrokenPipeError('closed')
            sock.sendall = fail
            with pytest.raises(BrokenPipeError): stream.send({'input': 2})
        def connect(*a): raise OSError('unavailable')
        monkeypatch.setattr(module.transport, 'connect', connect)
        with pytest.raises(module.ConnectFailed): client.open_stream('events')
    finally:
        trace.ACTIVE.reset(token)
    entries = record.logs['socket']
    assert [e['direction'] for e in entries] == ['stream-open', 'stream-receive', 'stream-send', 'stream-receive', 'stream-receive', 'stream-send', 'stream-open']
    assert entries[3]['line'] == '{"frame":1}'
    assert 'ClientTimeout' in entries[4]['error']
    assert 'BrokenPipeError' in entries[5]['error']
    assert 'ConnectFailed' in entries[6]['error']


def test_real_relay_worker_inherits_trace_context(tmp_path, monkeypatch):
    import tempfile
    import shutil
    from test_client import FakeServer, reply
    from agterm_conformance.client import Client
    from agterm_conformance.plugin import Cli
    monkeypatch.syspath_prepend(str(Path(__file__).parents[1] / 'tests/cli'))
    from test_cli_only import _Relay
    root = Path(tempfile.mkdtemp(prefix='trace-relay-'))
    from dataclasses import dataclass
    @dataclass
    class App:
        root: Path
        socket: Path
        log: Path
        pid: int = 123
        def client(self): return Client(self.socket, self.root)
    app = App(root, root / 'real.sock', root / 'app.log')
    record = trace.Trace('relay', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    server = FakeServer(app.socket, reply({'ok': True, 'result': {}}))
    relay = None
    try:
        relay = _Relay(Cli(Path('/unused'), app))
        assert Client(relay.cli.instance.socket, root).request('tree').ok
    finally:
        if relay: relay.close()
        server.close()
        trace.ACTIVE.reset(token)
        shutil.rmtree(root)
    assert any(entry['socket'] == str(app.socket) for entry in record.logs['socket'])
