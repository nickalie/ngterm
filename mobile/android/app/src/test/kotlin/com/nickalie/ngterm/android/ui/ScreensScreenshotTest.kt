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
import com.nickalie.ngterm.android.data.ClipboardRead
import com.nickalie.ngterm.android.data.ClipboardSnapshot
import com.nickalie.ngterm.android.data.RED_PNG
import com.nickalie.ngterm.android.data.part
import com.nickalie.ngterm.android.ui.machines.HostKeyChangedScreen
import com.nickalie.ngterm.android.ui.machines.HostKeySheet
import com.nickalie.ngterm.android.ui.machines.MachineDraft
import com.nickalie.ngterm.android.ui.machines.MachineFormScreen
import com.nickalie.ngterm.android.ui.machines.MachinesUiState
import com.nickalie.ngterm.android.ui.machines.SignInSheet
import com.nickalie.ngterm.android.ui.onboarding.ImportKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.ImportProblem
import com.nickalie.ngterm.android.ui.onboarding.PublicKeyScreen
import com.nickalie.ngterm.android.ui.onboarding.WelcomeScreen
import com.nickalie.ngterm.android.ui.terminal.ClipboardReadSheet
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.TailscaleSignIn
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.PaneRef
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
internal class WindowRule(
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
 * Task 13's, 14's and 16's screens rendered by Robolectric and compared with their goldens
 * (src/test/screenshots/screens): the first run, the Machines home, a machine's sessions (and an agterm machine's card
 * and sessions, with its badge), the machine form, the host-key sheet, the sign-in sheet, the changed-key stop and a
 * program's clipboard read, each on a phone (light, dark, font scale 2.0), an unfolded foldable (light) and a
 * landscape tablet (light, dark). The keys shown are made up.
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
    fun machineSessions() = capture("machine_sessions") { SessionsSample(SampleSessions) }

    @Test
    fun agtermMachine() =
        capture(
            "machines_agterm",
        ) { MachinesSample(MachinesUiState.Ready(listOf(SampleAgtermMachine), offline = false)) }

    @Test
    fun agtermMachineSessions() = capture("machine_sessions_agterm") { SessionsSample(SampleAgtermSessions) }

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

    @OptIn(ExperimentalRoborazziApi::class)
    @Test
    fun signInSheet() {
        compose.setContent {
            NgtermTheme {
                MachinesSample(MachinesUiState.Ready(listOf(SampleSignIn), offline = false))
                SignInSheet(
                    machine = "studio",
                    address = "nick@studio.tail0f3c.ts.net:22",
                    message = TailscaleSignIn.message,
                    url = TailscaleSignIn.url,
                    onCancel = {},
                )
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage(golden("sign_in_sheet"))
    }

    @OptIn(ExperimentalRoborazziApi::class)
    @Test
    fun clipboardReadSheet() {
        compose.setContent {
            NgtermTheme {
                MachinesSample(MachinesUiState.Ready(SampleMachines.take(1), offline = false))
                ClipboardReadSheet(
                    read =
                        ClipboardRead(
                            PaneRef("m1", "s1", "P1"),
                            readId = 1u,
                            listOnly = false,
                            mimes = listOf("text/plain", "text/html", "image/png"),
                            canRemember = true,
                        ),
                    machine = "build-01",
                    session = "docs-site",
                    preview =
                        ClipboardSnapshot(
                            listOf(
                                part("text/plain", "git push origin feature/clipboard\nnpm run deploy -- --prod"),
                                part("text/html", "<b>deploy</b>"),
                                ClipboardPart("image/png", RED_PNG),
                            ),
                            emptyList(),
                        ),
                    onAllow = {},
                    onAlways = {},
                    onDeny = {},
                )
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage(golden("clipboard_read_sheet"))
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
