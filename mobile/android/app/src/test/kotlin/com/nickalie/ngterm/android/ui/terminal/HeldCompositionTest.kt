package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Bitmap
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
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
import kotlin.math.ceil

/**
 * Text the soft keyboard holds at the cursor (a letter with its signs, an emoji sequence) as the terminal will place
 * it: from the cursor on, onto the next row at the right edge, the screen moving up when it runs past the last row,
 * and never over the text before the cursor (src/test/screenshots/terminal).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class HeldCompositionTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var state: TerminalState
    private lateinit var font: TerminalFont
    private var grid = PaneSize(0u, 0u, 0u, 0u)

    private val columns get() = grid.cols.toInt()
    private val rows get() = grid.rows.toInt()

    /** Shows a screen full of text, the cursor where [cursor] puts it on the grid. */
    private fun show(cursor: (PaneSize) -> FakeCursor) {
        val pane =
            SelectingPane(LINES) {
                grid = it
                cursor(it)
            }
        compose.setContent {
            NgtermTheme(darkTheme = true) {
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                this.state = state
                font = rememberTerminalFont(state.textSize, state.face)
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    onDispose { pane.onWake = null }
                }
                TerminalView(state, Modifier.fillMaxWidth().height(TERMINAL_HEIGHT))
            }
        }
        compose.waitForIdle()
    }

    private fun drawn(): Bitmap =
        compose
            .onNodeWithContentDescription("Terminal")
            .captureToImage()
            .asAndroidBitmap()

    private fun blank(): Set<Int> = setOf(requireNotNull(state.screen.header).defaultBackground)

    private fun hold(text: String): Bitmap {
        state.composing = text
        compose.waitForIdle()
        return drawn()
    }

    // the pixels of [columns] on [row], kept clear of the cells' edges, which antialiasing shares with the neighbors
    private fun Bitmap.cells(
        row: Int,
        columns: IntRange,
    ): List<Int> {
        val top = font.top(row).toInt()
        val left = ceil(font.left(columns.first)).toInt() + 1
        val right = font.left(columns.last + 1).toInt() - 1
        return (top until top + font.cellHeight).flatMap { y -> (left until right).map { x -> getPixel(x, y) } }
    }

    private fun same(
        message: String,
        expected: List<Int>,
        actual: List<Int>,
    ) = assertTrue("$message: ${expected.indices.count { expected[it] != actual[it] }} differ", expected == actual)

    private fun differs(
        message: String,
        before: List<Int>,
        after: List<Int>,
    ) = assertTrue(message, before != after)

    @Test
    fun aHeldWordAtTheRightEdgeGoesOnTheNextRow() {
        show { FakeCursor(it.cols.toInt() - 2, MIDDLE) }
        val before = drawn()
        val held = hold(HINDI)
        same(
            "the text before the cursor stays",
            before.cells(MIDDLE, 0 until columns - 2),
            held.cells(MIDDLE, 0 until columns - 2),
        )
        differs(
            "the word goes on at the next row",
            before.cells(MIDDLE + 1, 0..0),
            held.cells(MIDDLE + 1, 0..0),
        )
        same(
            "and covers only its own cells there",
            before.cells(MIDDLE + 1, WRAPPED until columns),
            held.cells(MIDDLE + 1, WRAPPED until columns),
        )
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/held_word_edge.png")
    }

    @Test
    fun aWideCharacterThatDoesNotFitTheRowStartsTheNext() {
        show { FakeCursor(it.cols.toInt() - 1, MIDDLE) }
        val before = drawn()
        val held = hold(EMOJI)
        same(
            "the row is left as it is",
            before.cells(MIDDLE, 0 until columns - 1),
            held.cells(MIDDLE, 0 until columns - 1),
        )
        assertEquals("but for its last cell, blank", blank(), held.cells(MIDDLE, columns - 1 until columns).toSet())
        val next = MIDDLE + 1
        differs("the character is drawn there", before.cells(next, 0..1), held.cells(next, 0..1))
        same("and nothing after it", before.cells(next, 2 until columns), held.cells(next, 2 until columns))
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/held_emoji_edge.png")
    }

    @Test
    fun aHeldWordPastTheLastRowMovesTheScreenUp() {
        show { FakeCursor(it.cols.toInt() - 2, it.rows.toInt() - 1) }
        val before = drawn()
        val held = hold(HINDI)
        for (row in 0 until rows - 2) {
            same(
                "row ${row + 1} moved up a row",
                before.cells(row + 1, 0 until columns),
                held.cells(row, 0 until columns),
            )
        }
        same(
            "the text before the cursor moved up with its row",
            before.cells(rows - 1, 0 until columns - 2),
            held.cells(rows - 2, 0 until columns - 2),
        )
        differs(
            "the word goes on at the new last row",
            before.cells(rows - 1, 0..0),
            held.cells(rows - 1, 0..0),
        )
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/held_word_bottom.png")
    }

    private companion object {
        val TERMINAL_HEIGHT = 240.dp
        const val MIDDLE = 2

        // Hindi "namaste": its letters carry signs and a conjunct, so the keyboard holds the whole word
        const val HINDI = "नमस्ते"

        // past the two cells left on the row, the word takes at most two more
        const val WRAPPED = 3
        const val EMOJI = "\uD83D\uDC68\u200D\uD83D\uDC69"
        val LINES = List(40) { row -> "$row: " + "lorem ipsum dolor sit amet ".repeat(4) }
    }
}
