package com.nickalie.ngterm.android.ui.terminal

import android.content.res.Configuration
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.test.withKeysDown
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * Typing into the terminal against a fake pane that records its input: through the soft keyboard (the input
 * method's connection: commits, compositions, Enter and backspace), through the extra-keys row (its keys, the sticky
 * Ctrl and Alt, a held arrow), and through a hardware keyboard (keys, and the app's chords).
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalKeyboardTest {
    @get:Rule
    val compose = createComposeRule()

    private val pane = SelectingPane()
    private lateinit var state: TerminalState
    private lateinit var view: View
    private val moves = mutableListOf<String>()
    private val keyboard =
        TerminalKeyboard(
            TerminalShortcuts(
                nextPane = { moves += "next pane" },
                previousPane = { moves += "previous pane" },
                nextSession = { moves += "next session" },
            ),
        )

    private val terminal get() = compose.onNodeWithContentDescription("Terminal")

    private fun show() {
        compose.setContent {
            NgtermTheme(darkTheme = true) {
                view = LocalView.current
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                this.state = state
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    onDispose { pane.onWake = null }
                }
                Column(Modifier.fillMaxSize()) {
                    TerminalView(state, Modifier.weight(1f).fillMaxWidth(), keyboard = keyboard)
                    ExtraKeysRow(keyboard)
                }
            }
        }
        compose.waitForIdle()
    }

    /** Taps the terminal, which takes the focus and opens the soft keyboard. */
    private fun focus() {
        terminal.performTouchInput { click(center) }
        compose.waitForIdle()
        pane.inputs.clear()
    }

    private fun connect(info: EditorInfo = EditorInfo()): InputConnection =
        requireNotNull(view.onCreateInputConnection(info)) { "no input connection while the terminal has focus" }

    private fun keys(): List<PaneKey> = pane.inputs.filterIsInstance<FakeInput.Key>().map { it.key }

    private fun presses(): List<PaneKey> = keys().filter { it.action != PaneKeyAction.RELEASE }

    private fun press(
        name: String,
        mods: Int = 0,
    ) = KeyMapping.named(name, mods)

    private fun key(name: String) = compose.onNodeWithContentDescription(name)

    @Test
    fun aTapFocusesTheTerminalAndOpensTheSoftKeyboard() {
        show()
        terminal.performTouchInput { click(center) }
        compose.waitForIdle()
        terminal.assertIsFocused()
        assertTrue(keyboard.target === state)
        val methods = RuntimeEnvironment.getApplication().getSystemService(InputMethodManager::class.java)
        assertTrue("the soft keyboard opens", shadowOf(methods).isSoftInputVisible)

        val info = EditorInfo()
        connect(info)
        assertEquals(InputType.TYPE_CLASS_TEXT, info.inputType and InputType.TYPE_MASK_CLASS)
        assertTrue("no suggestions", info.inputType and InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS != 0)
        assertEquals("no autocorrect", 0, info.inputType and InputType.TYPE_TEXT_FLAG_AUTO_CORRECT)
        assertEquals("no automatic capitals", 0, info.inputType and InputType.TYPE_TEXT_FLAG_CAP_SENTENCES)
        assertTrue(info.imeOptions and EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING != 0)
        assertTrue(info.imeOptions and EditorInfo.IME_FLAG_NO_FULLSCREEN != 0)
        assertTrue(info.imeOptions and EditorInfo.IME_FLAG_NO_ENTER_ACTION != 0)
    }

    @Test
    fun theSoftKeyboardTypesWhatItCommitsAndItsKeys() {
        show()
        focus()
        val connection = connect()
        connection.commitText("ls -la", 1)
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ENTER))
        connection.commitText("echo hi\n", 1)
        connection.deleteSurroundingText(2, 0)
        connection.performEditorAction(EditorInfo.IME_ACTION_DONE)
        assertEquals(
            listOf(
                FakeInput.Typed("ls -la"),
                FakeInput.Key(press("enter")),
                FakeInput.Key(press("enter", 0).copy(action = PaneKeyAction.RELEASE)),
                FakeInput.Typed("echo hi"),
                FakeInput.Key(press("enter")),
                FakeInput.Key(press("enter").copy(action = PaneKeyAction.RELEASE)),
            ),
            pane.inputs.take(6),
        )
        assertEquals(
            "two backspaces for nothing composed, then Enter for the action",
            listOf("backspace", "backspace", "enter"),
            presses().drop(2).map { it.key },
        )
    }

    // the program's input line as the pane's input builds it: typed text, and a backspace taking the last character
    private fun line(): String =
        pane.inputs.fold("") { line, input ->
            when {
                input is FakeInput.Typed -> {
                    line + input.text
                }

                input is FakeInput.Key && input.key.action == PaneKeyAction.PRESS && input.key.key == "backspace" -> {
                    line.dropLast(if (line.isEmpty()) 0 else line.length - line.offsetByCodePoints(line.length, -1))
                }

                else -> {
                    line
                }
            }
        }

    @Test
    fun eachLetterReachesTheProgramAsTheKeyboardComposesIt() {
        show()
        focus()
        val connection = connect()
        connection.commitText("/", 1)
        connection.setComposingText("c", 1)
        assertEquals("/c", line())
        connection.setComposingText("cl", 1)
        connection.setComposingText("cle", 1)
        assertEquals("/cle", line())
        connection.setComposingText("cl", 1)
        assertEquals("a backspace in the word is the program's", "/cl", line())
        connection.setComposingText("clear", 1)
        connection.commitText("clear", 1)
        connection.commitText(" ", 1)
        assertEquals("/clear ", line())
        connection.setComposingText("teh", 1)
        connection.commitText("the ", 1)
        assertEquals("a word the keyboard replaces is retyped in place", "/clear the ", line())
        connection.setComposingText("a😀", 1)
        connection.setComposingText("a", 1)
        assertEquals("/clear the a", line())
        connection.setComposingText("a\nb", 1)
        assertEquals("/clear the ab", line())
        assertEquals(press("enter"), presses().last())
        connection.setComposingText("c", 1)
        assertEquals("a line break ended the composition: the next one starts after it", "/clear the abc", line())
        compose.waitForIdle()
        assertEquals("nothing waits outside the program", "", state.composing)
    }

    @Test
    fun aCompositionReachesTheProgramAsItChanges() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("ni", 1)
        assertEquals("ni", line())
        connection.setComposingText("に", 1)
        assertEquals("a conversion replaces the reading on the program's line", "に", line())
        connection.commitText("日本", 1)
        assertEquals("日本", line())
        connection.setComposingText("語", 1)
        connection.finishComposingText()
        connection.commitText("!", 1)
        compose.waitForIdle()
        assertEquals("日本語!", line())
        assertEquals("", state.composing)
        assertEquals(
            listOf(
                FakeInput.Typed("ni"),
                FakeInput.Typed("に"),
                FakeInput.Typed("日本"),
                FakeInput.Typed("語"),
                FakeInput.Typed("!"),
            ),
            pane.inputs.filterIsInstance<FakeInput.Typed>(),
        )
    }

    @Test
    fun theExtraKeysSendTheirKeys() {
        show()
        focus()
        key("Escape").performClick()
        key("Tab").performClick()
        key("Page down").performClick()
        key("Vertical bar").performClick()
        compose.waitForIdle()
        assertEquals(
            listOf(press("escape"), press("tab"), press("page_down")),
            presses(),
        )
        assertEquals(FakeInput.Typed("|"), pane.inputs.last())
    }

    @Test
    fun stickyCtrlHoldsForTheNextKeyAndAltLocks() {
        show()
        focus()
        key("Control").performClick()
        compose.waitForIdle()
        assertEquals(Sticky.Once, keyboard.ctrl)
        connect().commitText("c", 1)
        compose.waitForIdle()
        assertEquals(listOf(KeyMapping.forText("c", KeyMapping.CTRL)), presses())
        assertEquals("a once-held Ctrl lets go after one key", Sticky.Off, keyboard.ctrl)

        pane.inputs.clear()
        key("Alt").performClick()
        key("Alt").performClick()
        compose.waitForIdle()
        assertEquals(Sticky.Locked, keyboard.alt)
        key("Up arrow").performClick()
        key("Slash").performClick()
        compose.waitForIdle()
        assertEquals(
            listOf(press("arrow_up", KeyMapping.ALT), KeyMapping.forText("/", KeyMapping.ALT)),
            presses(),
        )
        assertEquals("a locked Alt stays", Sticky.Locked, keyboard.alt)
        key("Alt").performClick()
        compose.waitForIdle()
        assertEquals(Sticky.Off, keyboard.alt)
    }

    @Test
    fun aHeldArrowRepeats() {
        show()
        focus()
        key("Left arrow").performTouchInput { longClick(center, durationMillis = HOLD_MS) }
        compose.waitForIdle()
        val lefts = presses().count { it.key == "arrow_left" }
        assertTrue("one press, then a repeat every 50 ms after 400 ms: $lefts", lefts >= MIN_REPEATS)
        pane.inputs.clear()
        key("Escape").performTouchInput { longClick(center, durationMillis = HOLD_MS) }
        compose.waitForIdle()
        assertEquals("Esc doesn't repeat", listOf(press("escape")), presses())
    }

    @Test
    fun aHardwareKeyboardTypesAndItsChordsRunInTheApp() {
        show()
        focus()
        terminal.performKeyInput { pressKey(Key.A) }
        compose.waitForIdle()
        assertEquals(listOf(KeyMapping.forText("a", 0)), presses())

        pane.inputs.clear()
        terminal.performKeyInput {
            withKeyDown(Key.CtrlLeft) {
                pressKey(Key.Equals)
                pressKey(Key.Equals)
            }
        }
        compose.waitForIdle()
        assertEquals(15.sp, state.textSize)
        terminal.performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.Minus) } }
        compose.waitForIdle()
        assertEquals(14.sp, state.textSize)
        terminal.performKeyInput { withKeyDown(Key.CtrlLeft) { pressKey(Key.Zero) } }
        compose.waitForIdle()
        assertEquals(TerminalTextSize, state.textSize)
        assertEquals(
            "the chords' keys never reach the program, only Ctrl itself",
            setOf("control_left"),
            keys().map { it.key }.toSet(),
        )

        pane.inputs.clear()
        terminal.performKeyInput {
            withKeysDown(listOf(Key.CtrlLeft, Key.ShiftLeft)) {
                pressKey(Key.DirectionRight)
                pressKey(Key.DirectionLeft)
                pressKey(Key.DirectionDown)
                pressKey(Key.DirectionUp)
            }
        }
        compose.waitForIdle()
        assertEquals(listOf("next pane", "previous pane", "next session"), moves)
        val up = presses().singleOrNull { it.key == "arrow_up" }
        assertNotNull("a chord the app has no action for is the program's", up)
        assertEquals(KeyMapping.CTRL or KeyMapping.SHIFT, up!!.mods.toInt())
    }

    @Test
    fun aMouseClickFocusesTheTerminalWithoutTheSoftKeyboard() {
        show()
        terminal.performMouseInput { click(center) }
        compose.waitForIdle()
        terminal.assertIsFocused()
        val methods = RuntimeEnvironment.getApplication().getSystemService(InputMethodManager::class.java)
        assertFalse("a mouse click doesn't open the soft keyboard", shadowOf(methods).isSoftInputVisible)
        terminal.performTouchInput { click(center) }
        compose.waitForIdle()
        assertTrue("a tap does", shadowOf(methods).isSoftInputVisible)
        connect()
    }

    @Test
    fun aKeyFromElsewhereCommitsTheCompositionFirst() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("ls", 1)
        key("Tab").performClick()
        compose.waitForIdle()
        assertEquals(listOf(FakeInput.Typed("ls"), FakeInput.Key(press("tab"))), pane.inputs.take(2))
        assertEquals("", state.composing)

        pane.inputs.clear()
        connection.setComposingText("cd", 1)
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
        connection.setComposingText("pwd", 1)
        connection.performEditorAction(EditorInfo.IME_ACTION_DONE)
        connection.setComposingText("x", 1)
        key("Vertical bar").performClick()
        compose.waitForIdle()
        assertEquals(
            listOf(
                FakeInput.Typed("cd"),
                FakeInput.Key(press("enter")),
                FakeInput.Typed("pwd"),
                FakeInput.Key(press("enter")),
                FakeInput.Typed("x"),
                FakeInput.Typed("|"),
            ),
            pane.inputs.filter { it !is FakeInput.Key || it.key.action != PaneKeyAction.RELEASE },
        )
    }

    @Test
    fun aStickyModifierArmedWhileComposingIsForTheKeyAfter() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("ls", 1)
        key("Control").performClick()
        compose.waitForIdle()
        assertEquals("the composition went out plain", listOf(FakeInput.Typed("ls")), pane.inputs)
        assertEquals(Sticky.Once, keyboard.ctrl)
        key("Left arrow").performClick()
        compose.waitForIdle()
        assertEquals(listOf(press("arrow_left", KeyMapping.CTRL)), presses())
        assertEquals(Sticky.Off, keyboard.ctrl)

        pane.inputs.clear()
        connection.setComposingText("かな", 1)
        key("Control").performClick()
        connection.sendKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ENTER))
        compose.waitForIdle()
        assertEquals(
            listOf(FakeInput.Typed("かな"), FakeInput.Key(press("enter", KeyMapping.CTRL))),
            pane.inputs.filter { it !is FakeInput.Key || it.key.action != PaneKeyAction.RELEASE },
        )

        pane.inputs.clear()
        key("Control").performClick()
        connection.setComposingText("c", 1)
        connection.finishComposingText()
        compose.waitForIdle()
        assertEquals(
            "a composition after the tap still gets the modifier",
            listOf(KeyMapping.forText("c", KeyMapping.CTRL)),
            presses(),
        )
    }

    @Test
    fun aDeletionPastTheCompositionIsTheProgramsBackspace() {
        show()
        focus()
        val connection = connect()
        connection.commitText("xy", 1)
        connection.setComposingText("ab", 1)
        connection.deleteSurroundingText(1, 0)
        assertEquals("xya", line())
        connection.deleteSurroundingText(2, 0)
        assertEquals("x", line())
        compose.waitForIdle()
        assertEquals("", state.composing)
        assertEquals(List(3) { press("backspace") }, presses())
        connection.setComposingText("q", 1)
        assertEquals("the composition starts afresh after the deletion", "xq", line())
    }

    @Test
    fun aDeletionInCodePointsTakesAWholeEmojiFromTheComposition() {
        show()
        focus()
        val connection = connect()
        connection.commitText("x", 1)
        connection.setComposingText("a\uD83D\uDE00", 1)
        connection.deleteSurroundingTextInCodePoints(1, 0)
        assertEquals("xa", line())
        connection.deleteSurroundingTextInCodePoints(2, 0)
        assertEquals("one past the composition is the program's", "", line())
        assertEquals(List(3) { press("backspace") }, presses())
    }

    @Test
    fun aConnectionCommitsToTheTerminalItWasOpenedFor() {
        val other = SelectingPane()
        val first = TerminalState(pane, EmptyCoroutineContext)
        val second = TerminalState(other, EmptyCoroutineContext)
        val host = View(RuntimeEnvironment.getApplication())
        keyboard.target = first
        val connection = TerminalInputConnection(host, keyboard, first).also(keyboard.connections::bind)
        connection.setComposingText("vi", 1)
        // the target moving on finishes the composition where it was typed, and ends the connection
        keyboard.target = second
        assertEquals(listOf(FakeInput.Typed("vi")), pane.inputs.toList())
        connection.commitText("x", 1)
        connection.closeConnection()
        assertEquals(listOf(FakeInput.Typed("vi")), pane.inputs.toList())
        assertTrue("nothing crosses to the terminal targeted since", other.inputs.isEmpty())
    }

    /** A hardware keyboard with [layout], typing into the fake pane through a [TerminalState] of its own. */
    private fun hardware(layout: KeyLayout): TerminalKeyboard {
        state = TerminalState(pane, EmptyCoroutineContext)
        return TerminalKeyboard(layout = layout).also { it.target = state }
    }

    private fun TerminalKeyboard.stroke(
        keyCode: Int,
        meta: Int = 0,
    ) {
        key(KeyEvent(0L, 0L, KeyEvent.ACTION_DOWN, keyCode, 0, meta))
        key(KeyEvent(0L, 0L, KeyEvent.ACTION_UP, keyCode, 0, meta))
    }

    // what reached the program as text: typed text and the keys' text, in order
    private fun texts(): List<String> =
        pane.inputs
            .mapNotNull {
                when (it) {
                    is FakeInput.Typed -> {
                        it.text
                    }

                    is FakeInput.Key -> {
                        it.key.text.takeIf { _ ->
                            it.key.action == PaneKeyAction.PRESS && !it.key.composing
                        }
                    }

                    else -> {
                        null
                    }
                }
            }.filter { it.isNotEmpty() }

    @Test
    fun aDeadKeyHoldsItsAccentForTheNextKey() {
        val german = hardware(TestLayout.German)
        german.key(KeyEvent(0L, 0L, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_EQUALS, 0, 0))
        assertEquals("the accent waits at the cursor", "´", state.composing)
        german.key(KeyEvent(0L, 0L, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_EQUALS, 0, 0))
        german.stroke(KeyEvent.KEYCODE_E)
        assertEquals("", state.composing)
        assertEquals(listOf("é"), texts())
        val combined = presses().last()
        assertEquals("key_e", combined.key)

        pane.inputs.clear()
        german.stroke(KeyEvent.KEYCODE_GRAVE)
        german.stroke(KeyEvent.KEYCODE_SPACE)
        german.stroke(KeyEvent.KEYCODE_GRAVE)
        german.stroke(KeyEvent.KEYCODE_GRAVE)
        german.stroke(KeyEvent.KEYCODE_EQUALS)
        german.stroke(KeyEvent.KEYCODE_Q)
        german.stroke(KeyEvent.KEYCODE_GRAVE)
        german.stroke(KeyEvent.KEYCODE_C, KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON)
        german.stroke(KeyEvent.KEYCODE_GRAVE)
        german.stroke(KeyEvent.KEYCODE_ENTER)
        assertEquals(
            "a space or the same accent types it; a key it doesn't go on, or a chord, comes after it",
            listOf("^", "^", "´", "q", "^", "c", "^"),
            texts(),
        )
        assertEquals(
            listOf("key_c" to KeyMapping.CTRL, "enter" to 0),
            presses().filter { it.key == "key_c" || it.key == "enter" }.map { it.key to it.mods.toInt() },
        )
    }

    @Test
    fun altGrTypesTheLayoutsCharacters() {
        val german = hardware(TestLayout.German)
        german.stroke(KeyEvent.KEYCODE_Q, KeyEvent.META_ALT_ON or KeyEvent.META_ALT_RIGHT_ON)
        german.stroke(KeyEvent.KEYCODE_RIGHT_BRACKET, KeyEvent.META_ALT_ON or KeyEvent.META_ALT_RIGHT_ON)
        assertEquals(listOf("@", "~"), texts())
        assertTrue(presses().all { it.consumed.toInt() and KeyMapping.ALT != 0 })
    }

    @Test
    fun ctrlBracketIsTheProgramsOnDvorak() {
        val dvorak = hardware(TestLayout.Dvorak)
        val ctrl = KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON
        dvorak.stroke(KeyEvent.KEYCODE_MINUS, ctrl)
        dvorak.stroke(KeyEvent.KEYCODE_EQUALS, ctrl)
        assertEquals(TerminalTextSize, state.textSize)
        assertEquals(listOf("[", "]"), presses().map { it.text })
        assertTrue(presses().all { it.mods.toInt() == KeyMapping.CTRL })
        dvorak.stroke(KeyEvent.KEYCODE_RIGHT_BRACKET, ctrl)
        assertEquals("Dvorak's '=' key is the chord", 14.sp, state.textSize)
    }

    @Test
    fun theRowStandsAsideForAHardwareKeyboard() {
        compose.setContent {
            // as Android configures a phone with a keyboard attached and open
            val attached =
                Configuration(LocalConfiguration.current).apply {
                    keyboard = Configuration.KEYBOARD_QWERTY
                    hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
                }
            CompositionLocalProvider(LocalConfiguration provides attached) {
                NgtermTheme(darkTheme = true) { ExtraKeysRow(keyboard) }
            }
        }
        compose.onNodeWithContentDescription("Escape").assertDoesNotExist()
    }

    private companion object {
        const val HOLD_MS = 700L

        // the press, then repeats from 400 ms every 50 ms while held to 700 ms
        const val MIN_REPEATS = 4
    }
}
