"""Private X11/a11y lifecycle and launch selection, without GI or an X server."""
import os
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest
from agterm_conformance.impls import base
from agterm_conformance.impls.rust import RustAdapter
from agterm_conformance import linux_ui, plugin


class FakeAtspi:
    initialized = False
    def is_initialized(self): return self.initialized
    def init(self): self.initialized = True; return 0
    def exit(self): self.initialized = False


@pytest.fixture
def ui_run(tmp_path, monkeypatch):
    run = linux_ui.A11yRun()
    run.root = tmp_path
    run.address = 'unix:path=' + str(tmp_path / 'a11y')
    run.active = True
    monkeypatch.setattr(linux_ui, '_active_run', run)
    return run


def test_init_refuses_foreign_or_preinitialized_bus(ui_run, monkeypatch):
    atspi = FakeAtspi()
    monkeypatch.setenv('AT_SPI_BUS_ADDRESS', 'unix:path=/users/bus')
    with pytest.raises(base.LaunchRefused, match='run'):
        ui_run.init_atspi(atspi)
    monkeypatch.setenv('AT_SPI_BUS_ADDRESS', ui_run.address)
    atspi.initialized = True
    with pytest.raises(base.LaunchRefused, match='initialized'):
        ui_run.init_atspi(atspi)
    atspi.initialized = False
    assert ui_run.init_atspi(atspi) is atspi
    assert ui_run.init_atspi(atspi) is atspi


def test_x11_env_allows_only_owned_display_and_bus(ui_run, tmp_path):
    root = tmp_path / 'instance'
    adapter = RustAdapter(None)
    env = adapter.build_env(root)
    context = linux_ui.X11Context(ui_run, ':789', root)
    context.apply(env)
    base.check_launch_env(env, root, True, True, True, base.RUST_PREFIX, ui=context)
    assert 'WAYLAND_DISPLAY' not in env
    assert env['GDK_BACKEND'] == 'x11'
    assert env['GSETTINGS_BACKEND'] == 'memory'
    for key, value in [('DISPLAY', ':0'), ('AT_SPI_BUS_ADDRESS', 'unix:path=/foreign'), ('GDK_BACKEND', 'wayland')]:
        with pytest.raises(base.LaunchRefused):
            base.check_launch_env({**env, key: value}, root, True, True, True, base.RUST_PREFIX, ui=context)
    ui_run.active = False
    with pytest.raises(base.LaunchRefused):
        context.apply(env)


def test_layer_selection_uses_path_not_fixture_name(tmp_path):
    assert plugin.is_ui_path(tmp_path / 'tests/ui/test_any.py')
    assert not plugin.is_ui_path(tmp_path / 'selftests/test_ui.py')
    assert not plugin.is_ui_path(tmp_path / 'tests/protocol/test_ui.py')


def test_ui_nonheadless_refused_before_root(fake_bin, monkeypatch):
    monkeypatch.setenv(base.DISPLAY_MODE_VARIABLE, 'session')
    monkeypatch.setattr(base, 'make_root', lambda: pytest.fail('root created'))
    with pytest.raises(base.LaunchRefused, match='headless'):
        RustAdapter(fake_bin).launch(ui=object())


@pytest.mark.parametrize('missing', ['Xvfb', 'gdbus', 'dbus-daemon', 'at-spi-bus-launcher', 'at-spi2-registryd'])
def test_setup_names_missing_program(missing, monkeypatch):
    monkeypatch.setattr(linux_ui.shutil, 'which', lambda name: None if name == missing else '/bin/' + name)
    monkeypatch.setattr(linux_ui, 'LIBEXEC_DIRS', ())
    with pytest.raises(base.LaunchError, match=missing):
        linux_ui.check_setup(check_bindings=False)


def test_libexec_program_discovery(tmp_path, monkeypatch):
    program = tmp_path / 'at-spi-bus-launcher'
    program.write_text(''); program.chmod(0o755)
    monkeypatch.setattr(linux_ui.shutil, 'which', lambda name: None)
    monkeypatch.setattr(linux_ui, 'LIBEXEC_DIRS', (str(tmp_path),))
    assert linux_ui.program('at-spi-bus-launcher') == str(program)


