"""AT-SPI mapping and input checks need no GI or X server."""
import subprocess
import sys

import pytest
from agterm_conformance.drivers.base import NAMED_KEYS, PRINTABLE_KEYS, ROLES, Chord


def test_lazy_import():
    result = subprocess.run([sys.executable, '-c',
        "import sys; import agterm_conformance.drivers.atspi; assert 'gi' not in sys.modules; assert 'Xlib' not in sys.modules"],
        capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


def test_role_mapping():
    from agterm_conformance.drivers.atspi import ROLE_NAMES
    assert set(ROLE_NAMES.values()) <= ROLES
    assert ROLE_NAMES['push button'] == 'button'
    assert ROLE_NAMES['check box'] == 'checkbox'
    assert ROLE_NAMES['entry'] == 'text-field'


def test_key_mapping():
    from agterm_conformance.drivers.atspi import key_symbol, text_symbol, modifier_symbols
    assert all(key_symbol(key) for key in NAMED_KEYS | PRINTABLE_KEYS)
    assert key_symbol('delete') == 0xff08
    assert key_symbol('forward-delete') == 0xffff
    assert text_symbol('A') == (ord('a'), True)
    assert text_symbol('!') == (ord('1'), True)
    assert text_symbol('é') == (0xe9, False)
    assert text_symbol('界') == (0x1000000 | ord('界'), False)
    assert modifier_symbols(Chord('a', frozenset({'cmd', 'shift'}))) == [0xffe1, 0xffe3]
    assert modifier_symbols(Chord('a', frozenset({'cmd', 'ctrl'}))) == [0xffe3]


def test_refuses_instance_before_importing_bindings():
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import DriverError
    with pytest.raises(DriverError, match='matching instance'):
        AtspiDriver(42)


def test_snapshot_reads_portable_values():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    driver = object.__new__(AtspiDriver)
    driver.atspi = NS(StateType=NS(CHECKED='checked', SELECTED='selected', ENABLED='enabled'))
    class Handle:
        def get_state_set(self): return NS(contains=lambda state: state in {'checked', 'enabled'})
        def get_role_name(self): return 'check box'
        def get_interfaces(self): return []
        def get_accessible_id(self): return 'check'
        def get_name(self): return 'A checkbox'
    el = driver.snapshot(Handle())
    assert (el.ui_id, el.role, el.name, el.value, el.enabled) == ('check', 'checkbox', 'A checkbox', 1, True)
    handle = Handle()
    handle.get_role_name = lambda: 'text'
    handle.get_interfaces = lambda: ['Text', 'EditableText']
    handle.get_text = lambda start, end: 'hello'
    assert driver.snapshot(handle).role == 'text-field'
    assert driver.snapshot(handle).value == 'hello'
    handle.get_interfaces = lambda: ['Value']
    handle.get_current_value = lambda: 0.5
    assert driver.snapshot(handle).value == 0.5


def input_driver():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    driver = object.__new__(AtspiDriver)
    events = []
    driver.X = NS(KeyPress=2, KeyRelease=3, ButtonPress=4, ButtonRelease=5, MotionNotify=6)
    driver.xtest = NS(fake_input=lambda connection, *args, **kw: events.append((args, kw)))
    driver.connection = NS(sync=lambda: None, close=lambda: None)
    driver.instance = NS(ui=NS(check=lambda *args: None), env={}, root=None)
    driver._keycode = lambda symbol: symbol
    driver._activate = lambda: None
    return driver, events


def test_chords_release_modifiers_and_shifted_typing():
    driver, events = input_driver()
    driver._post_chord(Chord('a', frozenset({'cmd', 'shift'})))
    assert [args for args, _ in events] == [(2,0xffe1),(2,0xffe3),(2,97),(3,97),(3,0xffe3),(3,0xffe1)]
    events.clear()
    driver._type_text('A!é界')
    assert [args for args, _ in events if args[0] == 2] == [(2,0xffe1),(2,97),(2,0xffe1),(2,49),(2,233),(2,0x100754c)]


def test_pointer_and_wheel_event_directions():
    driver, events = input_driver()
    driver._pointer_move(10.4, 20.7)
    driver._pointer_button('right', True)
    driver._pointer_button('right', False)
    driver._wheel(-1, 2)
    assert events[0] == ((6,), {'x':10, 'y':21})
    assert [args for args, _ in events[1:]] == [(4,3),(5,3),(4,5),(5,5),(4,5),(5,5),(4,6),(5,6)]


def test_actions_and_native_read_errors():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import DriverError, Element, ReadFailed
    driver = object.__new__(AtspiDriver)
    driver._native_error = OSError
    driver.pid = 42
    with pytest.raises(ReadFailed, match='42'):
        driver._read(lambda: (_ for _ in ()).throw(OSError('gone')))
    called = []
    handle = NS(get_n_actions=lambda: 1, get_action_name=lambda i:'click', do_action=lambda i:called.append(i) or True)
    driver._press_element(Element('x','button',handle=handle))
    assert called == [0]
    handle.get_n_actions = lambda: 0
    with pytest.raises(DriverError, match='no press action'):
        driver._press_element(Element('x','button',handle=handle))


def test_menu_open_and_hover_are_not_retried_before_readback():
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import Element
    driver = object.__new__(AtspiDriver)
    driver._menu_path = None
    driver._menu_depth = 0
    button = Element('app-menu-button', 'button')
    submenu = Element(None, 'menu-item', 'View')
    final = Element(None, 'menu-item', 'Find…')
    items, actions, menus = {}, [], []
    driver._elements = lambda key: [button]
    driver.app = object()
    driver._containing_frame = lambda el: driver.app
    driver._window = lambda: object()
    driver._menu_button = lambda window: (button, driver.app)
    driver._activate_menu_window = lambda: None
    driver._menus = lambda scope: list(menus)
    driver._scoped_menu_item = lambda scope, title: items.get(title)
    driver.mouse_click = lambda el: actions.append(('click',el))
    driver._hover = lambda el: actions.append(('hover',el))
    path = ('View','Find…')
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert actions == [('click',button)]
    menus.append(object())
    items['View'] = submenu
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert actions == [('click',button),('hover',submenu)]
    menus.append(object())
    items['Find…'] = final
    assert driver._menu_command(path) is final


def test_disappearing_child_is_retryable_not_an_empty_tree():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import ReadFailed
    driver = object.__new__(AtspiDriver)
    driver.pid = 7
    driver.app = NS(clear_cache=lambda: None, get_child_count=lambda: 1, get_child_at_index=lambda i: None)
    with pytest.raises(ReadFailed, match='tree changed'):
        list(driver.walk())


def test_unicode_mapping_uses_only_empty_private_keycodes():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import DriverError
    driver = object.__new__(AtspiDriver)
    changes = []
    driver.connection = NS(keysym_to_keycodes=lambda symbol: [(8, 2)], close=lambda: None, sync=lambda: None,
        display=NS(info=NS(min_keycode=8, max_keycode=9)),
        get_keyboard_mapping=lambda first,count: [(65,0),(0,0)],
        change_keyboard_mapping=lambda code,rows: changes.append((code,rows)))
    driver._mapped_symbols = {}
    assert driver._keycode(0x100754c) == 9
    assert driver._keycode(0x100754c) == 9
    assert changes == [(9, [(0x100754c, 0x100754c)])]
    with pytest.raises(DriverError, match='no unused keycode'):
        driver._keycode(0x100754d)


def test_menu_path_keeps_deferred_popup_scope_with_duplicate_names():
    from agterm_conformance.drivers.atspi import AtspiDriver
    from agterm_conformance.drivers.base import Element
    class Node:
        def __init__(self, name, role='menu item', children=()):
            self.name, self.role, self.children = name, role, list(children)
        def get_name(self): return self.name
        def get_role_name(self): return self.role
        def get_child_count(self): return len(self.children)
        def get_child_at_index(self, i): return self.children[i]
        def clear_cache(self): pass
    target, open_window, top = Node('File'), Node('Open Window'), Node('File')
    root_menu = Node('', 'menu', [top])
    first_popup = Node('', 'menu', [open_window])
    last_popup = Node('', 'menu', [target])
    frame = Node('', 'frame')
    # A popup in another window must never become the current popup.
    foreign = Node('', 'frame', [Node('', 'menu', [Node('File')])])
    driver = object.__new__(AtspiDriver)
    driver.app = Node('', 'application', [frame, foreign])
    driver._native_error = OSError
    driver._menu_path = None
    driver._elements = lambda key: [Element(key, 'button')]
    driver._containing_frame = lambda el: frame
    driver._window = lambda: object()
    driver._menu_button = lambda window: (Element('app-menu-button', 'button'), frame)
    driver._activate_menu_window = lambda: None
    driver.mouse_click = lambda el: frame.children.append(root_menu)
    driver.snapshot = lambda h: Element(None, 'menu-item', h.name, handle=h)
    hovered = []
    driver._hover = lambda el: hovered.append(el.handle)
    path = ('File', 'Open Window', 'File')
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert hovered == [top]
    frame.children.append(first_popup)
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert hovered == [top, open_window]
    # The ancestor File item remains visible, but cannot satisfy the final segment.
    frame.children.append(last_popup)
    assert driver._menu_command(path).handle is target


class Window:
    """A private-Xvfb top-level window: no WM, so configure moves it directly (parent is the root)."""
    def __init__(self, pid=None, x=0, y=0, title='home'):
        from types import SimpleNamespace as NS
        self.NS = NS
        self.pid, self.x, self.y, self.title, self.children, self.parent = pid, x, y, title, [], None
        self.configured, self.sticky, self.on_configure = [], False, None
        self.width = self.height = 100
    def get_full_property(self, *args): return self.NS(value=[self.pid]) if self.pid else None
    def get_attributes(self): return self.NS(map_state=2)
    def query_tree(self): return self.NS(children=self.children, parent=self.parent)
    def translate_coords(self, window, *args): return self.NS(x=window.x, y=window.y)
    def get_geometry(self): return self.NS(x=self.x, y=self.y, width=self.width, height=self.height)
    def get_wm_name(self): return self.title
    def get_full_text_property(self, *args): return self.get_wm_name()
    def set_input_focus(self, *args): self.connection.set_input_focus(self)
    def configure(self, **kw):
        self.configured.append(kw)
        if self.on_configure is not None:
            self.on_configure(self, kw)
        if not self.sticky or len(self.configured) == 1:
            self.x, self.y = kw.get('x', self.x), kw.get('y', self.y)
            self.width, self.height = kw.get('width', self.width), kw.get('height', self.height)
    def __repr__(self): return f'Window({self.x},{self.y})'


class Frame:
    """GPUI's frame: extents are its own window's root origin; ACTIVE as GPUI last processed it."""
    def __init__(self, window, name=None, active=lambda frame: False):
        from types import SimpleNamespace as NS
        self.NS, self.window, self.name, self.active = NS, window, name, active
    def get_role_name(self): return 'frame'
    def get_parent(self): return None
    def get_name(self): return self.window.title if self.name is None else self.name
    def get_state_set(self): return self.NS(contains=lambda state: self.active(self))
    def get_extents(self, kind):
        window = self.window
        return self.NS(x=window.x, y=window.y, width=window.width, height=window.height)


def window_driver():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    root, first, second, child, foreign = Window(), Window(42), Window(42, 200), Window(), Window(99, 400)
    root.children = [first, second, foreign]
    for w in root.children: w.parent = root
    second.children = [child]
    child.parent = second
    connection = NS(focus=second, intern_atom=lambda name:1, screen=lambda:NS(root=root),
                    sync=lambda:None, close=lambda:None, focus_calls=[])
    connection.get_input_focus = lambda: NS(focus=connection.focus)
    def set_focus(focus, *args):
        connection.focus_calls.append(focus)
        connection.focus = focus
    connection.set_input_focus = set_focus
    for w in (root, first, second, child, foreign):
        w.connection = connection
    driver = object.__new__(AtspiDriver)
    driver.pid, driver.connection = 42, connection
    driver.instance = NS(ui=NS(check=lambda *args:None), env={}, root=None)
    driver.X = NS(IsViewable=2, RevertToParent=2, RevertToNone=0, NONE=0, CurrentTime=0, Above=0)
    driver.Xatom = NS(CARDINAL=1)
    driver.atspi = NS(CoordType=NS(SCREEN=0), StateType=NS(ACTIVE='active'))
    driver._native_error = OSError
    driver._wait = lambda fn, *args: fn()
    driver.app = application([])
    return driver, first, second, child, foreign


def application(frames, poll=lambda: None):
    from types import SimpleNamespace as NS
    return NS(clear_cache=poll, get_child_count=lambda:len(frames), get_child_at_index=lambda i:frames[i])


def raises(window):
    return [kw for kw in window.configured if 'stack_mode' in kw]


def probes(window):
    return [(kw['x'], kw['y']) for kw in window.configured if 'stack_mode' not in kw]


def twin_windows(driver, first, second, reverse=False, queued=None):
    """Unnamed windows of one session share centered bounds and title; the frame has no X11 id."""
    second.x = first.x
    active = queued.active if queued else lambda frame: False
    frames = [Frame(first, active=active), Frame(second, active=active)]
    driver.app = application(frames[::-1] if reverse else frames, queued.step if queued else lambda: None)
    return frames


def test_keyboard_preserves_owned_focus_including_descendant():
    driver, first, second, child, foreign = window_driver()
    for focused in (second, child):
        driver.connection.focus = focused
        driver._activate()
        assert driver.connection.focus is second
    for focused in (foreign, 1):
        driver.connection.focus = focused
        driver._activate()
        assert driver.connection.focus is first


def test_element_focus_activates_containing_window():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    driver.connection.focus = first
    frame = Frame(second)
    seen = []
    handle = NS(get_role_name=lambda:'entry', get_parent=lambda:frame,
                grab_focus=lambda:seen.append(driver.connection.focus) or True)
    driver._focus(Element('entry','text-field',handle=handle))
    assert seen == [second]


def test_pointer_raises_and_focuses_the_target_window_not_the_focused_one():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    # The second window covers the first and holds focus; the target lives in the first.
    second.x = first.x
    first.title, second.title = 'First', 'Second'
    handle = NS(get_role_name=lambda:'push button', get_parent=lambda:Frame(first))
    driver._pointer_target(Element('sidebar-toggle-button', 'button', handle=handle))
    assert raises(first) == [{'stack_mode':0}] and first.configured == raises(first)
    assert driver.connection.focus is first


@pytest.mark.parametrize('reverse', [False, True])
@pytest.mark.parametrize('target', [0, 1])
def test_pointer_identifies_target_among_identical_windows_by_moving_one(reverse, target):
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    windows = (first, second)
    frames = twin_windows(driver, first, second, reverse)
    handle = NS(get_role_name=lambda:'push button', get_parent=lambda:frames[target])
    driver._pointer_target(Element('sidebar-toggle-button', 'button', handle=handle))
    assert [w for w in windows if raises(w)] == [windows[target]]
    # The probe never touches focus: only the raise focuses, and only the target's window.
    assert driver.connection.focus_calls == [windows[target]]
    for window in windows:
        assert (window.x, window.y, window.width, window.height) == (0, 0, 100, 100)
        # Position-only moves, each to a fresh origin and back.
        assert all(set(kw) == {'x', 'y'} for kw in window.configured if 'stack_mode' not in kw)


def real_wait(driver, timeout=0.05):
    """The driver's own wait, which raises ElementNotFound on timeout, with a short deadline."""
    import types
    from agterm_conformance.drivers.base import Driver
    driver.name, driver.timeout = 'atspi', timeout
    driver._present = lambda: ''
    driver._wait = types.MethodType(Driver._wait, driver)


class QueuedFocus:
    """GPUI handles FocusIn/FocusOut on its own thread: ACTIVE lags X focus by queued events."""
    def __init__(self, connection, published):
        self.connection, self.published, self.queue = connection, published, []
        set_focus = connection.set_input_focus
        def focus(window, *args):
            self.queue += [('out', connection.focus), ('in', window)]
            set_focus(window, *args)
        connection.set_input_focus = focus
    def step(self):
        # One queued event per accessibility poll: a poll can land between FocusOut and FocusIn.
        if self.queue:
            kind, window = self.queue.pop(0)
            self.published = window if kind == 'in' else None if self.published is window else self.published
    def active(self, frame):
        return self.published is frame.window


@pytest.mark.parametrize('reverse', [False, True])
@pytest.mark.parametrize('target', [0, 1])
def test_queued_focus_switch_cannot_pair_target_with_its_twin(reverse, target):
    """X focus just moved from the twin to the target; GPUI has processed none of it.

    An activation probe saw no frame ACTIVE after the twin's FocusOut, then the target's
    queued FocusIn, and credited that to whichever candidate it had just focused."""
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    real_wait(driver, 1.0)
    windows = (first, second)
    twin = windows[1 - target]
    queued = QueuedFocus(driver.connection, twin)
    driver.connection.focus = twin
    frames = twin_windows(driver, first, second, reverse, queued=queued)
    driver.connection.set_input_focus(windows[target])
    driver.connection.focus_calls.clear()
    handle = NS(get_role_name=lambda:'push button', get_parent=lambda:frames[target])
    assert driver._window(Element('sidebar-toggle-button', 'button', handle=handle)) is windows[target]
    assert driver.connection.focus_calls == [] and driver.connection.focus is windows[target]


def test_menu_button_pairs_window_with_its_frame_despite_queued_focus():
    from agterm_conformance.drivers.base import Element
    for reverse in (False, True):
        for target in (0, 1):
            driver, first, second, _, _ = window_driver()
            real_wait(driver, 1.0)
            windows = (first, second)
            queued = QueuedFocus(driver.connection, windows[1 - target])
            driver.connection.focus = windows[1 - target]
            frames = twin_windows(driver, first, second, reverse, queued=queued)
            driver.connection.set_input_focus(windows[target])
            buttons = [Element('app-menu-button', 'button', handle=frame) for frame in frames]
            driver._containing_frame = lambda el: el.handle
            driver._elements = lambda key: list(reversed(buttons)) if reverse else buttons
            assert driver._menu_button(windows[target]) == (buttons[target], frames[target])
            assert driver.connection.focus is windows[target]
            assert (windows[target].x, windows[target].y) == (0, 0)


def test_repeated_probes_never_reuse_a_position():
    driver, first, second, _, foreign = window_driver()
    frames = twin_windows(driver, first, second)
    # Origins already held by a window or frame are never probe positions.
    foreign.x = foreign.y = 1
    seen = []
    for _ in range(3):
        for target in (0, 1):
            assert driver._identify(frames[target], [first, second]) is (first, second)[target]
    for window in (first, second):
        seen += probes(window)[0::2]
        assert probes(window)[1::2] == [(0, 0)] * (len(probes(window)) // 2)
    assert len(seen) == len(set(seen)) == 9
    assert (1, 1) not in seen and (0, 0) not in seen


def test_probe_refuses_when_positions_are_exhausted():
    from agterm_conformance.drivers.atspi import INT16_MAX
    driver, first, second, _, _ = window_driver()
    frames = twin_windows(driver, first, second)
    driver._probe_next = INT16_MAX + 1
    assert driver._identify(frames[0], [first, second]) is None
    assert first.configured == second.configured == []


def test_probe_timeout_restores_position_and_focus():
    from agterm_conformance.drivers.base import ElementNotFound
    driver, first, second, _, _ = window_driver()
    real_wait(driver)
    frames = twin_windows(driver, first, second)
    # A frame that never publishes its window's move times out the probe.
    frozen = [Frame(first, name='home'), Frame(second, name='home')]
    for frame in frozen:
        frame.get_extents = lambda kind: frame.NS(x=0, y=0, width=100, height=100)
    driver.app = application(frozen)
    with pytest.raises(ElementNotFound):
        driver._identify(frozen[0], [first, second])
    assert (first.x, first.y, second.x, second.y) == (0, 0, 0, 0)
    assert probes(first)[-1] == (0, 0) and probes(second) == []
    assert driver.connection.focus_calls == [] and driver.connection.focus is second


def test_probe_error_restores_position():
    driver, first, second, _, _ = window_driver()
    frames = twin_windows(driver, first, second)
    def fail_on_probe(window, kw):
        if len(window.configured) == 1:
            window.x, window.y = kw['x'], kw['y']
            raise RuntimeError('X error during the probe')
    first.on_configure = fail_on_probe
    with pytest.raises(RuntimeError, match='X error'):
        driver._identify(frames[0], [first, second])
    assert (first.x, first.y) == (0, 0) and probes(first)[-1] == (0, 0)


def test_unconfirmed_restore_is_not_retryable():
    from agterm_conformance.drivers.atspi import ProbeRestoreFailed
    from agterm_conformance.drivers.base import DriverError
    driver, first, second, _, _ = window_driver()
    real_wait(driver)
    frames = twin_windows(driver, first, second)
    first.sticky = True
    with pytest.raises(ProbeRestoreFailed) as raised:
        driver._identify(frames[0], [first, second])
    # Not an AssertionError: an outer wait must not retry on moved geometry.
    assert isinstance(raised.value, DriverError) and not isinstance(raised.value, AssertionError)


def test_identification_refuses_when_the_probe_reaches_no_single_frame():
    driver, first, second, _, _ = window_driver()
    frames = twin_windows(driver, first, second)
    # Two frames that both report the moved window identify nothing.
    frozen = [frames[0], Frame(first), frames[1]]
    driver.app = application(frozen)
    assert driver._identify(frames[1], [first, second]) is None
    assert (first.x, first.y) == (0, 0)


def test_overlay_elements_resolve_to_their_top_level_frame():
    """GPUI popups and modals are dialog nodes inside the frame, not X11 windows of their own."""
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, _, second, _, _ = window_driver()
    app = NS(get_role_name=lambda:'application', get_parent=lambda:None)
    frame = NS(get_role_name=lambda:'frame', get_parent=lambda:app,
               get_extents=lambda kind:NS(x=200,y=0,width=100,height=100))
    dialog = NS(get_role_name=lambda:'dialog', get_parent=lambda:frame,
                get_extents=lambda kind:NS(x=250,y=30,width=40,height=60))
    menu = NS(get_role_name=lambda:'menu', get_parent=lambda:dialog)
    item = Element(None, 'menu-item', 'File', handle=NS(get_role_name=lambda:'menu item', get_parent=lambda:menu))
    assert driver._containing_frame(item) is frame
    assert driver._window(item) is second


def test_pointer_move_does_not_refocus_another_window():
    driver, events = input_driver()
    driver._activate = lambda *args: pytest.fail('a move must not activate the focused window')
    driver._pointer_move(1, 2)
    assert events == [((6,), {'x':1, 'y':2})]


def test_element_focus_disambiguates_overlapping_windows_by_title():
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    second.x = first.x
    first.title, second.title = 'First', 'Second'
    frame = Frame(second)
    assert driver._window(Element(None,'window',handle=frame)) is second
    assert first.configured == second.configured == []
    # A stale ACTIVE frame is never read as identity: twins are probed.
    first.title = 'Second'
    frames = [Frame(first, active=lambda f: True), frame]
    driver.app = application(frames)
    assert driver._window(Element(None,'window',handle=frame)) is second
    assert driver._window(Element(None,'window',handle=frames[0])) is first


@pytest.mark.parametrize('same_title', [False, True])
@pytest.mark.parametrize('reverse', [False, True])
def test_menu_uses_focused_window_independent_of_accessibility_order(reverse, same_title):
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    second.x = first.x
    first.title = 'Same' if same_title else 'First'
    second.title = 'Same' if same_title else 'Second'
    # A stale ACTIVE first frame must not be chosen for the focused second window.
    first_frame, second_frame = Frame(first, active=lambda f: True), Frame(second)
    driver.app = application([first_frame, second_frame])
    buttons = [Element('app-menu-button', 'button', handle=first_frame),
               Element('app-menu-button', 'button', handle=second_frame)]
    driver._containing_frame = lambda el: el.handle
    driver._elements = lambda key: list(reversed(buttons)) if reverse else buttons
    driver._menu_path = None
    menus, actions = [], []
    popup = object()
    target = Element(None, 'menu-item', 'File', handle=object())
    driver._menus = lambda scope: list(menus) if scope is second_frame else []
    driver._scoped_menu_item = lambda scope, name: target if scope is popup else None
    driver.mouse_click = lambda el: (actions.append(('click',el)), menus.append(popup))
    assert driver._menu_command(('File',)) is target
    assert driver._menu_window is second_frame
    assert raises(second) == [{'stack_mode':0}] and raises(first) == []
    assert actions == [('click', buttons[1])]
    assert driver.connection.focus is second and (second.x, second.y) == (0, 0)


def test_menu_button_refuses_ambiguous_overlapping_frames():
    from agterm_conformance.drivers.base import Element
    driver, _, second, _, _ = window_driver()
    second.title = 'Same'
    # Two buttons in one frame: the probe reaches one frame but two buttons.
    frame = Frame(second)
    driver.app = application([frame])
    driver._elements = lambda key: [Element(key,'button'), Element(key,'button')]
    driver._containing_frame = lambda el: frame
    assert driver._menu_button(second) is None
    assert (second.x, second.y) == (200, 0)


def test_menu_button_waits_for_frame_bounds_and_reads_utf8_window_title():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    # The window moved, but its frame still publishes the old origin: not yet a match.
    frame = Frame(first)
    frame.get_extents = lambda kind: NS(x=300, y=0, width=100, height=100)
    driver._elements = lambda key: [Element(key,'button')]
    driver._containing_frame = lambda el: frame
    assert driver._menu_button(first) is None
    # A frame of another title never serves this window, even at its bounds.
    driver._containing_frame = lambda el: Frame(first, name='Other')
    assert driver._menu_button(first) is None
    first.get_full_text_property = lambda *args:'home — Second'
    first.get_wm_name = lambda:'wrong legacy decoding'
    assert driver._window_title(first) == 'home — Second'
    first.get_full_text_property = lambda *args:None
    assert driver._window_title(first) == 'wrong legacy decoding'
