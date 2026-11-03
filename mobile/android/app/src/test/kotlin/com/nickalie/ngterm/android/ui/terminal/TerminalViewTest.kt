package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.theme.CopperTerminal
import com.nickalie.ngterm.android.ui.theme.DraftingTerminal
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.PaneInputException
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
 * The view against a fake pane: the grid it gives as its size changes (a phone turned on its side), the frames it
 * asks for (whole after an attach, the changes since the frame it holds otherwise), and the colors it gives.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalViewTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var state: TerminalState
    private var size by mutableStateOf(PORTRAIT)
    private var palette by mutableStateOf(CopperTerminal)

    private fun show(pane: FakePane) {
        compose.setContent {
            val state = remember(pane) { TerminalState(pane, EmptyCoroutineContext) }
            this.state = state
            DisposableEffect(pane) {
                pane.onWake = state::dirty
                onDispose { pane.onWake = null }
            }
            Box(Modifier.fillMaxSize()) {
                TerminalView(state, Modifier.requiredSize(size), palette = palette)
            }
        }
        compose.waitForIdle()
    }

    /** A pane whose frames fill the grid last given with each row's number, at seq 1 and up; closed with none. */
    private fun numberedPane(): FakePane {
        val pane = FakePane()
        var seq = 0L
        pane.frames = { _ ->
            // as the real pane shown with no grid: no terminal until the view gives one
            val grid = pane.sizes.lastOrNull() ?: throw PaneInputException.Closed()
            seq++
            val writer = RowBufferWriter(grid.cols.toInt(), grid.rows.toInt(), seq = seq)
            for (row in 0 until grid.rows.toInt()) writer.text(row, "row $row of ${grid.rows}")
            writer.bytes()
        }
        return pane
    }

    @Test
    fun turningThePhoneGivesThePaneTheGridThatFitsAndDrawsIt() {
        val pane = numberedPane()
        show(pane)
        val portrait = pane.sizes.single()
        assertEquals("the view's grid is the one it gave", portrait, state.grid)
        // whole cells of the font: 411dp at xhdpi is 822 pixels
        assertEquals(822 / portrait.cellWidth.toInt(), portrait.cols.toInt())
        assertEquals(1782 / portrait.cellHeight.toInt(), portrait.rows.toInt())
        assertEquals("row 0 of ${portrait.rows}", state.screen.text(0))

        size = DpSize(PORTRAIT.height, PORTRAIT.width)
        compose.waitForIdle()
        val landscape = pane.sizes.last()
        assertEquals("one resize per turn", 2, pane.sizes.size)
        assertTrue("more columns: $portrait -> $landscape", landscape.cols > portrait.cols)
        assertTrue("fewer rows: $portrait -> $landscape", landscape.rows < portrait.rows)
        assertEquals(portrait.cellWidth to portrait.cellHeight, landscape.cellWidth to landscape.cellHeight)
        assertEquals(landscape.rows.toInt(), state.screen.rowCount)
        assertEquals("row 0 of ${landscape.rows}", state.screen.text(0))

        size = PORTRAIT
        compose.waitForIdle()
        assertEquals("back upright, the first grid again", portrait, pane.sizes.last())
        assertEquals(portrait.rows.toInt(), state.screen.rowCount)
    }

    @Test
    fun aTooSmallViewGivesNoGrid() {
        val pane = numberedPane()
        size = DpSize(4.dp, 4.dp)
        show(pane)
        assertEquals(emptyList<Any>(), pane.sizes)
        assertEquals(null, state.grid)
    }

    @Test
    fun framesAreAskedSinceTheOneHeldAndWholeAfterAnAttach() {
        val pane = FakePane()
        var seq = 4L
        pane.frames = { since ->
            seq++
            RowBufferWriter(10, 2, seq = seq, full = since == 0uL).text(0, "frame $seq").bytes()
        }
        show(pane)
        val first = pane.asked.first()
        assertEquals("the first read is whole", 0uL, first)
        val held = state.screen.seq

        pane.asked.clear()
        state.dirty()
        compose.waitForIdle()
        assertEquals("a dirty pane is read since the frame held", listOf(held.toULong()), pane.asked)

        pane.asked.clear()
        state.attached()
        compose.waitForIdle()
        assertEquals("a fresh terminal is read whole", listOf(0uL), pane.asked)
        assertEquals("frame $seq", state.screen.text(0))
    }

    @Test
    fun aPaneWithNoTerminalKeepsTheLastScreen() {
        val pane = numberedPane()
        show(pane)
        val frames = pane.frames
        pane.frames = null
        state.dirty()
        compose.waitForIdle()
        assertEquals("row 0 of ${pane.sizes.last().rows}", state.screen.text(0))
        compose.onNodeWithContentDescription("Terminal").assertExists()

        pane.frames = frames
        state.attached()
        compose.waitForIdle()
        assertEquals(0uL, pane.asked.last())
    }

    @Test
    fun aStaleFrameIsAskedForAgain() {
        val pane = FakePane()
        var stale = true
        pane.frames = { _ ->
            val writer =
                RowBufferWriter(
                    10,
                    2,
                    seq = if (stale) 1 else 2,
                    stale = stale,
                ).text(0, if (stale) "old" else "new")
            stale = false
            writer.bytes()
        }
        show(pane)
        compose.waitUntil(STALE_WAIT_MS) { state.screen.text(0) == "new" }
    }

    @Test
    fun theViewGivesThePaneItsPalette() {
        val pane = numberedPane()
        show(pane)
        assertEquals(listOf(CopperTerminal.toPaneColors()), pane.colors)
        palette = DraftingTerminal
        compose.waitForIdle()
        assertEquals(DraftingTerminal.toPaneColors(), pane.colors.last())
        val colors = pane.colors.last()
        assertEquals(TerminalPalette.ANSI_COLORS, colors.palette.size)
        assertEquals(0xFFF6F8F8u, colors.background)
        assertEquals(0xFF984A19u, colors.cursor)
    }

    private companion object {
        val PORTRAIT = DpSize(411.dp, 891.dp)
        const val STALE_WAIT_MS = 2_000L
    }
}
