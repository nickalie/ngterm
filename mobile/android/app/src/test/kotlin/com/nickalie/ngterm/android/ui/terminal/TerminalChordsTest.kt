package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyEvent
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import kotlin.coroutines.EmptyCoroutineContext

/** Ownership of a hardware key's whole stroke when navigation becomes available while it is held. */
@RunWith(RobolectricTestRunner::class)
class TerminalChordsTest {
    private val pane = SelectingPane()
    private val state = TerminalState(pane, EmptyCoroutineContext)
    private val keyboard = TerminalKeyboard().also { it.target = state }
    private var moves = 0
    private val shortcuts =
        TerminalShortcuts(
            nextPane = { moves++ },
            previousPane = { moves++ },
            nextSession = { moves++ },
            previousSession = { moves++ },
        )

    @Test
    fun aMoveInstalledMidHoldLeavesThePressRepeatsAndReleaseToTheProgram() {
        val arrows =
            mapOf(
                KeyEvent.KEYCODE_DPAD_RIGHT to "arrow_right",
                KeyEvent.KEYCODE_DPAD_LEFT to "arrow_left",
                KeyEvent.KEYCODE_DPAD_DOWN to "arrow_down",
                KeyEvent.KEYCODE_DPAD_UP to "arrow_up",
            )
        for ((code, name) in arrows) {
            keyboard.shortcuts = TerminalShortcuts()
            pane.inputs.clear()
            keyboard.key(event(code))
            keyboard.key(event(code, repeat = 1))
            // A remote layout adds a pane, or another session attaches, before the next autorepeat.
            keyboard.shortcuts = shortcuts
            keyboard.key(event(code, repeat = 2))
            keyboard.key(event(code, action = KeyEvent.ACTION_UP, meta = 0))
            val press = KeyMapping.named(name, KeyMapping.CTRL or KeyMapping.SHIFT)
            assertEquals(
                name,
                listOf(
                    press,
                    press.copy(action = PaneKeyAction.REPEAT),
                    press.copy(action = PaneKeyAction.REPEAT),
                    KeyMapping.named(name, 0).copy(action = PaneKeyAction.RELEASE),
                ),
                pane.inputs.filterIsInstance<FakeInput.Key>().map { it.key },
            )
            assertEquals("no move before a fresh press: $name", 0, moves)
        }
        pane.inputs.clear()
        keyboard.key(event(KeyEvent.KEYCODE_DPAD_RIGHT))
        keyboard.key(event(KeyEvent.KEYCODE_DPAD_RIGHT, repeat = 1))
        keyboard.key(event(KeyEvent.KEYCODE_DPAD_RIGHT, action = KeyEvent.ACTION_UP, meta = 0))
        assertEquals("a fresh press moves once", 1, moves)
        assertTrue("the claimed stroke never reaches the program", pane.inputs.isEmpty())
    }

    @Test
    fun aMoveInstalledMidHoldLeavesTheWholeStrokeToTheChatField() {
        val code = KeyEvent.KEYCODE_DPAD_DOWN
        assertFalse(keyboard.move(event(code)))
        keyboard.shortcuts = shortcuts
        assertFalse(keyboard.move(event(code, repeat = 1)))
        assertFalse(keyboard.move(event(code, action = KeyEvent.ACTION_UP)))
        assertEquals(0, moves)

        assertTrue(keyboard.move(event(code)))
        assertTrue(keyboard.move(event(code, repeat = 1)))
        // Once claimed, the stroke stays the app's even if the destination disappears during the hold.
        keyboard.shortcuts = TerminalShortcuts()
        assertTrue(keyboard.move(event(code, repeat = 2)))
        assertTrue(keyboard.move(event(code, action = KeyEvent.ACTION_UP, meta = 0)))
        assertEquals(1, moves)
    }

    @Test
    fun aClaimedTextSizeChordStillRepeats() {
        val initial = state.textSize.value
        val code = KeyEvent.KEYCODE_EQUALS
        keyboard.key(event(code, meta = KeyEvent.META_CTRL_ON))
        keyboard.key(event(code, repeat = 1, meta = KeyEvent.META_CTRL_ON))
        keyboard.key(event(code, action = KeyEvent.ACTION_UP, meta = 0))
        assertEquals((initial + 2).sp, state.textSize)
        assertTrue(pane.inputs.isEmpty())
    }

    private fun event(
        code: Int,
        action: Int = KeyEvent.ACTION_DOWN,
        repeat: Int = 0,
        meta: Int = KeyEvent.META_CTRL_ON or KeyEvent.META_SHIFT_ON,
    ) = KeyEvent(0L, 0L, action, code, repeat, meta)
}
