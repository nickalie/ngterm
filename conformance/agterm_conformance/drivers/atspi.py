"""Linux accessibility on the run's private AT-SPI bus; XTest on the instance's own Xvfb.

Bindings are lazy. Neither the desktop display nor the desktop accessibility bus is used.
The portable cmd modifier maps to Control, matching ngterm's Linux key bindings.
"""
from __future__ import annotations

from collections import deque
from typing import Any

from agterm_conformance.drivers.base import Chord, Driver, DriverError, Element, ReadFailed, UiIds
from agterm_conformance.wait import DEFAULT_TIMEOUT, WaitTimeout, wait_until

ROLE_NAMES = {
    'button': 'button', 'tree item': 'row', 'push button': 'button', 'push button menu': 'menu-button', 'toggle button': 'button',
    'menu item': 'menu-item', 'check menu item': 'menu-item', 'radio menu item': 'menu-item',
    'check box': 'checkbox', 'radio button': 'radio', 'combo box': 'popup', 'entry': 'text-field',
    'password text': 'text-field', 'text': 'text', 'static': 'text', 'label': 'text',
    'slider': 'slider', 'table row': 'row', 'list item': 'row', 'panel': 'group', 'section': 'group',
    'grouping': 'group', 'image': 'image', 'frame': 'window', 'window': 'window', 'dialog': 'window',
    'page tab': 'radio',
}
KEY_SYMBOLS = {
    'return': 0xff0d, 'enter': 0xff8d, 'escape': 0xff1b, 'tab': 0xff09, 'space': 0x20,
    'delete': 0xff08, 'forward-delete': 0xffff, 'left': 0xff51, 'up': 0xff52, 'right': 0xff53,
    'down': 0xff54, 'home': 0xff50, 'end': 0xff57, 'page-up': 0xff55, 'page-down': 0xff56,
    **{f'f{i}': 0xffbd + i for i in range(1, 13)},
}
SHIFTED = dict(zip('~!@#$%^&*()_+{}|:"<>?', '`1234567890-=[]\\;\',./'))
MOD_SYMBOLS = {'shift': 0xffe1, 'ctrl': 0xffe3, 'cmd': 0xffe3, 'alt': 0xffe9}
INT16_MAX = 0x7fff


class ProbeRestoreFailed(DriverError):
    """A window moved to identify its frame is not confirmed back; nothing may retry on it."""


def key_symbol(key: str) -> int:
    return KEY_SYMBOLS[key] if key in KEY_SYMBOLS else ord(key)


def text_symbol(char: str) -> tuple[int, bool]:
    if 'A' <= char <= 'Z':
        return ord(char.lower()), True
    if char in SHIFTED:
        return ord(SHIFTED[char]), True
    value = ord(char)
    return (value if value <= 0xff else 0x1000000 | value), False


def modifier_symbols(chord: Chord) -> list[int]:
    return sorted({MOD_SYMBOLS[modifier] for modifier in chord.modifiers})


def _xbindings():
    try:
        from Xlib import X, Xatom, display
        from Xlib.ext import xtest
    except ImportError as error:
        raise DriverError(f'the atspi driver needs python-xlib: {error}') from error
    return X, Xatom, display, xtest


