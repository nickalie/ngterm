package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.theme.GhosttyTerminal
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.RowBufferWriter
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/**
 * The terminal drawing known buffers (TerminalScenes) on a phone: colors, attributes, wide characters, the cursor
 * shapes and the selection, in the theme's palettes (Copper dark, Drafting light) and Ghostty's, and at font scale 2.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private fun capture(
        name: String,
        dark: Boolean,
        palette: TerminalPalette? = null,
        content: @Composable (TerminalPalette) -> Unit,
    ) {
        compose.setContent { NgtermTheme(darkTheme = dark) { content(palette ?: NgtermTheme.terminal) } }
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")
    }

    private fun scene(
        name: String,
        dark: Boolean,
        palette: TerminalPalette? = null,
        height: Dp = SCENE_HEIGHT,
        scene: (TerminalScenes.Scene) -> RowBufferWriter,
    ) = capture(name, dark, palette) { Terminal(TerminalScenes.pane(scene), it, Modifier.height(height)) }

    @Test
    fun colorsDark() = scene("colors_dark", dark = true, scene = TerminalScenes::colors)

    @Test
    fun colorsLight() = scene("colors_light", dark = false, scene = TerminalScenes::colors)

    @Test
    fun colorsGhostty() =
        scene("colors_ghostty", dark = true, palette = GhosttyTerminal, scene = TerminalScenes::colors)

    @Test
    fun attributesDark() = scene("attributes_dark", dark = true, scene = TerminalScenes::attributes)

    @Test
    fun attributesLight() = scene("attributes_light", dark = false, scene = TerminalScenes::attributes)

    @Test
    fun attributesFont2() {
        RuntimeEnvironment.setFontScale(2f)
        scene("attributes_font2", dark = true, height = SCENE_HEIGHT * 2, scene = TerminalScenes::attributes)
    }

    @Test
    fun wideDark() = scene("wide_dark", dark = true, scene = TerminalScenes::wide)

    @Test
    fun wideLight() = scene("wide_light", dark = false, scene = TerminalScenes::wide)

    @Test
    fun selectionDark() = scene("selection_dark", dark = true, height = SHORT_HEIGHT, scene = TerminalScenes::selection)

    @Test
    fun selectionLight() =
        scene("selection_light", dark = false, height = SHORT_HEIGHT, scene = TerminalScenes::selection)

    @Test
    fun cursorsDark() = capture("cursors_dark", dark = true) { Cursors(it) }

    @Test
    fun cursorsLight() = capture("cursors_light", dark = false) { Cursors(it) }

    /** The four shapes, the cursor on a wide character's tail, and a terminal out of focus (a hollow block). */
    @Composable
    private fun Cursors(palette: TerminalPalette) {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            for (style in 0..3) {
                Terminal(
                    TerminalScenes.pane(TerminalScenes.cursor(style)),
                    palette,
                    Modifier.height(LINE),
                )
            }
            Terminal(TerminalScenes.pane(TerminalScenes::cursorOnWide), palette, Modifier.height(LINE))
            Terminal(TerminalScenes.pane(TerminalScenes.cursor(0)), palette, Modifier.height(LINE), focused = false)
        }
    }

    @Composable
    private fun Terminal(
        pane: FakePane,
        palette: TerminalPalette,
        modifier: Modifier = Modifier,
        focused: Boolean = true,
    ) {
        val state = remember(pane) { TerminalState(pane, EmptyCoroutineContext) }
        DisposableEffect(pane) {
            pane.onWake = state::dirty
            onDispose { pane.onWake = null }
        }
        TerminalView(state, modifier.fillMaxWidth(), palette = palette, focused = focused)
    }

    private companion object {
        val SCENE_HEIGHT = 230.dp
        val SHORT_HEIGHT = 120.dp
        val LINE = 20.dp
    }
}
