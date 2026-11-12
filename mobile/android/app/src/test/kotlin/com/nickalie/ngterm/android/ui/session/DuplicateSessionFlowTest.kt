package com.nickalie.ngterm.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.assertAll
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isEnabled
import androidx.compose.ui.test.isNotEnabled
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Duplicate session, from a machine's list or the session view's More, ends in the copy open and in control. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class DuplicateSessionFlowTest : SessionViewFixture() {
    private val copy = PaneRef("m1", "fake-1", "P1")

    private fun takeLeads() =
        test.core.calls
            .filter { it.method == "takeLead" }
            .map { it.args }

    private fun duplicates() =
        test.core.calls
            .filter { it.method == "duplicateSession" }
            .map { it.args }

    private fun openMachine() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        val content: @Composable () -> Unit = {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout.value) }
        }
        compose.setContent(content)
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory =
            { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }).also { panes[pane] = it } }
        compose.onNodeWithText("build-01").performClick()
        compose.awaitText("docs-site")
    }

    private fun duplicate() {
        compose.onNodeWithContentDescription("More options for docs-site").performClick()
        compose.onNodeWithText("Duplicate session").performClick()
    }

    // the copy's stream goes live and its pane attaches as a follower: this device takes the lead without a tap
    private fun landsInControl() {
        waitFor {
            "fake-1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        compose.awaitText("Opening session 1")
        test.core.emit(MobileEvent.SessionConnection("m1", "fake-1", StreamState.Connected))
        test.core.emit(MobileEvent.SessionLayout("m1", "fake-1", RemoteLayout(listOf("P1"), "P1", null, false)))
        compose.waitForIdle()
        waitFor { copy in test.core.shownPanes }
        test.core.emit(MobileEvent.PaneLead(copy, PaneRole.FOLLOWER, true))
        waitFor { takeLeads().isNotEmpty() }
        assertEquals(listOf(listOf("m1", "fake-1", "P1")), takeLeads())
        compose.waitForIdle()
        assertTrue(shows("Attaching"))
        assertTrue(!shows("Another device is driving this pane"))
    }

    @Test
    fun aMachinesListDuplicatesARowAndOpensTheCopyInControl() {
        openMachine()
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        compose.onNodeWithText("New session").assertIsNotEnabled()
        compose.onNodeWithContentDescription("More options for docs-site").performClick()
        compose.onNodeWithText("Duplicate session").assertIsNotEnabled()
        gate.complete(Unit)
        landsInControl()
        assertEquals(listOf(listOf("m1", "s1")), duplicates())
    }

    @Test
    fun theSessionViewsMoreDuplicatesTheOpenSessionAndOpensTheCopy() {
        openSession()
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        gate.complete(Unit)
        landsInControl()
        assertEquals(listOf(listOf("m1", "s1")), duplicates())
    }

    @Test
    fun aCopyFinishingAfterSessionsWasLeftWaitsInTheListWithoutTheLead() {
        openSession()
        layout.value = NavLayout.Rail
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        compose.onNode(hasText("Settings") and isSelectable()).performClick()
        compose.waitForIdle()
        gate.complete(Unit)
        waitFor {
            "fake-1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        compose.waitForIdle()
        assertEquals(emptySet<SessionKey>(), test.graph.panes.claims.held.value)
        compose.onNode(hasText("Sessions") and isSelectable()).performClick()
        compose.waitForIdle()
        assertTrue(described("Pane 2 of 2"))
        assertTrue(!shows("Opening session 1"))
    }

    @Test
    fun aDuplicateFromTheSessionViewHoldsTheMachinesScreen() {
        openSession()
        layout.value = NavLayout.Rail
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        compose.onNode(hasText("Machines") and isSelectable()).performClick()
        compose.onNodeWithText("build-01").performClick()
        compose.awaitText("Duplicating")
        compose.onNodeWithText("New session").assertIsNotEnabled()
        compose.onNodeWithContentDescription("More options for docs-site").performClick()
        compose.onNodeWithText("Duplicate session").assertIsNotEnabled()
        gate.complete(Unit)
        waitFor { !shows("Duplicating") }
        assertEquals(listOf(listOf("m1", "s1")), duplicates())
    }

    @Test
    fun aCopyFinishingInTheFrameSessionsIsLeftWaitsInTheListWithoutTheLead() {
        openSession()
        layout.value = NavLayout.Rail
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        compose.mainClock.autoAdvance = false
        compose.onNode(hasText("Settings") and isSelectable()).performClick()
        gate.complete(Unit)
        waitFor {
            "fake-1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        assertEquals(emptySet<SessionKey>(), test.graph.panes.claims.held.value)
        compose.mainClock.autoAdvance = true
        compose.onNode(hasText("Sessions") and isSelectable()).performClick()
        compose.waitForIdle()
        assertTrue(described("Pane 2 of 2"))
    }

    @Test
    fun aDuplicateHoldsNewSessionOnAMachineWithNoSessions() {
        test.seedMachine(testMachine(id = "m2", label = "build-02", host = "build-02.lan", keyId = test.seedKey().id))
        openSession()
        layout.value = NavLayout.Rail
        val gate = CompletableDeferred<Unit>().also { test.core.holds["duplicateSession"] = it }
        duplicate()
        compose.awaitText("Duplicating")
        compose.onNode(hasText("Machines") and isSelectable()).performClick()
        compose.onNodeWithText("build-02").performClick()
        compose.awaitText("No sessions on build-02")
        compose.onAllNodesWithText("New session").assertCountEquals(2).assertAll(isNotEnabled())
        gate.complete(Unit)
        waitFor { test.graph.creates.underWay.value == null }
        compose.onAllNodesWithText("New session").assertCountEquals(2).assertAll(isEnabled())
    }

    @Test
    fun aFailedDuplicateSaysWhyAndStaysPut() {
        openMachine()
        test.core.failNext("duplicateSession", MobileException.Remote("build-01 does not run live sessions"))
        duplicate()
        compose.awaitText("build-01 does not run live sessions")
        compose.onNodeWithText("docs-site").assertExists()
        assertTrue(
            test.connections.attached.value["m1"]
                .isNullOrEmpty(),
        )
    }
}
