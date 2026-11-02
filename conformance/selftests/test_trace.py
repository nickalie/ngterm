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


def test_action_success_failure_and_tree_dump(tmp_path):
    from test_drivers import FakeDriver
    from agterm_conformance.drivers.base import UiIds, Element
    record = trace.Trace('actions', tmp_path / 'traces', 'always')
    token = trace.ACTIVE.set(record)
    try:
        driver = FakeDriver([])
        driver.press('a')
        with pytest.raises(ValueError): driver.press('not-a-key')
        assert driver.tree_dump().endswith('\n')
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
    trace.event('socket', request='outside test')
    assert record.logs['socket'] == []


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


def test_released_trace_drops_in_memory_evidence(tmp_path):
    record = trace.Trace('released', tmp_path / 'traces', 'on-failure')
    record.remember(instance(tmp_path))
    record.record('socket', {'large': 'payload'})
    record.capture()
    record.release()
    assert record.instances == {} and record.files == {}
    assert record.logs == {'actions': [], 'socket': [], 'cli': []}
