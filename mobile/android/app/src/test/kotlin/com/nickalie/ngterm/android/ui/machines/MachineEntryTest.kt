package com.nickalie.ngterm.android.ui.machines

import android.view.MotionEvent
import android.view.inspector.WindowInspector
import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

/** A machine card's actions, summaries and reconnect countdown. */
@RunWith(RobolectricTestRunner::class)
class MachineEntryTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val item = mutableStateOf(MachineItem(testMachine(), Link.Offline, null))
    private val calls = mutableListOf<String>()
    private val actions =
        MachineActions(
            onAdd = {},
            onOpen = { calls += "open:$it" },
            onEdit = { calls += "edit:$it" },
            onConnect = { calls += "connect:$it" },
            onDisconnect = { calls += "disconnect:$it" },
            onRemove = { calls += "remove:$it" },
            onReviewHostKey = { calls += "review:$it" },
            onRetryRead = {},
        )

    private fun card(now: () -> Long = { 0L }) {
        compose.setContent { NgtermTheme { MachineEntry(item.value, actions, now) } }
    }

    private fun menu(action: String) {
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNode(hasText(action) and hasAnyAncestor(isPopup())).performClick()
    }

    @Test
    fun menuConnectsEveryIdleOrFailedStateAndDisconnectsEveryPendingOrConnectedState() {
        card()
        val idle =
            listOf(
                Link.Offline,
                Link.Unreachable("Connection refused"),
                Link.HostKeyUnknown("SHA256:new"),
                Link.HostKeyChanged("SHA256:old", "SHA256:new"),
            )
        idle.forEach { link ->
            compose.runOnIdle { item.value = item.value.copy(link = link) }
            menu("Connect")
            assertEquals("connect:m1", calls.last())
        }
        val links =
            listOf(Link.Connected, Link.Connecting, Link.Reconnecting(4000), Link.SignIn("sign in", "https://host"))
        links.forEach { link ->
            compose.runOnIdle { item.value = item.value.copy(link = link) }
            menu("Disconnect")
            assertEquals("disconnect:m1", calls.last())
        }
        assertEquals(idle.size + links.size, calls.size)
        menu("Edit")
        menu("Remove")
        compose.onNodeWithText("build-01").performClick()
        assertEquals(listOf("edit:m1", "remove:m1", "open:m1"), calls.takeLast(3))
    }

    @Test
    fun dismissingTheMenuDoesNotPerformAnAction() {
        card()
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        // Robolectric does not route touches between windows: deliver Android's outside event to the popup.
        compose.runOnIdle {
            val outside = MotionEvent.obtain(0L, 0L, MotionEvent.ACTION_OUTSIDE, 0f, 0f, 0)
            try {
                WindowInspector.getGlobalWindowViews().last().dispatchTouchEvent(outside)
            } finally {
                outside.recycle()
            }
        }
        compose.onNodeWithText("Remove").assertDoesNotExist()
        assertEquals(emptyList<String>(), calls)
    }

    @Test
    fun failuresAndRefusalsOfferTheirSpecificRecoveryAction() {
        card()
        compose.onNodeWithText("Not connected").assertExists()
        compose.onNodeWithText("Connect").performClick()
        compose.runOnIdle { item.value = item.value.copy(link = Link.Unreachable("Connection refused")) }
        compose.onNodeWithText("Connection refused").assertExists()
        compose.onNodeWithText("Retry").performClick()
        compose.runOnIdle { item.value = item.value.copy(link = Link.HostKeyUnknown("SHA256:new")) }
        compose.onNodeWithText("Confirm its host key to connect").assertExists()
        compose.runOnIdle { item.value = item.value.copy(link = Link.HostKeyChanged("SHA256:old", "SHA256:new")) }
        compose.onNodeWithText("Not connecting until you review it").assertExists()
        compose.onNodeWithText("Review").performClick()
        assertEquals(listOf("connect:m1", "connect:m1", "review:m1"), calls)
    }

    @Test
    fun countdownRoundsUpExpiresAndCanBeRetriedImmediately() {
        var now = 0L
        item.value = item.value.copy(link = Link.Reconnecting(1500))
        card { now }
        compose.onNodeWithContentDescription("Reconnecting in 2 s").assertExists()
        compose.mainClock.autoAdvance = false
        now = 1000L
        compose.mainClock.advanceTimeBy(1100L)
        compose.onNodeWithContentDescription("Reconnecting in 1 s").assertExists()
        now = 2000L
        compose.mainClock.advanceTimeBy(1100L)
        compose.onNodeWithContentDescription("Reconnecting").assertExists()
        compose.mainClock.autoAdvance = true
        compose.onNodeWithText("Retry now").performClick()
        assertEquals(listOf("connect:m1"), calls)
    }

    @Test
    fun connectedSummariesDistinguishUnknownEmptyIdleAndAgentStatuses() {
        item.value = item.value.copy(link = Link.Connected)
        card()
        compose.onNodeWithText("No sessions").assertDoesNotExist()
        compose.runOnIdle { item.value = item.value.copy(sessions = emptyList()) }
        compose.onNodeWithText("No sessions").assertExists()
        compose.runOnIdle {
            item.value =
                item.value.copy(
                    sessions =
                        listOf(
                            fakeSession("a").copy(status = "active"),
                            fakeSession("b").copy(status = "active"),
                        ),
                )
        }
        compose.onNodeWithContentDescription("2 working").assertExists()
        compose.runOnIdle {
            item.value = item.value.copy(sessions = listOf(fakeSession("a"), fakeSession("b").copy(status = " ")))
        }
        compose.onNodeWithContentDescription("2 idle").assertExists()
    }
}
