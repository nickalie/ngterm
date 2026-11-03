package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyEvent
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/**
 * Android's key events turned into the pane's keys (`PaneKey`, ngterm-term's `KeyInput` over the binding): the key's
 * position by Ghostty's name, the modifier bits, the text before Ctrl and Alt, the unshifted character and the action;
 * a character typed with sticky modifiers; and the app's hardware chords.
 */
@RunWith(RobolectricTestRunner::class)
class KeyMappingTest {
    private fun event(
        keyCode: Int,
        meta: Int = 0,
        action: Int = KeyEvent.ACTION_DOWN,
        repeat: Int = 0,
    ) = KeyEvent(0L, 0L, action, keyCode, repeat, meta)

    private fun key(
        name: String,
        text: String = "",
        mods: Int = 0,
        consumed: Int = 0,
        unshifted: String? = null,
    ) = PaneKey(PaneKeyAction.PRESS, name, mods.toUByte(), consumed.toUByte(), false, text, unshifted)

    @Test
    fun keysAreNamedByTheirPositionAsGhosttyNamesThem() {
        val names =
            mapOf(
                KeyEvent.KEYCODE_A to "key_a",
                KeyEvent.KEYCODE_Z to "key_z",
                KeyEvent.KEYCODE_0 to "digit_0",
                KeyEvent.KEYCODE_9 to "digit_9",
                KeyEvent.KEYCODE_ENTER to "enter",
                KeyEvent.KEYCODE_DEL to "backspace",
                KeyEvent.KEYCODE_FORWARD_DEL to "delete",
                KeyEvent.KEYCODE_ESCAPE to "escape",
                KeyEvent.KEYCODE_TAB to "tab",
                KeyEvent.KEYCODE_DPAD_UP to "arrow_up",
                KeyEvent.KEYCODE_DPAD_LEFT to "arrow_left",
                KeyEvent.KEYCODE_MOVE_HOME to "home",
                KeyEvent.KEYCODE_MOVE_END to "end",
                KeyEvent.KEYCODE_PAGE_UP to "page_up",
                KeyEvent.KEYCODE_PAGE_DOWN to "page_down",
                KeyEvent.KEYCODE_F1 to "f1",
                KeyEvent.KEYCODE_F12 to "f12",
                KeyEvent.KEYCODE_NUMPAD_5 to "numpad_5",
                KeyEvent.KEYCODE_NUMPAD_ENTER to "numpad_enter",
                KeyEvent.KEYCODE_GRAVE to "backquote",
                KeyEvent.KEYCODE_APOSTROPHE to "quote",
                KeyEvent.KEYCODE_CTRL_LEFT to "control_left",
                KeyEvent.KEYCODE_META_RIGHT to "meta_right",
                KeyEvent.KEYCODE_VOLUME_UP to "unidentified",
            )
        for ((code, name) in names) {
            assertEquals(KeyEvent.keyCodeToString(code), name, KeyMapping.physicalKey(code))
        }
    }

    @Test
    fun theModifiersHeldAreThePanesBits() {
        assertEquals(0, KeyMapping.mods(0))
        assertEquals(KeyMapping.SHIFT, KeyMapping.mods(KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON))
        assertEquals(KeyMapping.CTRL, KeyMapping.mods(KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_RIGHT_ON))
        assertEquals(KeyMapping.ALT, KeyMapping.mods(KeyEvent.META_ALT_ON))
        assertEquals(KeyMapping.SUPER, KeyMapping.mods(KeyEvent.META_META_ON))
        assertEquals(KeyMapping.CAPS_LOCK, KeyMapping.mods(KeyEvent.META_CAPS_LOCK_ON))
        assertEquals(KeyMapping.NUM_LOCK, KeyMapping.mods(KeyEvent.META_NUM_LOCK_ON))
        assertEquals(
            KeyMapping.CTRL or KeyMapping.SHIFT,
            KeyMapping.mods(KeyEvent.META_CTRL_ON or KeyEvent.META_SHIFT_ON),
        )
    }

