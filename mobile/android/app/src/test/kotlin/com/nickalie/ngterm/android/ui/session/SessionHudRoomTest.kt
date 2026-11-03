package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.RemoteHud
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The HUD steps aside in a short window (a phone on its side with the soft keyboard up) and comes back with room. */
@RunWith(AndroidJUnit4::class)
class SessionHudRoomTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theHudShowsWithRoomAndStepsAsideWithout() {
        val height = mutableStateOf(480.dp)
        val hud = RemoteHud("Deploying", null, null, null, null, null, "top", false, null, false, true, null, 1, null)
        val live = LiveSession(StreamState.Connected, null, null, RemoteLayout(listOf("P1"), "P1", null, false), hud)
        val key = SessionKey("m1", "s1")
        val session = AttachedSession(key, "build-01", "docs-site", null, null, Link.Connected, live)
        compose.setContent {
            NgtermTheme {
                Box(Modifier.fillMaxWidth().height(height.value)) {
                    SessionScreen(
                        session = session,
                        focused = "P1",
                        panes = emptyMap(),
                        failures = emptyMap(),
                        compact = true,
                        actions = NoActions,
                    )
                }
            }
        }
        compose.onNodeWithText("Deploying").assertExists()
        height.value = 240.dp
        compose.waitForIdle()
        compose.onNodeWithText("Deploying").assertDoesNotExist()
        height.value = 320.dp
        compose.waitForIdle()
        compose.onNodeWithText("Deploying").assertExists()
    }

    private companion object {
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