@pytest.fixture
def fake_xvfb(tmp_path, monkeypatch):
    executable = tmp_path / 'Xvfb'
    executable.write_text(f'#!{sys.executable}\n' + Path(__file__).with_name('fake_xvfb.py').read_text())
    executable.chmod(0o755)
    original = linux_ui.program
    monkeypatch.setattr(linux_ui, 'start_status', lambda root, env, timeout: base._spawn(
        [sys.executable, '-c', 'import signal; signal.pause()'], env, root / 'status.log', 'wb'))
    monkeypatch.setattr(linux_ui, 'program', lambda name: str(executable) if name == 'Xvfb' else original(name))
    return executable


def test_ui_instances_have_distinct_displays_and_keep_them_on_relaunch(ui_run, fake_xvfb, fake_bin, roots, monkeypatch):
    monkeypatch.setenv('DISPLAY', ':0')
    monkeypatch.setenv('AT_SPI_BUS_ADDRESS', 'unix:path=/desktop')
    monkeypatch.setenv('NGTERM_SOCKET', '/desktop/socket')
    monkeypatch.setenv('AGTERM_SESSION_ID', 'user')
    monkeypatch.setenv('ZMX_DIR', '/desktop/zmx')
    adapter = RustAdapter(fake_bin)
    first = adapter.launch(ui=ui_run)
    second = None
    try:
        second = adapter.launch(ui=ui_run)
        assert first.ui.display != second.ui.display
        assert first.env['AT_SPI_BUS_ADDRESS'] == second.env['AT_SPI_BUS_ADDRESS'] == ui_run.address
        assert first.env['DBUS_SESSION_BUS_ADDRESS'] != second.env['DBUS_SESSION_BUS_ADDRESS']
        assert not {'NGTERM_SOCKET', 'AGTERM_SESSION_ID', 'ZMX_DIR', 'WAYLAND_DISPLAY'} & first.env.keys()
        process, bus, context = first.compositor, first.session_bus, first.ui
        first = adapter.relaunch(first)
        assert first.compositor is process and first.session_bus is bus and first.ui is context
        assert context.status.poll() is None
        assert first.env['DISPLAY'] == context.display
    finally:
        adapter.stop(first)
        if second:
            adapter.stop(second)
    assert process.poll() is not None and bus.poll() is not None
    assert context.status.poll() is not None
    assert not first.root.exists() and not second.root.exists()


def test_xvfb_failure_reaps_bus_and_root(ui_run, fake_bin, roots, monkeypatch):
    monkeypatch.setattr(linux_ui, 'program', lambda name: '/bin/false')
    with pytest.raises(base.LaunchError, match='Xvfb'):
        RustAdapter(fake_bin).launch(ui=ui_run)
    assert all(not root.exists() for root in roots)


def test_missing_typelib_names_setup_step(monkeypatch):
    monkeypatch.setitem(sys.modules, 'gi', None)
    with pytest.raises(base.LaunchError, match='Atspi-2.0 typelib'):
        linux_ui.atspi_bindings()


def test_launch_fixture_options_cannot_be_overridden():
    calls = []
    adapter = SimpleNamespace(launch=lambda **kw: calls.append(kw) or SimpleNamespace())
    launcher = plugin.Launcher(adapter, ui='owned')
    launcher(ui='foreign')
    assert calls == [{'ui': 'owned'}]


def test_request_module_selects_ui_launch_context(ui_run, tmp_path):
    config = SimpleNamespace(stash={plugin._A11Y_KEY: ui_run})
    request = SimpleNamespace(config=config, node=SimpleNamespace(path=tmp_path / 'tests/ui/test_rows.py'))
    assert plugin.launch_options(request) == {'ui': ui_run}
    request.node.path = tmp_path / 'tests/protocol/test_rows.py'
    assert plugin.launch_options(request) == {}


