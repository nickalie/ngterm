package com.nickalie.ngterm.android.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.nickalie.ngterm.android.ENCRYPTED_FAKE_KEY
import com.nickalie.ngterm.android.ui.machines.HostKeyChangedScreen
import com.nickalie.ngterm.android.ui.machines.HostKeySheet
import com.nickalie.ngterm.android.ui.machines.MachineDraft
import com.nickalie.ngterm.android.ui.machines.MachineFormScreen
import com.nickalie.ngterm.android.ui.machines.MachinesUiState
import com.nickalie.ngterm.android.ui.onboarding.ImportKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.ImportProblem
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.WelcomeScreen
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.GraphicsMode

/** A window the screens are rendered in: its golden suffix, Robolectric's qualifiers, and the font scale. */
data class Window(
    val name: String,
    val qualifiers: String,
    val fontScale: Float = 1f,
) {
    override fun toString() = name
}

/** Sets the window before the compose rule starts its activity. */
private class WindowRule(
    private val window: Window,
) : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ) = object : Statement() {
        override fun evaluate() {
            RuntimeEnvironment.setQualifiers(window.qualifiers)
            RuntimeEnvironment.setFontScale(window.fontScale)
            base.evaluate()
        }
    }
}

/**
 * Task 13's screens rendered by Robolectric and compared with their goldens (src/test/screenshots/screens): the first
 * run, the Machines home, the machine form, the host-key sheet and the changed-key stop, each on a phone (light, dark,
 * font scale 2.0), an unfolded foldable (light) and a landscape tablet (light, dark). The keys shown are made up.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ScreensScreenshotTest(
    private val window: Window,
) {
    private val compose = createComposeRule()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(WindowRule(window)).around(compose)

    private fun capture(
        screen: String,
        content: @Composable () -> Unit,
    ) {
        compose.setContent { NgtermTheme { Ground { content() } } }
        compose.onRoot().captureRoboImage(golden(screen))
    }

    private fun golden(screen: String) = "src/test/screenshots/screens/${screen}_$window.png"

    @Test
    fun welcome() = capture("welcome") { WelcomeScreen(busy = false, failure = null, onCreate = {}, onImport = {}) }

    @Test
    fun publicKey() = capture("public_key") { PublicKeyScreen(SampleKey, onDone = {}) }

    @Test
    fun importKey() =
        capture("import_key") {
            ImportKeyScreen(
                busy = false,
                problem = ImportProblem.WrongPassphrase,
                onImport = { _, _ -> },
                onUnreadable = {},
                onBack = {},
                initialKey = ENCRYPTED_FAKE_KEY,
            )
        }

    @Test
    fun machines() = capture("machines") { MachinesSample(MachinesUiState.Ready(SampleMachines, offline = false)) }

    @Test
    fun machineForm() {
        compose.setContent {
            NgtermTheme {
                InScaffold {
                    MachineFormScreen(
                        initial = MachineDraft(label = "build-01", host = "nick@build-01.lan", user = ""),
                        editing = false,
                        keys = listOf(SampleKey, SampleImportedKey),
                        pinned = null,
                        onSave = {},
                        onClose = {},
                        onShowKey = {},
                    )
                }
            }
        }
        // Save shows what is wrong
        compose.onNodeWithText("Save").performClick()
        compose.onRoot().captureRoboImage(golden("machine_form"))
    }

    @OptIn(ExperimentalRoborazziApi::class)
    @Test
    fun hostKeySheet() {
        compose.setContent {
            NgtermTheme {
                MachinesSample(MachinesUiState.Ready(SampleMachines.take(1), offline = false))
                HostKeySheet(
                    "build-01",
                    "nick@build-01.lan:22",
                    "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E",
                    onAnswer = {},
                )
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage(golden("host_key_sheet"))
    }

    @Test
    fun hostKeyChanged() =
        capture("host_key_changed") {
            InScaffold {
                HostKeyChangedScreen(
                    machine = "nas",
                    pinned = "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E",
                    offered = "SHA256:Zb3x9QeTf0LmN4kR7sVw2uYcHj6pDa1gEoI8nXy5Kq0",
                    onBack = {},
                    onRemovePin = {},
                )
            }
        }

    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun windows() =
            listOf(
                Window("compact_light", "w411dp-h891dp-xhdpi"),
                Window("compact_dark", "w411dp-h891dp-night-xhdpi"),
                Window("compact_font2", "w411dp-h891dp-xhdpi", fontScale = 2f),
                Window("medium_light", "w700dp-h1000dp-hdpi"),
                Window("expanded_light", "w1280dp-h800dp-land-mdpi"),
                Window("expanded_dark", "w1280dp-h800dp-land-night-mdpi"),
            ).map { arrayOf<Any>(it) }
    }
}
