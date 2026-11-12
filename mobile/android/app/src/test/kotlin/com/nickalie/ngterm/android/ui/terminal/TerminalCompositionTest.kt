package com.nickalie.ngterm.android.ui.terminal

import android.content.ClipData
import android.content.ClipboardManager
import android.content.res.Configuration
import android.icu.text.BreakIterator
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
import androidx.compose.ui.focus.FocusManager
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.withKeyDown
import androidx.compose.ui.test.withKeysDown
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import com.nickalie.ngterm.core.ffi.PaneWheelOutcome
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
 * The soft keyboard's compositions against a fake pane that records its input: each letter reaches the program as it
 * is composed, a change comes out the same in every line editor, and a key, the pointer or a paste from elsewhere ends
 * the composition in the right order.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalCompositionTest {
    @get:Rule
    val compose = createComposeRule()

    private val pane = SelectingPane()
    private lateinit var state: TerminalState
    private lateinit var view: View
    private lateinit var focusManager: FocusManager
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
                focusManager = LocalFocusManager.current
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

    /** How a program's line editor takes back the last character on a backspace. */
    private enum class Erase {
        /** A grapheme cluster: Claude Code, Codex. */
        Cluster,

        /** A code point that takes room, with the zero-width ones after it: readline, zsh. */
        Readline,

        /** A code point: a terminal's own line editing (IUTF8). */
        CodePoint,
    }

    // the program's input line built from the pane's input, a backspace erasing as [erase] does
    private fun line(erase: Erase): String =
        pane.inputs.fold("") { line, input ->
            when {
                input is FakeInput.Typed -> {
                    line + input.text
                }

                input is FakeInput.Paste && (input.allowUnsafe || '\n' !in input.text) -> {
                    line + input.text
                }

                input is FakeInput.Key && input.key.action == PaneKeyAction.PRESS && input.key.key == "backspace" -> {
                    line.take(line.erased(erase))
                }

                else -> {
                    line
                }
            }
        }

    private fun String.erased(erase: Erase): Int {
        if (isEmpty()) return 0
        return when (erase) {
            Erase.Cluster -> {
                BreakIterator.getCharacterInstance().also { it.setText(this) }.preceding(length)
            }

            Erase.CodePoint -> {
                offsetByCodePoints(length, -1)
            }

            Erase.Readline -> {
                var at = offsetByCodePoints(length, -1)
                while (at > 0 && codePointAt(at).zeroWidth()) at = offsetByCodePoints(at, -1)
                at
            }
        }
    }

    private fun Int.zeroWidth() =
        Character.getType(this).toByte() in
            setOf(Character.NON_SPACING_MARK, Character.ENCLOSING_MARK, Character.FORMAT)

    /** The program's line, the same whichever way its line editor erases. */
    private fun line(): String {
        val lines = Erase.entries.associateWith(::line)
        assertEquals("the line in every line editor: $lines", 1, lines.values.toSet().size)
        return lines.getValue(Erase.Cluster)
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
    fun aCompositionChangesTheSameInEveryLineEditor() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("क", 1)
        connection.setComposingText("का", 1)
        connection.setComposingText("कै", 1)
        assertEquals("a letter and its signs wait at the cursor", "" to "कै", line() to state.composing)
        connection.setComposingText("कि", 1)
        connection.commitText("कि ", 1)
        connection.setComposingText("क्ष", 1)
        connection.setComposingText("क्", 1)
        connection.commitText("क्त ", 1)
        assertEquals("vowel signs and conjuncts", "कि क्त ", line())
        assertTrue("nothing is taken back", presses().isEmpty())

        connection.setComposingText("e", 1)
        assertEquals("a lone letter goes at once", "कि क्त e", line())
        connection.setComposingText("e\u0301", 1)
        connection.setComposingText("e\u0300", 1)
        connection.commitText("e\u0300 ", 1)
        assertEquals("कि क्त e\u0300 ", line())
        connection.setComposingText("a\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67", 1)
        connection.setComposingText("a\uD83D\uDC68\u200D\uD83D\uDC69", 1)
        assertEquals("an emoji waits", "\uD83D\uDC68\u200D\uD83D\uDC69", state.composing)
        connection.commitText("a\uD83D\uDC68\u200D\uD83D\uDC69 ", 1)
        connection.setComposingText("teh", 1)
        connection.commitText("the ", 1)
        assertEquals(
            "accents, emoji sequences and a corrected word",
            "कि क्त e\u0300 a\uD83D\uDC68\u200D\uD83D\uDC69 the ",
            line(),
        )
        assertEquals("", state.composing)
        assertEquals("backspaces only over lone letters", List(3) { press("backspace") }, presses())
    }

    @Test
    fun aConsonantAfterAViramaWaitsInEveryConjunctScript() {
        show()
        focus()
        val connection = connect()
        // Myanmar and Khmer, two of the scripts whose conjuncts joined grapheme clusters in Unicode 16
        var typed = ""
        for ((virama, first, second) in listOf(Triple("က္", "က", "ခ"), Triple("ក្", "ក", "ខ"))) {
            connection.commitText(virama, 1)
            connection.setComposingText(first, 1)
            assertEquals("a consonant waits", typed + virama to first, line() to state.composing)
            connection.setComposingText(second, 1)
            connection.commitText("$second ", 1)
            typed += "$virama$second "
        }
        assertEquals("က္ခ ក្ខ ", line())
        assertTrue("nothing is taken back", presses().isEmpty())
    }

    @Test
    fun aDeletionBeforeAHeldMarkLeavesTheLetterItGoesOn() {
        show()
        focus()
        val connection = connect()
        connection.commitText("xy", 1)
        connection.setComposingText("\u0301", 1)
        connection.deleteSurroundingText(1, 0)
        connection.commitText("\u0301", 1)
        assertEquals("x\u0301", line())
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
        connection.setComposingText("", 1)
        compose.waitForIdle()
        assertEquals(
            "a composition after the tap still gets the modifier, and the chord ends it: nothing to take back",
            listOf(KeyMapping.forText("c", KeyMapping.CTRL)),
            presses(),
        )
        connection.setComposingText("d", 1)
        assertEquals(FakeInput.Typed("d"), pane.inputs.last())
    }

    @Test
    fun aDeletionIsAroundTheCompositionAsInTheFrameworksEditors() {
        show()
        focus()
        val connection = connect()
        connection.commitText("xy", 1)
        connection.setComposingText("ab", 1)
        connection.deleteSurroundingText(1, 0)
        assertEquals("the composition stays, the letter before it goes", "xab", line())
        connection.commitText("abc", 1)
        assertEquals("xabc", line())
        connection.setComposingText("\uD83D\uDE00", 1)
        connection.deleteSurroundingTextInCodePoints(0, 2)
        assertEquals(List(2) { press("delete") }, presses().takeLast(2))
        connection.setComposingText("\uD83D\uDE00!", 1)
        assertEquals("the composition goes on", "xabc" to "\uD83D\uDE00!", line() to state.composing)
        connection.finishComposingText()
        assertEquals("xabc\uD83D\uDE00!", line())
        assertEquals("", state.composing)
    }

    @Test
    fun aTapThePointerSendsEndsTheComposition() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("teh", 1)
        pane.tracking = true
        terminal.performTouchInput { click(center) }
        compose.waitForIdle()
        connection.commitText("the ", 1)
        assertTrue(
            "the click may have moved the cursor: nothing is taken back",
            presses().none { it.key == "backspace" },
        )
        assertEquals("tehthe ", line())
    }

    @Test
    fun aWheelTheProgramTakesEndsTheComposition() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("ab", 1)
        terminal.performTouchInput { swipeUp() }
        compose.waitForIdle()
        connection.setComposingText("a", 1)
        assertEquals("a scroll of the view leaves the composition", "a", line())
        pane.wheelOutcome = PaneWheelOutcome.CURSOR_KEYS
        terminal.performTouchInput { swipeUp() }
        compose.waitForIdle()
        connection.setComposingText("", 1)
        assertEquals("cursor keys end it", "a", line())
    }

    @Test
    fun aPasteEndsTheComposition() {
        show()
        focus()
        val clipboard = RuntimeEnvironment.getApplication().getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("test", "X"))
        val connection = connect()
        connection.setComposingText("helo", 1)
        paste()
        connection.commitText("hello", 1)
        assertEquals("heloXhello", line())
    }

    @Test
    fun aPasteThatWaitsForAYesLeavesTheComposition() {
        show()
        focus()
        val clipboard = RuntimeEnvironment.getApplication().getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("test", "rm -rf\n"))
        val connection = connect()
        connection.setComposingText("helo", 1)
        paste()
        connection.commitText("hello", 1)
        assertEquals("hello", line())
    }

    @Test
    fun heldTextGoesBeforeAPasteOrThePointer() {
        show()
        focus()
        val clipboard = RuntimeEnvironment.getApplication().getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("test", "X"))
        val connection = connect()
        connection.setComposingText("कै", 1)
        paste()
        assertEquals("कैX", line())
        pane.inputs.clear()
        connection.setComposingText("कि", 1)
        terminal.performTouchInput { swipeUp() }
        compose.waitForIdle()
        assertEquals(FakeInput.Typed("कि"), pane.inputs.first())
        assertTrue(pane.inputs.any { it is FakeInput.Wheel })
        pane.inputs.clear()
        pane.tracking = true
        connection.setComposingText("कु", 1)
        terminal.performTouchInput { click(center) }
        compose.waitForIdle()
        assertEquals(FakeInput.Typed("कु"), pane.inputs.first())
        assertEquals("", state.composing)
    }

    // a selection's Paste, from the toolbar a long press opens
    private fun paste() {
        terminal.performTouchInput { longClick(center) }
        compose.onNodeWithText("Paste").performClick()
        compose.waitForIdle()
    }

    @Test
    fun theMouseMovingOverTheProgramLeavesTheComposition() {
        show()
        focus()
        val connection = connect()
        pane.tracking = true
        connection.setComposingText("teh", 1)
        terminal.performMouseInput {
            moveTo(topLeft)
            moveTo(center)
        }
        compose.waitForIdle()
        assertTrue("the program heard the motion", pane.inputs.any { it is FakeInput.Mouse })
        connection.commitText("the ", 1)
        assertEquals("the ", line())
        connection.setComposingText("कै", 1)
        terminal.performMouseInput { moveTo(topLeft) }
        compose.waitForIdle()
        assertEquals("a held word waits on", "the " to "कै", line() to state.composing)
        connection.commitText("कि ", 1)
        assertEquals("the कि ", line())
    }

    @Test
    fun aHeldWordGoesWhereItWasTypedWhenTheTerminalLosesTheFocus() {
        show()
        focus()
        val connection = connect()
        connection.setComposingText("aकै", 1)
        assertEquals("a" to "कै", line() to state.composing)
        compose.runOnIdle { focusManager.clearFocus() }
        compose.waitForIdle()
        assertEquals("aकै" to "", line() to state.composing)
        connection.commitText("x", 1)
        assertEquals("nothing reaches the terminal once it let the connection go", "aकै", line())
    }

    @Test
    fun thePointerOnAnotherTerminalLeavesTheComposition() {
        val first = TerminalState(pane, EmptyCoroutineContext)
        val second = TerminalState(SelectingPane(), EmptyCoroutineContext)
        keyboard.target = first
        val connection =
            TerminalInputConnection(View(RuntimeEnvironment.getApplication()), keyboard, first)
                .also(keyboard.connections::bind)
        connection.setComposingText("teh", 1)
        keyboard.endComposition(second)
        connection.commitText("the ", 1)
        assertEquals("the ", line())
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
        assertEquals(listOf(FakeInput.Typed("vi")), pane.inputs.toList())
        keyboard.target = second
        assertEquals(listOf(FakeInput.Typed("vi")), pane.inputs.toList())
        connection.commitText("x", 1)
        connection.closeConnection()
        assertEquals(listOf(FakeInput.Typed("vi")), pane.inputs.toList())
        assertTrue("nothing crosses to the terminal targeted since", other.inputs.isEmpty())
    }
}
