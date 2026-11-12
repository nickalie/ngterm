package com.nickalie.ngterm.android.ui.session

import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.terminal.TerminalKeyboard
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.coroutines.EmptyCoroutineContext

/**
 * Held text in the session view shows off the terminal: in the extra keys' top line, or in the bar's title while
 * Settings hides the row, and the terminal keeps its place and grid either way.
 */
@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "w411dp-h891dp")
class SessionHeldTextTest {
    @get:Rule
    val compose = createComposeRule()

    private val keyboard = TerminalKeyboard()
    private val extraKeys = mutableStateOf(ExtraKeysChoice.Hidden)
    private val state = TerminalState(SelectingPane(), EmptyCoroutineContext)

    private fun show() {
        val key = SessionKey("m1", "s1")
        val live = LiveSession(StreamState.Connected, null, null, RemoteLayout(listOf("P1"), "P1", null, false), null)
        val session = AttachedSession(key, "build-01", "docs-site", null, null, Link.Connected, live)
        compose.setContent {
            NgtermTheme {
                SessionScreen(
                    session = session,
                    focused = "P1",
                    panes = mapOf(PaneRef("m1", "s1", "P1") to state),
                    failures = emptyMap(),
                    compact = true,
                    actions = NoActions,
                    keyboard = keyboard,
                    extraKeys = extraKeys.value,
                )
            }
        }
        compose.waitForIdle()
    }

    private fun hold(text: String) {
        state.composing = text
        compose.waitForIdle()
    }

    private fun terminal(): Rect = compose.onNodeWithContentDescription("Terminal").fetchSemanticsNode().boundsInRoot

    private fun strip(): Rect =
        compose.onNodeWithContentDescription("$WAITING $HINDI").fetchSemanticsNode().boundsInRoot

    @Test
    fun withNoRowTheBarsTitleHoldsIt() {
        show()
        val view = terminal()
        val grid = state.grid
        hold(HINDI)
        assertTrue("the strip is over the terminal, in the bar: ${strip()}, $view", strip().bottom <= view.top)
        compose.onNodeWithText("docs-site").assertDoesNotExist()
        assertEquals("the terminal keeps its place", view, terminal())
        assertEquals("and its grid", grid, state.grid)
        hold("")
        compose.onNodeWithText("docs-site").assertExists()
        assertEquals(view, terminal())
    }

    @Test
    fun withTheRowItsTopLineHoldsIt() {
        extraKeys.value = ExtraKeysChoice.Fit
        show()
        val view = terminal()
        hold(HINDI)
        assertTrue("the strip is under the terminal: ${strip()}, $view", strip().top >= view.bottom)
        compose.onNodeWithText("docs-site").assertExists()
        assertEquals("the terminal keeps its place", view, terminal())
    }

    private companion object {
        const val HINDI = "नमस्ते"
        const val WAITING = "Waiting to be typed:"
        val NoActions =
            SessionViewActions(
                onBack = {},
                onFocus = { _, _ -> },
                onShow = { _, _ -> },
                onRetry = { _, _ -> },
                onTakeLead = {},
                onDetach = {},
                onEnd = {},
                onNextSession = {},
                onPreviousSession = {},
            )
    }
}
