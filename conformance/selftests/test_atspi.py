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
    items, actions = {}, []
    driver._elements = lambda key: [button]
    driver._menu_item = lambda title: items.get(title)
    driver.mouse_click = lambda el: actions.append(('click',el))
    driver._hover = lambda el: actions.append(('hover',el))
    path = ('View','Find…')
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert actions == [('click',button)]
    items['View'] = submenu
    assert driver._menu_command(path) is None
    assert driver._menu_command(path) is None
    assert actions == [('click',button),('hover',submenu)]
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
