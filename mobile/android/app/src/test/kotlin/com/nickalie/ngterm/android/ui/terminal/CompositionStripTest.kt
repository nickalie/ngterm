package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.PaneSize
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * What waits to be typed, in the extra keys' top line under a shell whose prompt fills the screen to its last row,
 * never on the grid (src/test/screenshots/terminal): a held Hindi word, an emoji sequence and a word too long for the
 * width, dark and light.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class CompositionStripTest {
    @get:Rule
    val compose = createComposeRule()

    private val keyboard = TerminalKeyboard()
    private val direction = mutableStateOf(LayoutDirection.Ltr)
    private lateinit var state: TerminalState

    private fun show(dark: Boolean = true) {
        val pane = PromptAtTheFoot()
        compose.setContent {
            CompositionLocalProvider(LocalLayoutDirection provides direction.value) {
                NgtermTheme(darkTheme = dark) {
                    val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                    this.state = state
                    DisposableEffect(Unit) {
                        pane.onWake = state::dirty
                        keyboard.target = state
                        onDispose { pane.onWake = null }
                    }
                    Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
                        Box(Modifier.weight(1f).fillMaxWidth()) {
                            TerminalView(state, Modifier.fillMaxSize(), keyboard = keyboard)
                        }
                        ExtraKeysRow(keyboard)
                    }
                }
            }
        }
        compose.waitForIdle()
    }

    private fun hold(text: String) {
        state.composing = text
        compose.waitForIdle()
    }

    private val terminal get() = compose.onNodeWithContentDescription("Terminal")

    private fun strip(text: String) = compose.onNodeWithContentDescription("Waiting to be typed: $text")

    private fun shot(name: String) = compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")

    @Test
    fun heldTextCoversNoCellWithTheCursorOnTheLastRow() {
        show()
        val grid = requireNotNull(state.grid)
        assertEquals(
            "the cursor is on the last row",
            grid.rows.toInt() - 1,
            requireNotNull(state.screen.header).cursor.row,
        )
        val view = terminal.fetchSemanticsNode().boundsInRoot
        val before = terminal.captureToImage().asAndroidBitmap()
        hold(HINDI)
        val chip = strip(HINDI).fetchSemanticsNode().boundsInRoot
        assertTrue("the strip is under the terminal: $chip, $view", chip.top >= view.bottom)
        assertEquals("the terminal keeps its place", view, terminal.fetchSemanticsNode().boundsInRoot)
        assertEquals("and its grid", grid, state.grid)
        val after = terminal.captureToImage().asAndroidBitmap()
        assertTrue("every cell is drawn as it was", before.sameAs(after))
        shot("strip_word_dark")
    }

    @Test
    fun theStripTakesTheTopLineOnlyWhileSomethingWaits() {
        show()
        val row =
            compose
                .onRoot()
                .fetchSemanticsNode()
                .boundsInRoot.bottom
        compose.onNodeWithContentDescription("Escape").assertExists()
        hold(HINDI)
        strip(HINDI).assertExists()
        compose.onNodeWithContentDescription("Escape").assertDoesNotExist()
        compose.onNodeWithContentDescription("Tab").assertExists()
        hold("")
        compose.onNodeWithContentDescription("Waiting to be typed:", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("Escape").assertExists()
        assertEquals(
            row,
            compose
                .onRoot()
                .fetchSemanticsNode()
                .boundsInRoot.bottom,
        )
    }

    @Test
    fun aWordInLight() {
        show(dark = false)
        hold(HINDI)
        shot("strip_word_light")
    }

    @Test
    fun anEmojiSequence() {
        show()
        hold(EMOJI)
        strip(EMOJI).assertExists()
        shot("strip_emoji_dark")
    }

    @Test
    fun aLongWordScrollsInTheLine() {
        show()
        hold(HINDI_SENTENCE)
        val root = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val chip = strip(HINDI_SENTENCE).fetchSemanticsNode().boundsInRoot
        assertEquals("the strip takes the width", root.width, chip.width, 1f)
        assertEquals("on one key line", with(compose.density) { 48.dp.toPx() }, chip.height, 1f)
        shot("strip_long_dark")
    }

    @Test
    fun aRightToLeftScreenShowsTheEndOfLeftToRightText() {
        direction.value = LayoutDirection.Rtl
        show()
        hold(HINDI_SENTENCE)
        val rtl = strip(HINDI_SENTENCE).captureToImage().asAndroidBitmap()
        direction.value = LayoutDirection.Ltr
        compose.waitForIdle()
        val ltr = strip(HINDI_SENTENCE).captureToImage().asAndroidBitmap()
        assertTrue("the same end in view as on a left-to-right screen", rtl.sameAs(ltr))
    }

    // a shell whose screen has filled, its prompt and cursor on the grid's last row whatever the grid's size
    private class PromptAtTheFoot : FakePane() {
        private var seq = 0L

        init {
            frames = { _ ->
                val grid = sizes.lastOrNull() ?: PaneSize(40u, 6u, 1u, 1u)
                val columns = grid.cols.toInt()
                val rows = grid.rows.toInt()
                val cursor = FakeCursor(SHELL.last().length, rows - 1)
                val writer = RowBufferWriter(columns, rows, seq = ++seq, cursor = cursor)
                for ((at, line) in SHELL.withIndex()) {
                    val row = rows - SHELL.size + at
                    if (row >= 0) writer.text(row, line.take(columns))
                }
                writer.bytes()
            }
        }
    }

    private companion object {
        // Hindi "namaste": its letters carry signs and a conjunct
        const val HINDI = "नमस्ते"
        const val EMOJI = "👨‍👩‍👧"
        const val HINDI_SENTENCE = "नमस्ते दुनिया, यह एक बहुत लंबा वाक्य है जो पट्टी की चौड़ाई से आगे जाता है"
        val SHELL =
            listOf(
                "nick@build-01:~/api$ git status --short",
                " M crates/ngterm-mobile/src/api/pane.rs",
                "?? mobile/android/DESIGN.md",
                "nick@build-01:~/api$ echo ",
            )
    }
}
