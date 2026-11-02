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


def window_driver():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.atspi import AtspiDriver
    class Window:
        def __init__(self, pid=None, x=0):
            self.pid, self.x, self.children, self.parent = pid, x, [], None
        def get_full_property(self, *args): return NS(value=[self.pid]) if self.pid else None
        def get_attributes(self): return NS(map_state=2)
        def query_tree(self): return NS(children=self.children, parent=self.parent)
        def translate_coords(self, window, *args): return NS(x=window.x, y=0)
        def get_geometry(self): return NS(width=100, height=100)
        def get_full_text_property(self, *args): return self.get_wm_name()
        def set_input_focus(self, *args): connection.focus = self
    root, first, second, child, foreign = Window(), Window(42), Window(42, 200), Window(), Window(99)
    root.children = [first, second, foreign]
    for w in root.children: w.parent = root
    second.children = [child]
    child.parent = second
    connection = NS(focus=second, intern_atom=lambda name:1, screen=lambda:NS(root=root),
                    sync=lambda:None, close=lambda:None)
    connection.get_input_focus = lambda: NS(focus=connection.focus)
    driver = object.__new__(AtspiDriver)
    driver.pid, driver.connection = 42, connection
    driver.instance = NS(ui=NS(check=lambda *args:None), env={}, root=None)
    driver.X = NS(IsViewable=2, RevertToParent=2, CurrentTime=0)
    driver.Xatom = NS(CARDINAL=1)
    driver.atspi = NS(CoordType=NS(SCREEN=0), StateType=NS(ACTIVE='active'))
    driver._native_error = OSError
    driver._wait = lambda fn, *args: fn()
    return driver, first, second, child, foreign


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
    frame = NS(get_role_name=lambda:'frame', get_extents=lambda kind:NS(x=200,y=0,width=100,height=100))
    seen = []
    handle = NS(get_role_name=lambda:'entry', get_parent=lambda:frame,
                grab_focus=lambda:seen.append(driver.connection.focus) or True)
    driver._focus(Element('entry','text-field',handle=handle))
    assert seen == [second]


def test_element_focus_disambiguates_overlapping_windows_by_title():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    second.x = first.x
    first.get_wm_name = lambda:'First'
    second.get_wm_name = lambda:'Second'
    frame = NS(get_role_name=lambda:'frame', get_name=lambda:'Second',
               get_state_set=lambda:NS(contains=lambda state:False),
               get_extents=lambda kind:NS(x=0,y=0,width=100,height=100))
    assert driver._window(Element(None,'window',handle=frame)) is second
    # Identical geometry and title is ambiguous, so refuse instead of focusing the wrong window.
    first.get_wm_name = lambda:'Second'
    assert driver._window(Element(None,'window',handle=frame)) is None
    frame.get_state_set = lambda:NS(contains=lambda state:True)
    assert driver._window(Element(None,'window',handle=frame)) is second


@pytest.mark.parametrize('same_title', [False, True])
@pytest.mark.parametrize('reverse', [False, True])
def test_menu_uses_focused_window_independent_of_accessibility_order(reverse, same_title):
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, second, _, _ = window_driver()
    second.x = first.x
    first.get_wm_name = lambda: 'Same' if same_title else 'First'
    second.get_wm_name = lambda: 'Same' if same_title else 'Second'
    def frame(title):
        return NS(get_role_name=lambda:'frame', get_name=lambda:'Same' if same_title else title,
                  get_state_set=lambda:NS(contains=lambda state:title == 'Second'),
                  get_extents=lambda kind:NS(x=0,y=0,width=100,height=100))
    first_frame, second_frame = frame('First'), frame('Second')
    buttons = [Element('app-menu-button', 'button', handle=NS(get_parent=lambda:first_frame)),
               Element('app-menu-button', 'button', handle=NS(get_parent=lambda:second_frame))]
    driver._containing_frame = lambda el: el.handle.get_parent()
    driver._elements = lambda key: list(reversed(buttons)) if reverse else buttons
    driver._menu_path = None
    menus, actions = [], []
    popup = object()
    target = Element(None, 'menu-item', 'File', handle=object())
    driver._menus = lambda scope: list(menus) if scope is second_frame else []
    driver._scoped_menu_item = lambda scope, name: target if scope is popup else None
    second.configure = lambda **kw: actions.append(('raise', kw))
    driver.X.Above = 0
    driver.mouse_click = lambda el: (actions.append(('click',el)), menus.append(popup))
    assert driver._menu_command(('File',)) is target
    assert driver._menu_window is second_frame
    assert actions == [('raise', {'stack_mode':0}), ('click', buttons[1])]
    assert driver.connection.focus is second


def test_menu_button_refuses_ambiguous_overlapping_frames():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, _, second, _, _ = window_driver()
    second.get_wm_name = lambda: 'Same'
    frame = NS(get_name=lambda:'Same', get_state_set=lambda:NS(contains=lambda state:False),
               get_extents=lambda kind:NS(x=200,y=0,width=100,height=100))
    driver._elements = lambda key: [Element(key,'button'), Element(key,'button')]
    driver._containing_frame = lambda el: frame
    assert driver._menu_button(second) is None


def test_menu_waits_for_active_frame_and_reads_utf8_window_title():
    from types import SimpleNamespace as NS
    from agterm_conformance.drivers.base import Element
    driver, first, _, _, _ = window_driver()
    frame = NS(get_state_set=lambda:NS(contains=lambda state:False),
               get_extents=lambda kind:NS(x=0,y=0,width=100,height=100))
    driver._elements = lambda key: [Element(key,'button')]
    driver._containing_frame = lambda el:frame
    assert driver._menu_button(first) is None
    first.get_full_text_property = lambda *args:'home — Second'
    first.get_wm_name = lambda:'wrong legacy decoding'
    assert driver._window_title(first) == 'home — Second'
    first.get_full_text_property = lambda *args:None
    assert driver._window_title(first) == 'wrong legacy decoding'