@pytest.mark.parametrize('fail', [False, True])
def test_owned_run_starts_and_reaps_launcher_bus_and_registry(tmp_path, monkeypatch, roots, fail):
    original_spawn = base._spawn
    processes = []
    script = Path(__file__).with_name('fake_a11y_bus.py').resolve()
    def spawn(command, env, log, mode):
        if 'launcher' in command[0]:
            command = [sys.executable, str(script)]
        else:
            command = [sys.executable, '-c', 'import signal; signal.pause()']
        process = original_spawn(command, env, log, mode)
        processes.append(process)
        return process
    monkeypatch.delenv('AT_SPI_BUS_ADDRESS', raising=False)
    monkeypatch.setattr(linux_ui, 'check_setup', lambda: None)
    monkeypatch.setattr(linux_ui, 'atspi_bindings', FakeAtspi)
    monkeypatch.setattr(linux_ui, 'program', lambda name: name)
    monkeypatch.setattr(base, '_spawn', spawn)
    monkeypatch.setattr(base, 'start_session_bus', lambda root, env, timeout: spawn(['bus'], env, root / 'bus.log', 'wb'))
    def call(env, address, dest, path, method, *args):
        if fail and method.endswith('GetNameOwner'):
            raise base.LaunchError('registry failed')
        if method.endswith('GetAddress'):
            from agterm_conformance.wait import wait_until
            def child_ready():
                assert Path(env['XDG_RUNTIME_DIR'], 'child.pid').exists()
                return True
            wait_until(child_ready, timeout=5)
            return repr(('unix:path=' + env['XDG_RUNTIME_DIR'] + '/a11y',))
        return "(':1.0',)"
    monkeypatch.setattr(linux_ui, '_call', call)
    run = linux_ui.A11yRun()
    if fail:
        with pytest.raises(base.LaunchError, match='registry failed'):
            run.start(timeout=0.1)
        assert all(p.poll() is not None for p in processes)
        assert not run.root.exists()
        assert linux_ui._active_run is None
        return
    run.start()
    child_pid = int((run.root / 'run/child.pid').read_text())
    assert os.environ['AT_SPI_BUS_ADDRESS'] == run.address
    run.close()
    assert all(p.poll() is not None for p in processes)
    assert not Path(f'/proc/{child_pid}').exists()
    assert not run.root.exists()
    assert 'AT_SPI_BUS_ADDRESS' not in os.environ
    assert linux_ui._active_run is None
    run.close()  # teardown is idempotent


def test_collection_setup_failure_is_exit_two(pytester, fake_bin, monkeypatch):
    monkeypatch.setattr(linux_ui.A11yRun, 'start', lambda self: (_ for _ in ()).throw(base.LaunchError('missing Xvfb')))
    directory = pytester.path / 'tests/ui'
    directory.mkdir(parents=True)
    (directory / 'test_setup.py').write_text('def test_ui(impl_options): pass\n')
    result = pytester.runpytest('tests/ui', '--impl', 'rust', '--bin', str(fake_bin))
    assert result.ret == 2
    assert 'missing Xvfb' in result.stdout.str() + result.stderr.str()


def test_module_and_function_launches_follow_ui_layer(pytester, fake_bin, fake_xvfb, ui_run, monkeypatch):
    import json
    monkeypatch.setenv('FAKE_VERSION', json.dumps({'conformance': {'parityLevel': '0.39.1', 'capabilities': []}}))
    monkeypatch.setattr(linux_ui.A11yRun, 'start', lambda self: ui_run)
    monkeypatch.setattr(ui_run, 'close', lambda: None)
    directory = pytester.path / 'tests/ui'
    directory.mkdir(parents=True)
    (directory / 'test_launch.py').write_text('''
import pytest
@pytest.fixture(scope='module')
def seeded(shared_app):
    assert shared_app.ui is not None
    return shared_app

def test_module(seeded, ctl):
    assert ctl.instance is seeded
    assert seeded.env['GDK_BACKEND'] == 'x11'

def test_extra(launch):
    first = launch()
    assert first.instance.ui is not None
    context = first.instance.ui
    assert launch.relaunch(first).instance.ui is context

def test_function(app):
    assert app.ui is not None
''')
    pytester.runpytest('tests/ui', '--impl', 'rust', '--bin', str(fake_bin)).assert_outcomes(passed=3)
