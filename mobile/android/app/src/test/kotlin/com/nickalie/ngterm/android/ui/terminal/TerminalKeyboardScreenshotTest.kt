package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import com.github.takahirom.roborazzi.captureRoboImage
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
 * The keyboard's chrome (src/test/screenshots/terminal): the extra-keys row under a terminal on a phone, with Ctrl
 * held for the next key, Alt locked and a dead key's accent waiting in the strip, dark and light; the row at font
 * scale 2; and the row in one line on a phone held sideways.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class TerminalKeyboardScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private val keyboard =
        TerminalKeyboard().apply {
            toggle(StickyModifier.Ctrl)
            toggle(StickyModifier.Alt)
            toggle(StickyModifier.Alt)
        }

    private fun typing(
        name: String,
        dark: Boolean,
    ) {
        val pane = SelectingPane(SHELL, FakeCursor(SHELL.last().length, SHELL.lastIndex))
        compose.setContent {
            NgtermTheme(darkTheme = dark) {
                val state = remember { TerminalState(pane, EmptyCoroutineContext) }
                DisposableEffect(Unit) {
                    pane.onWake = state::dirty
                    keyboard.target = state
                    state.composing = ACCENT
                    onDispose { pane.onWake = null }
                }
                Column(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
                    Box(Modifier.fillMaxWidth().height(TERMINAL_HEIGHT)) {
                        TerminalView(state, Modifier.fillMaxSize(), keyboard = keyboard)
                        CompositionStrip(keyboard, Modifier.align(Alignment.BottomEnd).padding(8.dp))
                    }
                    ExtraKeysRow(keyboard)
                }
            }
        }
        compose.waitForIdle()
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")
    }

    @Test
    fun typingDark() = typing("keys_typing_dark", dark = true)

    @Test
    fun typingLight() = typing("keys_typing_light", dark = false)

    private fun row(name: String) {
        compose.setContent {
            NgtermTheme(darkTheme = true) {
                Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background)) {
                    ExtraKeysRow(keyboard, Modifier.align(Alignment.BottomCenter))
                }
            }
        }
        compose.waitForIdle()
        compose.onRoot().captureRoboImage("src/test/screenshots/terminal/$name.png")
    }

    @Test
    fun rowFont2() {
        RuntimeEnvironment.setFontScale(2f)
        row("keys_font2")
    }

    @Test
    @Config(qualifiers = "w891dp-h411dp-xhdpi")
    fun rowLandscape() = row("keys_landscape")

    private companion object {
        val TERMINAL_HEIGHT = 240.dp
        const val ACCENT = "´"
        val SHELL =
            listOf(
                "nick@build-01:~/api$ git status --short",
                " M crates/ngterm-mobile/src/api/pane.rs",
                "?? mobile/android/DESIGN.md",
                "nick@build-01:~/api$ echo ",
            )
    }
}
