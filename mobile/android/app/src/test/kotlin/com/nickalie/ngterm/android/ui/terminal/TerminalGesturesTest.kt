package com.nickalie.ngterm.android.ui.terminal

import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.click
import androidx.compose.ui.test.doubleClick
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pinch
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.PaneMouseAction
import com.nickalie.ngterm.core.ffi.PaneMouseButton
import com.nickalie.ngterm.core.ffi.PaneSize
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * Each gesture on the terminal against a fake pane that records its input: a finger's drag, fling, long press,
 * handle drag, toolbar, pinch and tap; a mouse's clicks, drags, wheel and right-click menu, with and without a
 * program tracking the mouse; and the confirmation of an unsafe paste.
 */
@OptIn(ExperimentalTestApi::class)
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalGesturesTest {
    @get:Rule
    val compose = createComposeRule()

    private val pane = SelectingPane()
    private lateinit var state: TerminalState

    private val terminal get() = compose.onNodeWithContentDescription("Terminal")

    private val grid get() = requireNotNull(state.grid)

    private val clipboard: ClipboardManager
        get() = RuntimeEnvironment.getApplication().getSystemService(ClipboardManager::class.java)

    private fun show() {
        compose.setContent {
            NgtermTheme(darkTheme = true) {
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                this.state = state
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    onDispose { pane.onWake = null }
                }
                TerminalView(state, Modifier.fillMaxSize())
            }
        }
        compose.waitForIdle()
    }

    /** The middle of cell ([column], [row]), in the view's pixels (near enough: the pane's cell is rounded). */
    private fun cell(
        column: Int,
        row: Int,
    ) = Offset((column + 0.5f) * grid.cellWidth.toFloat(), (row + 0.5f) * grid.cellHeight.toFloat())

    private fun point(
        column: Int,
        row: Int,
    ) = CellPoint(column.toUShort(), row.toUInt())

    private inline fun <reified T : FakeInput> inputs(): List<T> = pane.inputs.filterIsInstance<T>()

    private fun wheelRows() = inputs<FakeInput.Wheel>().map { it.wheel.rows }

    @Test
    fun aDragScrollsByTheRowsItCovers() {
        show()
        val rowHeight = grid.cellHeight.toFloat()
        terminal.performTouchInput {
            down(cell(5, 2))
            repeat(STEPS) { moveBy(Offset(0f, rowHeight * ROWS_DRAGGED / STEPS)) }
            // held still before letting go: no fling
            advanceEventTime(HOLD_MS)
            up()
        }
        compose.waitForIdle()
        val rows = wheelRows()
        assertTrue("every turn is up, toward the scrollback: $rows", rows.all { it > 0 })
        assertEquals("a row per row dragged", ROWS_DRAGGED, rows.sum())

        pane.inputs.clear()
        terminal.performTouchInput {
            down(cell(5, 30))
            repeat(STEPS) { moveBy(Offset(0f, -rowHeight * ROWS_DRAGGED / STEPS)) }
            advanceEventTime(HOLD_MS)
            up()
        }
        compose.waitForIdle()
        assertEquals("dragging up turns down", -ROWS_DRAGGED, wheelRows().sum())
    }

    @Test
    fun aFlickGoesOnScrollingAfterTheFingerLeaves() {
        show()
        val rowHeight = grid.cellHeight.toFloat()
        terminal.performTouchInput { swipeDown(startY = rowHeight, endY = rowHeight * 4, durationMillis = 60) }
        compose.mainClock.advanceTimeBy(FLING_MS)
        compose.waitForIdle()
        assertTrue("more rows than the finger covered: ${wheelRows()}", wheelRows().sum() > ROWS_DRAGGED)
    }

    @Test
    fun aLongPressSelectsTheWordWithHandlesAndCopyCopiesIt() {
        show()
        pane.selection = "beta"
        terminal.performTouchInput { longClick(cell(6, 1)) }
        compose.waitForIdle()
        assertEquals(listOf(FakeInput.SelectWord(point(6, 1))), pane.inputs)
        compose.onNodeWithText("Copy").performClick()
        compose.waitForIdle()
        assertEquals("beta", clipboard.primaryClip?.getItemAt(0)?.text)
        assertEquals(FakeInput.ClearSelection, pane.inputs.last())
        compose.onNodeWithText("Copy").assertDoesNotExist()
    }

    /** A finger long-presses [at], held until a frame shows the word, then drags to [to] and lifts. */
    private fun longPressAndDrag(
        at: Offset,
        to: Offset,
    ) {
        terminal.performTouchInput { down(at) }
        compose.mainClock.advanceTimeBy(LONG_PRESS_MS)
        compose.waitForIdle()
        terminal.performTouchInput {
            moveTo(to)
            up()
        }
        compose.waitForIdle()
    }

    @Test
    fun aLongPressThenADragExtendsTheSelection() {
        show()
        // "alpha" is cells 0 to 4 of row 1: dragging on from its 'p' keeps the word and extends from its start
        longPressAndDrag(cell(2, 1), cell(9, 3))
        assertEquals(
            listOf(FakeInput.SelectWord(point(2, 1)), FakeInput.Select(point(0, 1), point(9, 3), false)),
            pane.inputs,
        )
        compose.onNodeWithText("Select all").performClick()
        assertEquals(FakeInput.SelectAll, pane.inputs.last())
    }

    @Test
    fun aLongPressThenADragBackExtendsFromTheWordsEnd() {
        show()
        // "beta" is cells 6 to 9 of row 1: dragging back from its 't' keeps the word and extends from its end
        longPressAndDrag(cell(8, 1), cell(1, 0))
        assertEquals(FakeInput.Select(point(9, 1), point(1, 0), false), pane.inputs.last())
    }

    /** The end handle's knob of a selection whose last cell is ([column] - 1, [row]): right of the edge, under it. */
    private fun endKnob(
        column: Int,
        row: Int,
    ): Offset {
        val knob = 11f * 2
        return Offset(column * grid.cellWidth.toFloat() + knob, (row + 1) * grid.cellHeight.toFloat() + knob)
    }

    @Test
    fun draggingAHandleMovesItsEndOfTheSelection() {
        show()
        terminal.performTouchInput { longClick(cell(2, 1)) }
        compose.waitForIdle()
        // the word is "alpha", cells 0 to 4 of row 1
        terminal.performTouchInput {
            down(endKnob(5, 1))
            moveBy(Offset(grid.cellWidth.toFloat() * 3, grid.cellHeight.toFloat() * 2))
            up()
        }
        compose.waitForIdle()
        assertEquals(FakeInput.Select(point(0, 1), point(7, 3), false), pane.inputs.last())
    }

    @Test
    fun aHandleIsTakenWhereItIsDrawnAfterAPinch() {
        show()
        terminal.performTouchInput {
            pinch(
                start0 = center - Offset(PINCH_FROM, 0f),
                end0 = center - Offset(PINCH_TO, 0f),
                start1 = center + Offset(PINCH_FROM, 0f),
                end1 = center + Offset(PINCH_TO, 0f),
            )
        }
        compose.waitForIdle()
        assertTrue("larger text: ${state.textSize}", state.textSize > TerminalTextSize)
        terminal.performTouchInput { longClick(cell(2, 1)) }
        compose.waitForIdle()
        pane.inputs.clear()
        terminal.performTouchInput {
            down(endKnob(5, 1))
            moveBy(Offset(grid.cellWidth.toFloat() * 2, grid.cellHeight.toFloat()))
            up()
        }
        compose.waitForIdle()
        assertEquals(listOf(FakeInput.Select(point(0, 1), point(6, 2), false)), pane.inputs)
    }

    @Test
    fun aCanceledTouchClicksNothing() {
        show()
        pane.tracking = true
        terminal.performTouchInput {
            down(cell(3, 2))
            cancel()
        }
        compose.waitForIdle()
        assertEquals("a canceled touch is no tap", emptyList<FakeInput>(), pane.inputs)
    }

    @Test
    fun pasteConfirmsTextThatCanRunACommand() {
        show()
        clipboard.setPrimaryClip(ClipData.newPlainText("test", "rm -rf build\n"))
        terminal.performTouchInput { longClick(cell(2, 1)) }
        compose.onNodeWithText("Paste").performClick()
        compose.waitForIdle()
        assertEquals(FakeInput.Paste("rm -rf build\n", false), pane.inputs.last())
        compose.onNodeWithText("Paste text that can run a command?").assertExists()
        compose.onNodeWithText("rm -rf build\n").assertExists()
        compose.onNodeWithText("Paste").performClick()
        compose.waitForIdle()
        assertEquals(FakeInput.Paste("rm -rf build\n", true), pane.inputs.last())
        compose.onNodeWithText("Paste text that can run a command?").assertDoesNotExist()
    }

    @Test
    fun aPinchChangesTheTextSizeAndTheGrid() {
        show()
        val before = grid
        terminal.performTouchInput {
            pinch(
                start0 = center - Offset(PINCH_FROM, 0f),
                end0 = center - Offset(PINCH_TO, 0f),
                start1 = center + Offset(PINCH_FROM, 0f),
                end1 = center + Offset(PINCH_TO, 0f),
            )
        }
        compose.waitForIdle()
        assertTrue("larger text: ${state.textSize}", state.textSize > TerminalTextSize)
        assertEquals("the pinch's own steps are whole sp", 0f, state.textSize.value % 1f)
        assertTrue(
            "fewer, larger cells: $before -> $grid",
            grid.cols < before.cols && grid.cellHeight > before.cellHeight,
        )
        assertEquals(grid, pane.sizes.last())
        assertEquals("a pinch is no scroll", emptyList<Int>(), wheelRows())

        terminal.performTouchInput {
            pinch(
                start0 = center - Offset(PINCH_TO * 2, 0f),
                end0 = center - Offset(PINCH_NEAR, 0f),
                start1 = center + Offset(PINCH_TO * 2, 0f),
                end1 = center + Offset(PINCH_NEAR, 0f),
            )
        }
        compose.waitForIdle()
        assertEquals("never below the smallest size", TerminalTextSizes.start.sp, state.textSize)
    }

    @Test
    fun aTapClicksOnlyForAProgramTrackingTheMouse() {
        show()
        terminal.performTouchInput { click(cell(3, 2)) }
        compose.waitForIdle()
        assertEquals("no program asked: nothing to send", emptyList<FakeInput>(), pane.inputs)

        pane.tracking = true
        terminal.performTouchInput { click(cell(3, 2)) }
        compose.waitForIdle()
        val mice = inputs<FakeInput.Mouse>().map { it.mouse }
        assertEquals(listOf(PaneMouseAction.PRESS, PaneMouseAction.RELEASE), mice.map { it.action })
        assertTrue(mice.all { it.button == PaneMouseButton.LEFT })
        // the pane's pixels land in the cell tapped
        assertEquals(3, (mice[0].x / grid.cellWidth.toFloat()).toInt())
        assertEquals(2, (mice[0].y / grid.cellHeight.toFloat()).toInt())
    }

    @Test
    fun theMouseGoesToAProgramTrackingIt() {
        show()
        pane.tracking = true
        terminal.performMouseInput {
            moveTo(cell(1, 1))
            press()
            moveTo(cell(4, 2))
            release()
        }
        compose.waitForIdle()
        val pressed = inputs<FakeInput.Mouse>().map { it.mouse }.filter { it.button != null }
        assertEquals(
            listOf(PaneMouseAction.PRESS, PaneMouseAction.MOTION, PaneMouseAction.RELEASE),
            pressed.map { it.action },
        )
        assertTrue(
            "hovering reports a motion with no button",
            inputs<FakeInput.Mouse>().any { it.mouse.button == null },
        )
        assertEquals("no selection for a tracked drag", emptyList<FakeInput.Select>(), inputs<FakeInput.Select>())
    }

    @Test
    fun theMouseSelectsByDragAndDoubleClick() {
        show()
        terminal.performMouseInput {
            moveTo(cell(1, 1))
            press()
            moveTo(cell(4, 2))
            release()
        }
        compose.waitForIdle()
        assertEquals(FakeInput.Select(point(1, 1), point(4, 2), false), pane.inputs.last())
        // a mouse selection has no handles and no toolbar
        compose.onNodeWithText("Copy").assertDoesNotExist()

        pane.inputs.clear()
        terminal.performMouseInput { doubleClick(cell(7, 3)) }
        compose.waitForIdle()
        assertEquals(FakeInput.SelectWord(point(7, 3)), pane.inputs.last())
    }

    @Test
    fun theWheelScrollsAndTheRightButtonOpensTheMenu() {
        show()
        terminal.performMouseInput {
            moveTo(cell(1, 1))
            scroll(-WHEEL_STEPS)
        }
        compose.waitForIdle()
        assertEquals("a wheel turned up scrolls up", WHEEL_STEPS.toInt(), wheelRows().sum())

        terminal.performMouseInput { rightClick(cell(2, 2)) }
        compose.waitForIdle()
        compose.onNodeWithText("Copy").assertExists()
        compose.onNodeWithText("Select all").performClick()
        compose.waitForIdle()
        assertEquals(FakeInput.SelectAll, pane.inputs.last())
        compose.onNodeWithText("Select all").assertDoesNotExist()
    }

    private companion object {
        const val STEPS = 10
        const val ROWS_DRAGGED = 5
        const val HOLD_MS = 200L
        const val LONG_PRESS_MS = 1_000L
        const val FLING_MS = 2_000L
        const val PINCH_FROM = 50f
        const val PINCH_TO = 150f
        const val PINCH_NEAR = 5f
        const val WHEEL_STEPS = 3f
    }
}
