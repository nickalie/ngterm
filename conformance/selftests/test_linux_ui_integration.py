"""Opt-in real probe: CONFORMANCE_UI_PROBE_BIN and CONFORMANCE_UI_PROBE_ICD.

Uses only harness-owned displays, buses and state. Never selects the user's X display.
"""
import os
from contextlib import closing, ExitStack
from pathlib import Path

import pytest
from agterm_conformance import linux_ui
from agterm_conformance.impls import base
from agterm_conformance.impls.rust import RustAdapter
from agterm_conformance.match import assert_ok
from agterm_conformance.wait import wait_until

pytestmark = pytest.mark.skipif(not os.environ.get('CONFORMANCE_UI_PROBE_BIN'), reason='opt-in real Linux UI probe')


@pytest.fixture(scope='module')
def a11y_run():
    # libatspi owns a process-global connection: every probe in this pytest run shares one bus.
    run = linux_ui.A11yRun().start()
    helpers = [run.session_bus, run.launcher, run.registry]
    root = run.root
    try:
        yield run
    finally:
        run.close()
    assert all(p.poll() is not None for p in helpers)
    assert not root.exists()


def test_two_real_trees_relaunch_focus_and_cleanup(monkeypatch, real_compositor, a11y_run):
    from Xlib import X, XK, display
    from Xlib.ext import xtest
    monkeypatch.setenv(base.LAVAPIPE_ICD_VARIABLE, os.environ['CONFORMANCE_UI_PROBE_ICD'])
    run = a11y_run
    adapter = RustAdapter(Path(os.environ['CONFORMANCE_UI_PROBE_BIN']))
    apps = []
    helpers = []
    roots = []
    try:
        for _ in range(2):
            app = adapter.launch(ui=run)
            apps.append(app)
            roots.append(app.root)
            helpers.extend((app.compositor, app.session_bus, app.ui.status))
        assert apps[0].env['DISPLAY'] != apps[1].env['DISPLAY']
        assert apps[0].env['DBUS_SESSION_BUS_ADDRESS'] != apps[1].env['DBUS_SESSION_BUS_ADDRESS']
        def trees():
            desktop = run.atspi.get_desktop(0)
            nodes = [desktop.get_child_at_index(i) for i in range(desktop.get_child_count())]
            found = {node.get_process_id(): node for node in nodes}
            assert {a.pid for a in apps} <= found.keys()
            for app in apps:
                assert found[app.pid].get_child_at_index(0).get_child_count() > 0
            return True
        wait_until(trees, timeout=20)
        context = apps[0].ui
        apps[0] = adapter.relaunch(apps[0])
        assert apps[0].ui is context
        wait_until(trees, timeout=20)
        app = apps[0]
        before = assert_ok(app.request('tree'))['tree']['sidebarVisible']
        with closing(display.Display(app.ui.display)) as connection:
            def window():
                windows = [w for w in connection.screen().root.query_tree().children
                           if w.get_attributes().map_state == X.IsViewable]
                assert windows
                return windows[0]
            target = wait_until(window, timeout=10)
            target.set_input_focus(X.RevertToParent, X.CurrentTime)
            geometry = target.get_geometry()
            xtest.fake_input(connection, X.MotionNotify, x=geometry.x + geometry.width * 3 // 4,
                             y=geometry.y + geometry.height // 2)
            xtest.fake_input(connection, X.ButtonPress, 1)
            xtest.fake_input(connection, X.ButtonRelease, 1)
            connection.sync()
            keys = [connection.keysym_to_keycode(XK.string_to_keysym(name))
                    for name in ('Control_L', 'Shift_L', 's')]
            for key in keys:
                xtest.fake_input(connection, X.KeyPress, key)
            for key in reversed(keys):
                xtest.fake_input(connection, X.KeyRelease, key)
            connection.sync()
            def toggled():
                assert assert_ok(app.request('tree'))['tree']['sidebarVisible'] is not before
                return True
            wait_until(toggled, timeout=15)
    finally:
        with ExitStack() as cleanup:
            for app in apps:
                cleanup.callback(adapter.stop, app)
    assert all(p.poll() is not None for p in helpers)
    assert all(not root.exists() for root in roots)


@pytest.mark.parametrize('scale', [1, 2])
def test_accessibility_screen_bounds_follow_x11_window(monkeypatch, real_compositor, a11y_run, scale):
    """AccessKit root bounds track real initial, moved and resized X11 geometry, including HiDPI."""
    from Xlib import X, display
    monkeypatch.setenv(base.LAVAPIPE_ICD_VARIABLE, os.environ['CONFORMANCE_UI_PROBE_ICD'])
    monkeypatch.setenv('GPUI_X11_SCALE_FACTOR', str(scale))
    with ExitStack() as cleanup:
        run = a11y_run
        adapter = RustAdapter(Path(os.environ['CONFORMANCE_UI_PROBE_BIN']))
        app = adapter.launch(ui=run)
        cleanup.callback(adapter.stop, app)
        connection = cleanup.enter_context(closing(display.Display(app.ui.display)))

        def mapped():
            windows = [w for w in connection.screen().root.query_tree().children
                       if w.get_attributes().map_state == X.IsViewable]
            assert len(windows) == 1
            return windows[0]

        window = wait_until(mapped, timeout=10)

        def accessible_window():
            desktop = run.atspi.get_desktop(0)
            for i in range(desktop.get_child_count()):
                node = desktop.get_child_at_index(i)
                if node.get_process_id() == app.pid and node.get_child_count():
                    return node.get_child_at_index(0)
            raise AssertionError('application accessibility window not registered')

        accessible = wait_until(accessible_window, timeout=15)

        def matches_geometry():
            geometry = window.get_geometry()
            origin = connection.screen().root.translate_coords(window, 0, 0)
            actual = accessible.get_extents(run.atspi.CoordType.SCREEN)
            assert (actual.x, actual.y, actual.width, actual.height) == (
                origin.x, origin.y, geometry.width, geometry.height)
            def descendants(node):
                for i in range(node.get_child_count()):
                    child = node.get_child_at_index(i)
                    yield child
                    yield from descendants(child)
            controls = [node for node in descendants(accessible) if node.get_accessible_id()]
            assert controls, 'no authored accessibility control'
            control = controls[0]
            screen = control.get_extents(run.atspi.CoordType.SCREEN)
            local = control.get_extents(run.atspi.CoordType.WINDOW)
            assert local.width > 0 and local.height > 0
            assert (screen.x - local.x, screen.y - local.y) == (origin.x, origin.y)
            assert (screen.width, screen.height) == (local.width, local.height)
            return True

        wait_until(matches_geometry, timeout=10)
        for change in ({'x': 137, 'y': 89}, {'x': 211, 'y': 123, 'width': 760, 'height': 500},
                       {'width': 810, 'height': 540}):
            window.configure(**change)
            connection.sync()
            wait_until(matches_geometry, timeout=10)

        # A reparenting window manager gives real ConfigureNotify parent-relative coordinates.
        root = connection.screen().root
        frame = root.create_window(61, 47, 1000, 800, 0, connection.screen().root_depth)
        try:
            frame.map()
            window.reparent(frame, 23, 31)
            window.map()
            connection.sync()
            # ReparentNotify alone must refresh the origin before a ConfigureNotify arrives.
            wait_until(matches_geometry, timeout=10)
            window.configure(x=37, y=43, width=780, height=510)
            connection.sync()
            wait_until(matches_geometry, timeout=10)
        finally:
            window.reparent(root, 211, 123)
            frame.destroy()
            connection.sync()