class AtspiDriver(Driver):
    name = 'atspi'

    def __init__(self, pid: int, ui_ids: UiIds | None = None, timeout: float = DEFAULT_TIMEOUT,
                 *, instance: Any = None) -> None:
        super().__init__(pid, ui_ids, timeout)
        if instance is None or instance.ui is None or instance.pid != pid:
            raise DriverError('atspi requires the matching instance and its private Xvfb context')
        self.instance = instance
        instance.ui.check(instance.env, instance.root)
        self.atspi = instance.ui.run.init_atspi()
        from gi.repository import GLib
        self._native_error = GLib.Error
        self.X, self.Xatom, display, self.xtest = _xbindings()
        self.connection = display.Display(instance.ui.display)
        self._mapped_symbols: dict[int, int] = {}
        self._menu_path = None
        self._menu_depth = 0
        self._menu_scope = None
        try:
            if not self.connection.has_extension('XTEST'):
                raise DriverError('private Xvfb does not provide XTEST')
            seen = []
            def application():
                desktop = self.atspi.get_desktop(0)
                desktop.clear_cache()
                seen.clear()
                for i in range(desktop.get_child_count()):
                    app = desktop.get_child_at_index(i)
                    if app is None:
                        raise ReadFailed('atspi desktop changed during application discovery')
                    app_pid = app.get_process_id()
                    seen.append((app_pid, app.get_name()))
                    if app_pid == pid:
                        return app
                return None
            try:
                self.app = wait_until(lambda: self._read(application), timeout)
            except WaitTimeout as error:
                raise DriverError(f'atspi: pid {pid} did not register; applications seen: {seen}: {error}') from None
        except BaseException:
            self.connection.close()
            raise

    def __del__(self):
        connection = getattr(self, 'connection', None)
        if connection is not None:
            try:
                connection.close()
            except Exception:
                pass

    def _read(self, fn):
        try:
            return fn()
        except self._native_error as error:
            raise ReadFailed(f'atspi pid {self.pid}: {error}') from error

    def walk(self):
        self.app.clear_cache()
        queue = deque([(self.app, 0)])
        while queue:
            handle, depth = queue.popleft()
            if handle is None:
                raise ReadFailed(f'atspi pid {self.pid}: tree changed during traversal')
            yield handle
            if depth < 64:
                queue.extend((handle.get_child_at_index(i), depth + 1) for i in range(handle.get_child_count()))

    def snapshot(self, handle) -> Element:
        state = handle.get_state_set()
        role = ROLE_NAMES.get(handle.get_role_name(), 'other')
        interfaces = handle.get_interfaces()
        value = None
        if role in ('checkbox', 'radio'):
            value = int(state.contains(self.atspi.StateType.CHECKED) or state.contains(self.atspi.StateType.SELECTED))
        elif 'Text' in interfaces:
            value = handle.get_text(0, -1)
        elif 'Value' in interfaces:
            value = handle.get_current_value()
        if role == 'text' and 'EditableText' in interfaces:
            role = 'text-field'
        if role == 'text-field' and not state.contains(self.atspi.StateType.EDITABLE):
            # A read-only field is selectable text (upstream's static text with `.textSelection`).
            role = 'text'
        elif role == 'text' and value is None:
            # A static label without text runs exposes its text only as its name (AccessKit's label from value).
            value = handle.get_name() or None
        return Element(handle.get_accessible_id() or None, role, handle.get_name() or None, value,
                       state.contains(self.atspi.StateType.ENABLED), handle)

    def _all_elements(self):
        return self._read(lambda: [self.snapshot(handle) for handle in self.walk()])

    def _elements(self, ui_id):
        return self._read(lambda: [self.snapshot(h) for h in self.walk() if h.get_accessible_id() == ui_id])

    def _identified(self):
        return [el for el in self._all_elements() if el.ui_id]

    def _menu_item(self, name):
        return next((el for el in self._all_elements() if el.role == 'menu-item' and el.name == name), None)

    def _scoped_menu_item(self, scope, name):
        def find():
            self.app.clear_cache()
            queue = deque(scope.get_child_at_index(i) for i in range(scope.get_child_count()))
            while queue:
                handle = queue.popleft()
                if handle is None:
                    raise ReadFailed('menu changed during traversal')
                if ROLE_NAMES.get(handle.get_role_name()) == 'menu-item':
                    if handle.get_name() == name:
                        return self.snapshot(handle)
                    # A nested submenu belongs to this item, not the current menu.
                    continue
                queue.extend(handle.get_child_at_index(i) for i in range(handle.get_child_count()))
            return None
        return self._read(find)

    def _menus(self, scope):
        def collect():
            self.app.clear_cache()
            menus, queue = [], deque([scope])
            while queue:
                handle = queue.popleft()
                if handle is None:
                    raise ReadFailed('menu changed during traversal')
                if handle.get_role_name() == 'menu':
                    menus.append(handle)
                queue.extend(handle.get_child_at_index(i) for i in range(handle.get_child_count()))
            return menus
        return self._read(collect)

    @staticmethod
    def _top_level(handle):
        # In-window overlays (GPUI's deferred popups, modals) publish their own dialog nodes;
        # only the outermost window-role ancestor, the application's child, is an X11 window.
        frame = None
        while handle is not None and handle.get_role_name() != 'application':
            if ROLE_NAMES.get(handle.get_role_name()) == 'window':
                frame = handle
            handle = handle.get_parent()
        if frame is None:
            raise ReadFailed('element has no containing accessibility window')
        return frame

    def _containing_frame(self, element):
        return self._read(lambda: self._top_level(element.handle))

    def _window_title(self, window):
        title = window.get_full_text_property(self.connection.intern_atom('_NET_WM_NAME'),
                                              self.connection.intern_atom('UTF8_STRING'))
        return title if title is not None else window.get_wm_name()

    def _menu_button(self, window):
        # Accessibility traversal order is independent of X focus/stacking, and a frame's
        # ACTIVE state and extents lag the X server. Same-titled frames are told apart only by
        # moving the window (see _probe_frame), so input and deferred popup discovery agree.
        root = self.connection.screen().root
        origin, geometry = root.translate_coords(window, 0, 0), window.get_geometry()
        expected = (origin.x, origin.y, geometry.width, geometry.height)
        title = self._window_title(window)
        matches = [(button, frame) for button in self._elements('app-menu-button')
                   for frame in [self._containing_frame(button)] if self._read(frame.get_name) == title]
        if len(matches) > 1:
            found = self._probe_frame(window, [frame for _, frame in matches])
            matches = [(button, frame) for button, frame in matches if found is not None and frame == found]
        else:
            matches = [(button, frame) for button, frame in matches if self._extents(frame) == expected]
        return matches[0] if len(matches) == 1 else None

    def _extents(self, handle):
        bounds = self._read(lambda: handle.get_extents(self.atspi.CoordType.SCREEN))
        return bounds.x, bounds.y, bounds.width, bounds.height

    def _raise(self, window):
        window.configure(stack_mode=self.X.Above)
        window.set_input_focus(self.X.RevertToParent, self.X.CurrentTime)
        self.connection.sync()

    def _activate_menu_window(self):
        self._raise(self._menu_xwindow)

    def _menu_command(self, path):
        # GPUI defers popups: each new Menu is a sibling under the window, not
        # necessarily a descendant of its trigger. Observe the new popup after each
        # input exactly once, then scope name lookup to that popup's immediate items.
        if self._menu_path != path:
            window = self._window()
            self._menu_xwindow = window
            self._activate_menu_window()
            match = self._menu_button(window)
            if match is None:
                return None
            button, self._menu_window = match
            self._menu_xwindow = window
            self._menu_pending = self._menus(self._menu_window)
            self.mouse_click(button)
            self._menu_path, self._menu_depth = path, 0
        while True:
            if self._menu_pending is not None:
                opened = [menu for menu in self._menus(self._menu_window)
                          if menu not in self._menu_pending]
                if len(opened) != 1:
                    return None
                self._menu_scope, self._menu_pending = opened[0], None
            if self._menu_depth == len(path) - 1:
                item = self._scoped_menu_item(self._menu_scope, path[-1])
                return None if item is None else self._reveal(self._menu_scope, item)
            title = path[self._menu_depth]
            if title in ('ngterm', 'agterm'):
                self._menu_depth += 1
                continue
            item = self._scoped_menu_item(self._menu_scope, title)
            if item is None:
                return None
            before = self._menus(self._menu_window)
            self._activate_menu_window()
            self._hover(item)
            self._menu_pending = before
            self._menu_depth += 1

    def _reveal(self, scope, item, attempts=40):
        # A menu taller than its room scrolls, and a press lands at the item's centre: wheel the
        # menu under the pointer until the item lies inside it, as a person scrolls to it.
        def placement():
            top, bottom = self._extents(scope), self._extents(item.handle)
            if bottom[1] < top[1]:
                return -1
            if bottom[1] + bottom[3] > top[1] + top[3]:
                return 1
            return 0
        for _ in range(attempts):
            step = placement()
            if step == 0:
                return item
            x, y, w, h = self._extents(scope)
            self._pointer_move(x + w / 2, y + h / 2)
            self._wheel(0, step)
            # the scroll lands on the next frame: wait for the item to move before judging again
            before = self._extents(item.handle)
            try:
                wait_until(lambda: self._extents(item.handle) != before, 1)
            except WaitTimeout:
                pass
        raise DriverError(f'atspi: menu item {item.name!r} stays outside its menu after {attempts} wheel ticks')

    def _tab(self, name):
        return next((el for el in self._all_elements()
                     if el.name == name and (el.ui_id == f'settings-tab-{name}' or el.role == 'radio')), None)

    def _press_element(self, element):
        self._menu_path = None
        def perform():
            handle = element.handle
            for i in range(handle.get_n_actions()):
                if handle.get_action_name(i).lower() in ('click', 'press', 'activate', 'toggle'):
                    if handle.do_action(i):
                        return
                    raise DriverError(f'atspi action refused on {element}')
            raise DriverError(f'atspi has no press action on {element}')
        self._read(perform)

    def _text(self, element):
        return element.value if isinstance(element.value, str) else element.name or ''

    def _set_text(self, element, text):
        handle = element.handle
        if 'EditableText' in self._read(handle.get_interfaces):
            if not self._read(lambda: handle.set_text_contents(text)):
                raise DriverError(f'atspi cannot set text on {element}')
            return
        # AccessKit publishes no EditableText: replace the field's text as a person does, through the keyboard on
        # the focused field (select all, then type or delete).
        if element.role != 'text-field':
            raise DriverError(f'atspi cannot set text on {element}')
        self._focus(element)
        self._post_chord(Chord('a', frozenset({'ctrl'})))
        if text:
            self._type_text(text)
        else:
            self._post_chord(Chord('delete', frozenset()))

    def _window(self, element=None):
        self.instance.ui.check(self.instance.env, self.instance.root)
        atom = self.connection.intern_atom('_NET_WM_PID')
        root = self.connection.screen().root
        def find():
            windows = []
            queue = deque([root])
            while queue:
                window = queue.popleft()
                prop = window.get_full_property(atom, self.Xatom.CARDINAL)
                if prop is not None and prop.value[0] == self.pid and window.get_attributes().map_state == self.X.IsViewable:
                    windows.append(window)
                queue.extend(window.query_tree().children)
            if element is not None:
                handle = self._containing_frame(element)
                bounds = self._extents(handle)
                matches = []
                for window in windows:
                    origin = root.translate_coords(window, 0, 0)
                    geometry = window.get_geometry()
                    if (origin.x, origin.y, geometry.width, geometry.height) == bounds:
                        matches.append(window)
                if len(matches) > 1:
                    title = self._read(handle.get_name)
                    matches = [window for window in matches if self._window_title(window) == title]
                # Never pair the frame's ACTIVE state with an X focus read: GPUI updates ACTIVE only
                # when it processes FocusIn/FocusOut, so neither identifies a window. Twins are probed.
                if len(matches) > 1:
                    return self._identify(handle, matches)
                if len(matches) == 1:
                    return matches[0]
                # Never silently focus another window when an element cannot be located.
                return None
            focused = self.connection.get_input_focus().focus
            while hasattr(focused, 'query_tree'):
                if focused in windows:
                    return focused
                if focused == root:
                    break
                focused = focused.query_tree().parent
            return windows[0] if windows else None
        return self._wait(find, 'mapped private X11 window', None)

    def _identify(self, frame, windows):
        # Overlapping windows can share bounds and title (unnamed windows of one session), and
        # the accessibility frame carries no X11 window id. Freeze the twin frames before moving
        # anything (a moved twin no longer matches), then probe one candidate window at a time.
        extents, title = self._extents(frame), self._read(frame.get_name)
        def twins():
            self.app.clear_cache()
            found = []
            for i in range(self.app.get_child_count()):
                handle = self.app.get_child_at_index(i)
                if handle is None:
                    raise ReadFailed('atspi application changed during window identification')
                if (ROLE_NAMES.get(handle.get_role_name()) == 'window' and handle.get_name() == title
                        and self._extents(handle) == extents):
                    found.append(handle)
            return found
        frozen = self._read(twins)
        if frame not in frozen:
            frozen.append(frame)
        for window in windows:
            found = self._probe_frame(window, frozen)
            if found is None:
                return None
            if found == frame:
                return window
        return None

    def _probe_frame(self, window, frames):
        """The one frame of `frames` that publishes `window`'s position, or None.

        GPUI publishes each frame's extents from its own X11 window's live root origin, so a
        position no window held before can only be reported by the moved window's frame; no
        queued focus or geometry event of another window can produce it. Moves keep the size and
        never touch focus. The window goes back in every case; an unconfirmed restore raises a
        non-retryable ProbeRestoreFailed, so no wait retries or input proceeds on moved geometry.
        """
        root = self.connection.screen().root
        geometry, origin = window.get_geometry(), root.translate_coords(window, 0, 0)
        home, size = (origin.x, origin.y), (geometry.width, geometry.height)
        # configure takes parent-relative coordinates; probe positions are root coordinates.
        parent = (origin.x - geometry.x, origin.y - geometry.y)
        probe = self._probe_origin(self._occupied(root), parent)
        if probe is None:
            return None
        found = None
        try:
            window.configure(x=probe[0] - parent[0], y=probe[1] - parent[1])
            self.connection.sync()
            def landed():
                return [frame for frame in frames if self._extents(frame) == (*probe, *size)]
            here = self._wait(landed, 'accessibility frame at the probe position', None)
            found = here[0] if len(here) == 1 else None
        finally:
            self._restore(window, (geometry.x, geometry.y), home, size, frames, probe, found)
        return found

    def _restore(self, window, position, home, size, frames, probe, found):
        root = self.connection.screen().root
        try:
            window.configure(x=position[0], y=position[1])
            self.connection.sync()
            def restored():
                origin, geometry = root.translate_coords(window, 0, 0), window.get_geometry()
                if (origin.x, origin.y, geometry.width, geometry.height) != (*home, *size):
                    return False
                if found is not None:
                    return self._extents(found) == (*home, *size)
                return all(self._extents(frame)[:2] != probe for frame in frames)
            self._wait(restored, 'probe window restored to its position', None)
        except Exception as error:
            raise ProbeRestoreFailed(f'atspi: window {window} not confirmed back at {home}: {error}') from error

    def _occupied(self, root):
        taken = set()
        for window in root.query_tree().children:
            origin = root.translate_coords(window, 0, 0)
            taken.add((origin.x, origin.y))
        def frames():
            self.app.clear_cache()
            return [self.app.get_child_at_index(i) for i in range(self.app.get_child_count())]
        for frame in self._read(frames):
            if frame is None:
                raise ReadFailed('atspi application changed during window identification')
            taken.add(self._extents(frame)[:2])
        return taken

    def _probe_origin(self, taken, parent):
        # Monotonic and never reused by this driver: a late publication of an earlier probe
        # can never satisfy a later one. Root and parent-relative coordinates stay in INT16.
        while (step := getattr(self, '_probe_next', 1)) <= INT16_MAX:
            self._probe_next = step + 1
            point = (step, step)
            relative = (point[0] - parent[0], point[1] - parent[1])
            if point not in taken and all(-INT16_MAX - 1 <= value <= INT16_MAX for value in relative):
                return point
        return None

    def _activate(self, element=None):
        self._window(element).set_input_focus(self.X.RevertToParent, self.X.CurrentTime)
        self.connection.sync()

    def _focus(self, element):
        self._activate(element)
        if not self._read(element.handle.grab_focus):
            raise DriverError(f'atspi cannot focus {element}')

    def _hover(self, element):
        self.move(element)

    def _bounds(self, element):
        return self._extents(element.handle)

    def _pointer_target(self, element):
        # XTest input hits the topmost window at the point, and another window of this app
        # may cover the target's. Raise and focus the target's own window; never guess.
        self._raise(self._window(element))

    def _pointer_move(self, x, y):
        self.instance.ui.check(self.instance.env, self.instance.root)
        self.xtest.fake_input(self.connection, self.X.MotionNotify, x=round(x), y=round(y))
        self.connection.sync()

    def _pointer_button(self, button, down):
        self.instance.ui.check(self.instance.env, self.instance.root)
        self.xtest.fake_input(self.connection, self.X.ButtonPress if down else self.X.ButtonRelease,
                              1 if button == 'left' else 3)
        self.connection.sync()

    def _wheel(self, dx, dy):
        for distance, negative, positive in ((dy, 4, 5), (dx, 6, 7)):
            for _ in range(abs(distance)):
                number = positive if distance > 0 else negative
                self.xtest.fake_input(self.connection, self.X.ButtonPress, number)
                self.xtest.fake_input(self.connection, self.X.ButtonRelease, number)
        self.connection.sync()

    def _keycode(self, symbol):
        # ASCII US keys and modifiers are supplied by Xvfb. Unicode symbols may require an
        # unused keycode; mappings persist only on this disposable private X server.
        code = next((code for code, level in self.connection.keysym_to_keycodes(symbol) if level == 0), None)
        if code is not None:
            return code
        if symbol in self._mapped_symbols:
            return self._mapped_symbols[symbol]
        first = self.connection.display.info.min_keycode
        mapping = self.connection.get_keyboard_mapping(first, self.connection.display.info.max_keycode - first + 1)
        used = set(self._mapped_symbols.values())
        for index, row in enumerate(mapping):
            code = first + index
            if not any(row) and code not in used:
                self.connection.change_keyboard_mapping(code, [(symbol,) * len(row)])
                self.connection.sync()
                self._mapped_symbols[symbol] = code
                return code
        raise DriverError('private Xvfb has no unused keycode for Unicode input')

    def _keys(self, symbol, modifiers):
        codes = [self._keycode(mod) for mod in modifiers] + [self._keycode(symbol)]
        held = []
        try:
            for code in codes:
                self.xtest.fake_input(self.connection, self.X.KeyPress, code)
                held.append(code)
        finally:
            for code in reversed(held):
                self.xtest.fake_input(self.connection, self.X.KeyRelease, code)
            self.connection.sync()

    def _post_chord(self, chord: Chord):
        self._activate()
        self._keys(key_symbol(chord.key), modifier_symbols(chord))

    def _type_text(self, text):
        self._activate()
        # Allocate before posting anything so the app observes the complete keyboard map.
        symbols = [text_symbol(char) for char in text]
        for symbol, _ in symbols:
            self._keycode(symbol)
        for symbol, shift in symbols:
            self._keys(symbol, [MOD_SYMBOLS['shift']] if shift else [])
