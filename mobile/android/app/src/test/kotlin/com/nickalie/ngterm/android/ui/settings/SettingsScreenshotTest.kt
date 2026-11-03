package com.nickalie.ngterm.android.ui.settings

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performScrollTo
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.ui.InScaffold
import com.nickalie.ngterm.android.ui.SampleImportedKey
import com.nickalie.ngterm.android.ui.SampleKey
import com.nickalie.ngterm.android.ui.ScreensScreenshotTest
import com.nickalie.ngterm.android.ui.Window
import com.nickalie.ngterm.android.ui.WindowRule
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.GraphicsMode

/** Settings in the sample's state: two keys (one imported), a machine trusted with the clipboard, Done off. */
private val SampleSettings =
    SettingsUiState.Ready(
        settings = AppSettings(textSize = 14, palette = PaletteChoice.Theme, keepScreenOn = true),
        alerts = AlertKind.entries.toSet() - AlertKind.Done,
        keys =
            listOf(
                KeyItem(SampleKey, listOf("build-01", "nas"), everyMachine = false),
                KeyItem(SampleImportedKey, listOf("studio"), everyMachine = false),
            ),
        clipboard = listOf(TrustedMachine("m1", "build-01", "nick@build-01.lan:22")),
    )

private val NoSettingsActions = SettingsActions({}, { _, _ -> }, {}, {}, {}, {}, {}, {})

/**
 * Task 20's Settings rendered by Robolectric and compared with their goldens (src/test/screenshots/settings): its top
 * (the terminal's sample and controls), its foot (notifications turned off in the system, the clipboard, the keys and
 * the core's version) and a rotated key's public half with the switch to it, in the windows of
 * [ScreensScreenshotTest]. The keys shown are made up.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SettingsScreenshotTest(
    private val window: Window,
) {
    private val compose = createComposeRule()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(WindowRule(window)).around(compose)

    private fun golden(screen: String) = "src/test/screenshots/settings/${screen}_$window.png"

    private fun show(content: @Composable () -> Unit) {
        compose.setContent { NgtermTheme { InScaffold { content() } } }
    }

    private fun settings() =
        show {
            SettingsScreen(
                state = SampleSettings,
                coreVersion = "0.1.0",
                notificationsOn = false,
                keyWork = KeyWork(),
                actions = NoSettingsActions,
                wallpaperColors = true,
            )
        }

    @Test
    fun top() {
        settings()
        compose.onRoot().captureRoboImage(golden("settings"))
    }

    @Test
    fun foot() {
        settings()
        compose.onNodeWithText("core 0.1.0").performScrollTo()
        compose.onRoot().captureRoboImage(golden("settings_keys"))
    }

    @Test
    fun newKey() {
        show {
            PublicKeyScreen(
                key = SampleImportedKey,
                exit = ScreenExit(R.drawable.ic_back, "Back") {},
                footer = { SwitchMachines(KeyItem(SampleImportedKey, emptyList(), everyMachine = false), onUse = {}) },
            )
        }
        compose.onNodeWithText("Use for every machine").performScrollTo()
        compose.onRoot().captureRoboImage(golden("new_key"))
    }

    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun windows() = ScreensScreenshotTest.windows()
    }
}
