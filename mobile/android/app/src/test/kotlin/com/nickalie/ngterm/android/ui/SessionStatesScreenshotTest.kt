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
import com.nickalie.ngterm.android.ui.sessions.RenameSessionDialog
import com.nickalie.ngterm.android.ui.sessions.RenameState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.CatalogState
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
 * failed refresh over the list, an agterm outside Live sessions mode, a headless machine's catalog with its ended and
 * page rows while its stream is down), a New session under way and the New session and Rename session dialogs, on a
 * phone in light and dark.
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
    private val duplicating = SampleSessions.copy(ending = emptySet(), duplicating = "s4", creating = true)
    private val stale = SampleSessions.copy(failure = "build-01 gave no answer (Connection reset by peer)")
    private val notLive =
        empty.copy(
            hint =
                "agterm on build-01 is not in Live sessions mode: set Settings > General > Restore sessions to Live " +
                    "sessions and restart agterm; only sessions started after that can be attached",
        )

    private val catalog =
        SampleSessions.copy(
            sessions =
                SampleSessions.sessions.orEmpty() +
                    listOf(
                        fakeSession("s6", name = "migrate-db").copy(cwd = "/srv/app", ended = true),
                        fakeSession("p1", name = "release-notes.md").copy(cwd = "", page = "file"),
                        fakeSession("p2", name = "grafana").copy(cwd = "", page = "url"),
                    ),
            ending = emptySet(),
            catalog = CatalogState.Disconnected("connection reset by peer"),
        )

    @Test
    @Config(qualifiers = PHONE)
    fun catalogLight() = capture("machine_sessions_catalog_light") { SessionsSample(catalog) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun catalogDark() = capture("machine_sessions_catalog_dark") { SessionsSample(catalog) }

    @Test
    @Config(qualifiers = PHONE)
    fun emptyLight() = capture("machine_sessions_empty_light") { SessionsSample(empty) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun emptyDark() = capture("machine_sessions_empty_dark") { SessionsSample(empty) }

    @Test
    @Config(qualifiers = PHONE)
    fun creatingLight() = capture("machine_sessions_creating_light") { SessionsSample(empty, starting = true) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun creatingDark() = capture("machine_sessions_creating_dark") { SessionsSample(empty, starting = true) }

    @Test
    @Config(qualifiers = PHONE)
    fun duplicatingLight() = capture("machine_sessions_duplicating_light") { SessionsSample(duplicating) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun duplicatingDark() = capture("machine_sessions_duplicating_dark") { SessionsSample(duplicating) }

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

    @Test
    @Config(qualifiers = PHONE)
    fun notLiveLight() = capture("machine_sessions_not_live_light") { SessionsSample(notLive) }

    @Test
    @Config(qualifiers = PHONE_NIGHT)
    fun notLiveDark() = capture("machine_sessions_not_live_dark") { SessionsSample(notLive) }

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

    @OptIn(ExperimentalRoborazziApi::class)
    @Test
    fun renameSessionDialog() {
        compose.setContent {
            NgtermTheme {
                Ground { SessionsSample(SampleSessions) }
                RenameSessionDialog(
                    state = RenameState("s3", "docs-site", failed = "build-01 did not run Rename docs-site"),
                    onRename = {},
                    onDismiss = {},
                )
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/screens/rename_session_failed.png")
    }
}
