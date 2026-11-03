package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * The terminal's touch and mouse chrome on a phone (src/test/screenshots/terminal): a long press's selection with
 * its handles and toolbar, the right-click menu, and the unsafe-paste confirmation, dark and light, and the toolbar
 * at font scale 2.
 */
@OptIn(ExperimentalRoborazziApi::class)
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalInputScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private lateinit var state: TerminalState

    private fun show(dark: Boolean) {
        val pane = SelectingPane(SHELL, FakeCursor(SHELL.last().length, SHELL.lastIndex))
        compose.setContent {
            NgtermTheme(darkTheme = dark) {
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                this.state = state
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    onDispose { pane.onWake = null }
                }
                Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
                    TerminalView(state, Modifier.fillMaxWidth().height(TERMINAL_HEIGHT))
                }
            }
        }
        compose.waitForIdle()
    }

    private fun cell(
        column: Int,
        row: Int,
    ): Offset {
        val grid = requireNotNull(state.grid)
        return Offset((column + 0.5f) * grid.cellWidth.toFloat(), (row + 0.5f) * grid.cellHeight.toFloat())
    }

    private val terminal get() = compose.onNodeWithContentDescription("Terminal")

    private fun selection(
        name: String,
        dark: Boolean,
    ) {
        show(dark)
        // the first commit's hash
        terminal.performTouchInput { longClick(cell(2, 1)) }
        compose.waitForIdle()
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")
    }

    @Test
    fun selectionDark() = selection("touch_selection_dark", dark = true)

    @Test
    fun selectionLight() = selection("touch_selection_light", dark = false)

    @Test
    fun selectionFont2() {
        RuntimeEnvironment.setFontScale(2f)
        selection("touch_selection_font2", dark = true)
    }

    private fun menu(
        name: String,
        dark: Boolean,
    ) {
        show(dark)
        terminal.performMouseInput { rightClick(cell(12, 3)) }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/terminal/$name.png")
    }

    @Test
    fun menuDark() = menu("menu_dark", dark = true)

    @Test
    fun menuLight() = menu("menu_light", dark = false)

    private fun pasteConfirmation(
        name: String,
        dark: Boolean,
    ) {
        compose.setContent {
            NgtermTheme(darkTheme = dark) {
                Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background))
                PasteConfirmDialog(text = UNSAFE_PASTE, onPaste = {}, onDismiss = {})
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/terminal/$name.png")
    }

    @Test
    fun pasteConfirmationDark() = pasteConfirmation("paste_unsafe_dark", dark = true)

    @Test
    fun pasteConfirmationLight() = pasteConfirmation("paste_unsafe_light", dark = false)

    private companion object {
        val TERMINAL_HEIGHT = 360.dp
        val SHELL =
            listOf(
                "nick@build-01:~/api$ git log --oneline -3",
                "8570f95 fix: address code review findings",
                "d01bc86 feat: Android TerminalView rendering",
                "67f0ca4 fix: list again after a reconnect",
                "nick@build-01:~/api$ ",
            )
        const val UNSAFE_PASTE = "cd ~/api\ngit pull --rebase\ncargo test -p ngterm-mobile\n"
    }
}
