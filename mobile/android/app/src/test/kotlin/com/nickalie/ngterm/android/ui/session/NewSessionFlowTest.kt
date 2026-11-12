package com.nickalie.ngterm.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** One New session tap ends in the new session attached, open and in control. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class NewSessionFlowTest : SessionViewFixture() {
    private val created = PaneRef("m1", "fake-1", "P1")

    private fun takeLeads() =
        test.core.calls
            .filter { it.method == "takeLead" }
            .map { it.args }

    private fun openMachine() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        val content: @Composable () -> Unit = {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout.value) }
        }
        compose.setContent(content)
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.paneFactory =
            { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }).also { panes[pane] = it } }
        compose.onNodeWithText("build-01").performClick()
        compose.awaitText("No sessions on build-01")
    }

    private fun newSession() {
        compose.onAllNodesWithText("New session")[0].performClick()
        waitFor {
            "fake-1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        compose.awaitText("Opening session 1")
        test.core.emit(MobileEvent.SessionConnection("m1", "fake-1", StreamState.Connected))
        test.core.emit(MobileEvent.SessionLayout("m1", "fake-1", RemoteLayout(listOf("P1"), "P1", null, false)))
        compose.waitForIdle()
        waitFor { created in test.core.shownPanes }
    }

    @Test
    fun oneTapCreatesAttachesOpensAndTakesControlOfTheNewSession() {
        openMachine()
        newSession()
        test.core.emit(MobileEvent.PaneLead(created, PaneRole.FOLLOWER, true))
        waitFor { takeLeads().isNotEmpty() }
        assertEquals(listOf(listOf("m1", "fake-1", "P1")), takeLeads())
        test.core.emit(MobileEvent.PaneLead(created, PaneRole.LEADER, false))
        waitFor { described("Terminal") }
        assertTrue(!shows("Another device is driving this pane"))
        assertEquals(
            listOf(null),
            test.core.calls
                .filter { it.method == "createSession" }
                .map { it.args[1] },
        )
        // a later takeover is the user's to answer
        test.core.emit(MobileEvent.PaneLead(created, PaneRole.FOLLOWER, true))
        waitFor { shows("Another device is driving this pane") }
        assertEquals(1, takeLeads().size)
    }

    @Test
    fun aNewSessionThatLeadsAlreadyAttachesNoSecondTime() {
        openMachine()
        newSession()
        test.core.emit(MobileEvent.PaneLead(created, PaneRole.LEADER, false))
        waitFor { described("Terminal") }
        compose.waitForIdle()
        assertTrue(takeLeads().isEmpty())
    }

    @Test
    fun aFailedCreateSaysWhyAndStaysOnTheList() {
        openMachine()
        test.core.failNext("createSession", MobileException.Remote("build-01 does not run live sessions"))
        compose.onAllNodesWithText("New session")[0].performClick()
        compose.awaitText("build-01 does not run live sessions")
        compose.onNodeWithText("No sessions on build-01").assertExists()
        assertTrue(
            test.connections.attached.value["m1"]
                .isNullOrEmpty(),
        )
    }

    @Test
    fun aFailedAttachSaysWhyAndLeavesTheNewSessionListed() {
        openMachine()
        test.core.failNext("attachSession", MobileException.Remote("the session's daemon is not running"))
        compose.onAllNodesWithText("New session")[0].performClick()
        compose.awaitText("the session's daemon is not running")
        compose.onNodeWithText("session 1").assertExists()
        assertTrue(
            test.connections.attached.value["m1"]
                .isNullOrEmpty(),
        )
    }

    @Test
    fun aSessionOpenedFromTheListStillAsksBeforeTakingControl() {
        openSession()
        test.core.emit(MobileEvent.PaneLead(p1, PaneRole.FOLLOWER, true))
        waitFor { shows("Another device is driving this pane") }
        compose.waitForIdle()
        assertTrue(takeLeads().isEmpty())
    }
}
