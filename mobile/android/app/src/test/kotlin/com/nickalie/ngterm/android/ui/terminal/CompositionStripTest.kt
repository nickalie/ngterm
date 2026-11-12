package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Bitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.math.roundToInt

/**
 * What waits to be typed, in its strip over the terminal's bottom edge as the session view places it, never on the
 * grid (src/test/screenshots/terminal): a held Hindi word, an emoji sequence and a word too long for the width, dark
 * and light.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class CompositionStripTest {
    @get:Rule
    val compose = createComposeRule()

    private val keyboard = TerminalKeyboard()
    private lateinit var state: TerminalState

    private fun show(dark: Boolean = true) {
        val pane = SelectingPane(SHELL, FakeCursor(SHELL.last().length, SHELL.lastIndex))
        compose.setContent {
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
                        CompositionStrip(keyboard, Modifier.align(Alignment.BottomEnd).padding(8.dp))
                    }
                    ExtraKeysRow(keyboard)
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

    private fun Rect.inside(outer: Rect) =
        left >= outer.left && right <= outer.right && top >= outer.top && bottom <= outer.bottom

    // the terminal's pixels above [bottom] pixels from its top: what the strip leaves in view
    private fun Bitmap.above(bottom: Int): List<Int> =
        (0 until bottom).flatMap { y -> (0 until width).map { x -> getPixel(x, y) } }

    private fun shot(name: String) = compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")

    @Test
    fun nothingIsDrawnOnTheGridWhileTextIsHeld() {
        show()
        val grid = state.grid
        val before = terminal.captureToImage().asAndroidBitmap()
        hold(HINDI)
        val view = terminal.fetchSemanticsNode().boundsInRoot
        val chip = strip(HINDI).fetchSemanticsNode().boundsInRoot
        assertTrue("the strip floats over the terminal's bottom edge: $chip in $view", chip.inside(view))
        val clear = (chip.top - view.top).roundToInt()
        val cursorRow = requireNotNull(state.screen.header).cursor.row
        val cursorBottom = (cursorRow + 1) * requireNotNull(grid).cellHeight.toInt()
        assertTrue("the cursor's row stays in view", cursorBottom <= clear)
        val after = terminal.captureToImage().asAndroidBitmap()
        assertTrue("the grid is drawn as it was", before.above(clear) == after.above(clear))
        assertEquals("and keeps its size", grid, state.grid)
        shot("strip_word_dark")
    }

    @Test
    fun theStripShowsOnlyWhileSomethingWaits() {
        show()
        hold(HINDI)
        strip(HINDI).assertExists()
        hold("")
        strip(HINDI).assertDoesNotExist()
        compose.onNodeWithContentDescription("Waiting to be typed:", substring = true).assertDoesNotExist()
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
    fun aLongWordScrollsInTheWidth() {
        show()
        val long = HINDI_SENTENCE
        hold(long)
        val root = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val chip = strip(long).fetchSemanticsNode().boundsInRoot
        assertTrue("the strip stays on screen: $chip in $root", chip.inside(root))
        val margin = with(compose.density) { 8.dp.toPx() }
        assertEquals("and takes the width, its text scrolling in it", margin, chip.left - root.left, 1f)
        shot("strip_long_dark")
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
