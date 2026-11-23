package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTouchInput
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.PaneColorScheme
import com.nickalie.ngterm.core.ffi.PaneMouseAction
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteSession
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * A tap on a session link opens the session it names in place of the tap's own work, and any other tap does that
 * work; the terminal reports the theme's light or dark side to programs, and each flip of it.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalSessionLinkTest {
    @get:Rule
    val compose = createComposeRule()

    private val other = fakeSession("00000000-0000-0000-0000-0000000000A4", name = "other")

    // row 0 links a listed session, row 1 a link the machine lists no session for, row 2 is plain text
    private val pane =
        FakePane().apply {
            frames = { _ ->
                val grid = sizes.lastOrNull() ?: PaneSize(COLUMNS.toUShort(), ROWS.toUShort(), 1u, 1u)
                RowBufferWriter(grid.cols.toInt(), grid.rows.toInt())
                    .row(0, FakeRun(0, "other", attributes = FakeRun.HYPERLINK))
                    .row(1, FakeRun(0, "gone", attributes = FakeRun.HYPERLINK))
                    .text(2, "plain")
                    .bytes()
            }
            links = mapOf(CellPoint(2u, 0u) to other)
        }

    private lateinit var state: TerminalState
    private val opened = mutableListOf<RemoteSession>()
    private var dark by mutableStateOf(true)

    private val terminal get() = compose.onNodeWithContentDescription("Terminal")

    private fun show(links: Boolean = true) {
        compose.setContent {
            NgtermTheme(darkTheme = dark) {
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                this.state = state
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    onDispose { pane.onWake = null }
                }
                TerminalView(state, Modifier.fillMaxSize(), onSessionLink = if (links) ({ opened += it }) else null)
            }
        }
        compose.waitForIdle()
    }

    private fun tap(
        column: Int,
        row: Int,
    ) {
        val grid = requireNotNull(state.grid)
        terminal.performTouchInput {
            click(Offset((column + 0.5f) * grid.cellWidth.toFloat(), (row + 0.5f) * grid.cellHeight.toFloat()))
        }
        compose.waitForIdle()
    }

    private fun clicks() = pane.inputs.filterIsInstance<FakeInput.Mouse>().map { it.mouse.action }

    @Test
    fun aTapOnASessionLinkOpensItsSessionAndClicksNothing() {
        pane.tracking = true
        show()
        tap(2, 0)
        assertEquals(listOf(other), opened)
        assertEquals("the program hears no click", emptyList<PaneMouseAction>(), clicks())
    }

    @Test
    fun aTapOnAnyOtherCellDoesTheTapsOwnWork() {
        pane.tracking = true
        show()
        tap(1, 1)
        tap(1, 2)
        assertEquals("a link naming no listed session and plain text", emptyList<RemoteSession>(), opened)
        val click = listOf(PaneMouseAction.PRESS, PaneMouseAction.RELEASE)
        assertEquals(click + click, clicks())
    }

    @Test
    fun aViewWithNowhereToOpenALinkClicksOnIt() {
        pane.tracking = true
        show(links = false)
        tap(2, 0)
        assertEquals(listOf(PaneMouseAction.PRESS, PaneMouseAction.RELEASE), clicks())
    }

    @Test
    fun theTerminalReportsTheThemesSideAndEachFlip() {
        show()
        assertEquals(listOf(PaneColorScheme.DARK), pane.colorSchemes)
        dark = false
        compose.waitForIdle()
        assertEquals(listOf(PaneColorScheme.DARK, PaneColorScheme.LIGHT), pane.colorSchemes)
    }

    private companion object {
        const val COLUMNS = 40
        const val ROWS = 10
    }
}