    @Test
    fun aLetterTypesItselfAndShiftIsUsedUpByTheCapital() {
        assertEquals(key("key_a", "a", unshifted = "a"), KeyMapping.fromEvent(event(KeyEvent.KEYCODE_A)))
        val shift = KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON
        assertEquals(
            key("key_a", "A", mods = KeyMapping.SHIFT, consumed = KeyMapping.SHIFT, unshifted = "a"),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_A, shift)),
        )
        assertEquals(
            key("digit_1", "!", mods = KeyMapping.SHIFT, consumed = KeyMapping.SHIFT, unshifted = "1"),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_1, shift)),
        )
    }

    @Test
    fun ctrlAndAltLeaveTheTextAsTheLayoutTypesIt() {
        val ctrl = KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON
        assertEquals(
            key("key_c", "c", mods = KeyMapping.CTRL, unshifted = "c"),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_C, ctrl)),
        )
        val alt = KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON
        assertEquals(
            key("key_x", "x", mods = KeyMapping.ALT, unshifted = "x"),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_X, alt)),
        )
    }

    @Test
    fun functionKeysTypeNoText() {
        assertEquals(key("enter"), KeyMapping.fromEvent(event(KeyEvent.KEYCODE_ENTER)))
        assertEquals(key("tab"), KeyMapping.fromEvent(event(KeyEvent.KEYCODE_TAB)))
        assertEquals(key("backspace"), KeyMapping.fromEvent(event(KeyEvent.KEYCODE_DEL)))
        assertEquals(key("arrow_up"), KeyMapping.fromEvent(event(KeyEvent.KEYCODE_DPAD_UP)))
        assertEquals(
            key("arrow_left", mods = KeyMapping.CTRL or KeyMapping.SHIFT),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.META_CTRL_ON or KeyEvent.META_SHIFT_ON)),
        )
    }

    @Test
    fun aHeldKeyRepeatsAndALiftedOneIsReleased() {
        assertEquals(
            key("key_j", "j", unshifted = "j").copy(action = PaneKeyAction.REPEAT),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_J, repeat = 3)),
        )
        assertEquals(
            key("key_j", "j", unshifted = "j").copy(action = PaneKeyAction.RELEASE),
            KeyMapping.fromEvent(event(KeyEvent.KEYCODE_J, action = KeyEvent.ACTION_UP)),
        )
        assertNull(
            "a run of characters is the input method's text, not a key",
            KeyMapping.fromEvent(KeyEvent(0L, "ok", 0, 0)),
        )
    }

    @Test
    fun aDeadKeyComposes() {
        val acute = KeyMapping.key(KeyEvent.KEYCODE_E, PaneKeyAction.PRESS, 0, ACCENT or '´'.code, 'e'.code)
        assertEquals(key("key_e", unshifted = "e").copy(composing = true), acute)
    }

    @Test
    fun aCharacterWithStickyModifiersIsTheKeyThatTypesItOnAUsLayout() {
        assertEquals(
            key("key_c", "c", mods = KeyMapping.CTRL, unshifted = "c"),
            KeyMapping.forText("c", KeyMapping.CTRL),
        )
        assertEquals(
            key("key_d", "D", mods = KeyMapping.ALT or KeyMapping.SHIFT, consumed = KeyMapping.SHIFT, unshifted = "d"),
            KeyMapping.forText("D", KeyMapping.ALT),
        )
        assertEquals(
            key("backslash", "|", mods = KeyMapping.SHIFT, consumed = KeyMapping.SHIFT, unshifted = "\\"),
            KeyMapping.forText("|", 0),
        )
        assertEquals(key("slash", "/", unshifted = "/"), KeyMapping.forText("/", 0))
        assertEquals(
            key("space", " ", mods = KeyMapping.CTRL, unshifted = " "),
            KeyMapping.forText(" ", KeyMapping.CTRL),
        )
        assertEquals(key("unidentified", "é", mods = KeyMapping.ALT), KeyMapping.forText("é", KeyMapping.ALT))
    }

    @Test
    fun theAppsChordsAreDesktopsLinuxOnes() {
        val ctrl = KeyMapping.CTRL
        val ctrlShift = KeyMapping.CTRL or KeyMapping.SHIFT
        assertEquals(TerminalShortcut.Bigger, KeyMapping.shortcut(KeyEvent.KEYCODE_EQUALS, ctrl))
        assertEquals(TerminalShortcut.Bigger, KeyMapping.shortcut(KeyEvent.KEYCODE_EQUALS, ctrlShift))
        assertEquals(TerminalShortcut.Bigger, KeyMapping.shortcut(KeyEvent.KEYCODE_NUMPAD_ADD, ctrl))
        assertEquals(TerminalShortcut.Smaller, KeyMapping.shortcut(KeyEvent.KEYCODE_MINUS, ctrl))
        assertEquals(TerminalShortcut.ResetSize, KeyMapping.shortcut(KeyEvent.KEYCODE_0, ctrl))
        assertEquals(TerminalShortcut.NextPane, KeyMapping.shortcut(KeyEvent.KEYCODE_DPAD_RIGHT, ctrlShift))
        assertEquals(TerminalShortcut.PreviousPane, KeyMapping.shortcut(KeyEvent.KEYCODE_DPAD_LEFT, ctrlShift))
        assertEquals(TerminalShortcut.NextSession, KeyMapping.shortcut(KeyEvent.KEYCODE_DPAD_DOWN, ctrlShift))
        assertEquals(TerminalShortcut.PreviousSession, KeyMapping.shortcut(KeyEvent.KEYCODE_DPAD_UP, ctrlShift))
        // the locks don't count; anything else held, or nothing, is the program's
        assertEquals(TerminalShortcut.Smaller, KeyMapping.shortcut(KeyEvent.KEYCODE_MINUS, ctrl or KeyMapping.NUM_LOCK))
        assertNull(KeyMapping.shortcut(KeyEvent.KEYCODE_MINUS, 0))
        assertNull(KeyMapping.shortcut(KeyEvent.KEYCODE_DPAD_RIGHT, ctrl))
        assertNull(KeyMapping.shortcut(KeyEvent.KEYCODE_EQUALS, ctrl or KeyMapping.ALT))
        assertNull(KeyMapping.shortcut(KeyEvent.KEYCODE_C, ctrl))
    }

    private companion object {
        const val ACCENT = 0x80000000.toInt()
    }
}
