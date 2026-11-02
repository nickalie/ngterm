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


def test_two_real_trees_relaunch_focus_and_cleanup(monkeypatch, real_compositor):
    from Xlib import X, XK, display
    from Xlib.ext import xtest
    monkeypatch.setenv(base.LAVAPIPE_ICD_VARIABLE, os.environ['CONFORMANCE_UI_PROBE_ICD'])
    run = linux_ui.A11yRun().start()
    adapter = RustAdapter(Path(os.environ['CONFORMANCE_UI_PROBE_BIN']))
    apps = []
    helpers = [run.session_bus, run.launcher, run.registry]
    roots = [run.root]
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
            cleanup.callback(run.close)
            for app in apps:
                cleanup.callback(adapter.stop, app)
    assert all(p.poll() is not None for p in helpers)
    assert all(not root.exists() for root in roots)
