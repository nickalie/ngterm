package com.nickalie.ngterm.android.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureRoboImage
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.ui.sessions.NewSessionDialog
import com.nickalie.ngterm.android.ui.sessions.NewSessionState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private const val PHONE = "w411dp-h891dp-xhdpi"
private const val PHONE_NIGHT = "w411dp-h891dp-night-xhdpi"

/**
 * A machine's sessions in its other states (empty, reading, not connected, offline, unreachable, ngtermctl missing, a
 * failed refresh over the list) and the New session dialog, on a phone in light and dark.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SessionStatesScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    private fun capture(
        name: String,
        content: @Composable () -> Unit,
    ) {
        compose.setContent { NgtermTheme { Ground { content() } } }
        compose.onRoot().captureRoboImage("src/test/screenshots/screens/$name.png")
    }

    private val empty = SampleSessions.copy(sessions = emptyList(), attached = emptySet(), ending = emptySet())
    private val missing = SampleSessions.copy(sessions = null, failure = "/bin/sh: 1: ngtermctl: not found")
    private val unreachable =
        SampleSessions.copy(link = Link.Unreachable("build-01.lan:22: connection refused"), sessions = null)
    private val stale = SampleSessions.copy(failure = "build-01 gave no answer (Connection reset by peer)")

    @Test
    @Config(qualifiers = PHONE)
    fun emptyLight() = capture("machine_sessions_empty_light") { SessionsSample(empty) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun emptyDark() = capture("machine_sessions_empty_dark") { SessionsSample(empty) }

    @Test
    @Config(qualifiers = PHONE)
    fun readingLight() =
        capture("machine_sessions_reading_light") { SessionsSample(SampleSessions.copy(sessions = null)) }

    @Test
    @Config(qualifiers = PHONE)
    fun notConnectedLight() =
        capture("machine_sessions_not_connected_light") { SessionsSample(SampleSessions.copy(link = Link.Offline)) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun offlineDark() =
        capture("machine_sessions_offline_dark") {
            SessionsSample(SampleSessions.copy(link = Link.Offline, offline = true))
        }

    @Test
    @Config(qualifiers = PHONE)
    fun unreachableLight() = capture("machine_sessions_unreachable_light") { SessionsSample(unreachable) }

    @Test
    @Config(qualifiers = PHONE)
    fun missingCliLight() = capture("machine_sessions_missing_cli_light") { SessionsSample(missing) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun missingCliDark() = capture("machine_sessions_missing_cli_dark") { SessionsSample(missing) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun staleDark() = capture("machine_sessions_stale_dark") { SessionsSample(stale) }

    // Robolectric's default window: under a phone's qualifiers its layout of a text field in a dialog never settles
    @OptIn(ExperimentalRoborazziApi::class)
    @Test
    fun newSessionDialog() {
        compose.setContent {
            NgtermTheme {
                Ground { SessionsSample(SampleSessions) }
                NewSessionDialog(
                    machine = "build-01",
                    state = NewSessionState(failed = "build-01 does not run live sessions (restore mode off)"),
                    onCreate = {},
                    onDismiss = {},
                )
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/screens/new_session_failed.png")
    }
}
