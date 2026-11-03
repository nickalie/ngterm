package com.nickalie.ngterm.android.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.machines.MachinesUiState
import com.nickalie.ngterm.android.ui.onboarding.WelcomeScreen
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private const val PHONE = "w411dp-h891dp-xhdpi"
private const val PHONE_NIGHT = "w411dp-h891dp-night-xhdpi"

/** The Machines home's other states, and the first run's failure, on a phone in light and dark. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class MachineStatesScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private fun capture(
        name: String,
        content: @Composable () -> Unit,
    ) {
        compose.setContent { NgtermTheme { Ground { content() } } }
        compose.onRoot().captureRoboImage("src/test/screenshots/screens/$name.png")
    }

    private val offline = MachinesUiState.Ready(SampleMachines.take(2), offline = true)

    @Test
    @Config(qualifiers = PHONE)
    fun emptyLight() = capture("machines_empty_light") { MachinesSample(MachinesUiState.Ready(emptyList(), false)) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun emptyDark() = capture("machines_empty_dark") { MachinesSample(MachinesUiState.Ready(emptyList(), false)) }

    @Test
    @Config(qualifiers = PHONE)
    fun offlineLight() = capture("machines_offline_light") { MachinesSample(offline) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun offlineDark() = capture("machines_offline_dark") { MachinesSample(offline) }

    @Test
    @Config(qualifiers = PHONE)
    fun failedLight() =
        capture("machines_failed_light") { MachinesSample(MachinesUiState.Failed("the file is corrupt")) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun failedDark() = capture("machines_failed_dark") { MachinesSample(MachinesUiState.Failed("the file is corrupt")) }

    @Test
    @Config(qualifiers = PHONE)
    fun loadingLight() = capture("machines_loading_light") { MachinesSample(MachinesUiState.Loading) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun welcomeFailedDark() =
        capture("welcome_failed_dark") {
            WelcomeScreen(busy = false, failure = "the random source failed", onCreate = {}, onImport = {})
        }
}
