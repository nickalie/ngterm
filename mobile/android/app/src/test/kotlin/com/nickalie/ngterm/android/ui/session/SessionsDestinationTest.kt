package com.nickalie.ngterm.android.ui.session

import android.content.res.Configuration
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.withKeysDown
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Session navigation, confirmation, failure recovery and saved state over the real app. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class SessionsDestinationTest : SessionViewFixture() {
    private fun openSplit() {
        openSession()
        layout.value = NavLayout.Sidebar
        waitFor { panes["P1"] != null && panes["P2"] != null }
        compose.waitForIdle()
    }

    private fun terminal(index: Int) = compose.onAllNodesWithContentDescription("Terminal")[index]

    private fun terminals() = compose.onAllNodesWithContentDescription("Terminal").fetchSemanticsNodes().size

    private fun pressed(pane: String) =
        panes
            .getValue(pane)
            .inputs
            .filterIsInstance<FakeInput.Key>()
            .filter { it.key.action != PaneKeyAction.RELEASE }
            .map { it.key.key }

    private fun chord(arrow: Key) {
        compose.onRoot().performKeyInput {
            withKeysDown(listOf(Key.CtrlLeft, Key.ShiftLeft)) { pressKey(arrow) }
        }
        compose.waitForIdle()
    }

    private fun menu() = compose.onNodeWithContentDescription("More options for docs-site").performClick()

    private fun endConfirmed() = compose.onNode(hasText("End session") and hasAnyAncestor(isDialog())).performClick()

    @Test
    fun anOriginThatCannotStreamExplainsTheUpgradeAndOffersDetach() {
        openSession(withLayout = false)
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Unsupported))
        compose.awaitText("build-01's ngterm can't share live sessions")
        compose.onNodeWithContentDescription("Can't stream").assertExists()
        compose
            .onNodeWithText(
                "Update ngterm on build-01 to watch and type into its sessions from here.",
            ).assertExists()
        compose.onNodeWithText("Detach").performClick()
        compose.awaitText("Nothing attached")
        assertTrue(test.core.shownPanes.isEmpty())
    }

    @Test
    fun aSessionThatEndedElsewhereShowsItsLastNameAndCanBeClosed() {
        openSession()
        test.core.emit(MobileEvent.PaneEnded(p1, PaneEnd.Gone))
        compose.awaitText("docs-site is gone")
        compose.onNodeWithText("It ended on build-01, so there is nothing left to show.").assertExists()
        compose.onNodeWithText("Close").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("docs-site is gone").assertDoesNotExist()
        assertTrue(described("More options for docs-site"))
    }

    @Test
    fun endingFromTheBarConfirmsAndCancelKeepsTheSessionRunning() {
        openSession()
        menu()
        compose.onNodeWithText("End session").performClick()
        compose.onNodeWithText("End docs-site?").assertExists()
        assertTrue(test.core.calls.none { it.method == "endSession" })
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("End docs-site?").assertDoesNotExist()
        menu()
        compose.onNodeWithText("End session").performClick()
        endConfirmed()
        compose.awaitText("Nothing attached")
        assertTrue(
            test.core.sessions["m1"]
                .orEmpty()
                .isEmpty(),
        )
        assertTrue(test.core.calls.any { it.method == "endSession" && it.args == listOf("m1", "s1") })
    }

    @Test
    fun theAttachedListOpensAndTogglesChatViewAndItsEndDialogCancelsOrConfirms() {
        openSession()
        compose.onNodeWithContentDescription("Back").performClick()
        waitFor { !described("Escape") }
        menu()
        compose.onNodeWithText("Chat view").assertIsOn().performClick()
        menu()
        compose.onNodeWithText("Chat view").assertIsOff().performClick()
        menu()
        compose.onNodeWithText("End session").performClick()
        compose.onNodeWithText("Cancel").performClick()
        assertTrue(test.core.calls.none { it.method == "endSession" })
        compose.onNodeWithText("docs-site").performClick()
        waitFor { described("Escape") }
        compose.onNodeWithContentDescription("Back").performClick()
        waitFor { !described("Escape") }
        menu()
        compose.onNodeWithText("End session").performClick()
        endConfirmed()
        compose.awaitText("Nothing attached")
        assertTrue(
            test.core.sessions["m1"]
                .orEmpty()
                .isEmpty(),
        )
        compose.onNodeWithText("Open machines").performClick()
        compose.awaitText("Add machine")
    }

    @Test
    fun theAttachedListsDetachLeavesTheSessionRunningOnItsMachine() {
        openSession()
        layout.value = NavLayout.Sidebar
        compose.waitForIdle()
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        waitFor { !described("Escape") }
        menu()
        compose.onNodeWithText("Detach").performClick()
        compose.awaitText("Nothing attached")
        assertEquals(listOf("s1"), test.core.sessions["m1"]?.map { it.id })
    }

    @Test
    fun anEndRefusedByTheOriginShowsASnackbarAndLeavesTheTerminalUsable() {
        openSession()
        test.core.failNext("endSession", MobileException.Remote("session is protected"))
        menu()
        compose.onNodeWithText("End session").performClick()
        endConfirmed()
        compose.awaitText("session is protected")
        compose.onNodeWithContentDescription("Dismiss").performClick()
        waitFor { !shows("session is protected") }
        panes.getValue("P1").inputs.clear()
        compose.onNodeWithContentDescription("Escape").performClick()
        waitFor { pressed("P1") == listOf("escape") }
        assertEquals(listOf("s1"), test.core.sessions["m1"]?.map { it.id })
    }

    @Test
    fun theBarReportsStreamFailuresAndConnectingDuringAMachineReconnect() {
        openSession()
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Failed("stream lost")))
        waitFor { described("Reconnecting") }
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
        waitFor { !described("Reconnecting") }
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connecting))
        waitFor { described("Connecting") }
        test.core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(4000u)))
        waitFor { described("Reconnecting") }
        compose.onNodeWithText("docs-site").assertExists()
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun hardwareSessionChordsMoveForwardAndBackBetweenAttachedSessions() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession()
        // three sessions, so the next one (api-refactor) and the previous one (ops, wrapping round) differ
        test.core.sessions.getValue("m1") += fakeSession("s2", "api-refactor")
        test.core.sessions.getValue("m1") += fakeSession("s3", "ops")
        runBlocking {
            test.connections.listSessions("m1")
            test.connections.attach("m1", "s2")
            test.connections.attach("m1", "s3")
        }
        terminal(0).performClick()
        compose.waitForIdle()
        chord(Key.DirectionDown)
        compose.awaitText("Opening api-refactor")
        compose.onNodeWithText("Opening ops").assertDoesNotExist()
        // The new session has not received its layout yet; Back returns to the attached list.
        compose.onNodeWithContentDescription("Back").performClick()
        compose.awaitText("docs-site")
        compose.onNodeWithText("docs-site").performClick()
        waitFor { described("Terminal") }
        terminal(0).performClick()
        compose.waitForIdle()
        chord(Key.DirectionUp)
        compose.awaitText("Opening ops")
        compose.onNodeWithText("Opening api-refactor").assertDoesNotExist()
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aSessionChordWithNoOtherAttachedSessionReachesTheProgram() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession()
        terminal(0).performClick()
        compose.waitForIdle()
        panes.getValue("P1").inputs.clear()
        // docs-site is the only attached session: the chords have nowhere to go, so they are the program's
        chord(Key.DirectionUp)
        chord(Key.DirectionDown)
        waitFor { pressed("P1").containsAll(listOf("arrow_up", "arrow_down")) }
    }

    @Test
    fun aPaneAloneSurvivesSavedStateRestorationAndItsTabBringsTheSplitBack() {
        val restoration = StateRestorationTester(compose)
        openSession(restoration = restoration)
        layout.value = NavLayout.Sidebar
        waitFor { panes["P1"] != null && panes["P2"] != null }
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        compose.waitForIdle()
        assertEquals(1, terminals())
        restoration.emulateSavedInstanceStateRestore()
        compose.waitForIdle()
        assertEquals(1, terminals())
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected().performClick()
        compose.waitForIdle()
        assertEquals(2, terminals())
    }

    @Test
    fun theCollapsedRailOpensAnAttachedSessionFromAnotherDestination() {
        openSession()
        layout.value = NavLayout.Rail
        compose.waitForIdle()
        compose.onNode(hasText("Settings") and isSelectable()).performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("docs-site").performClick()
        waitFor { described("Terminal") }
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
    }

    @Test
    fun aChatWaitsForItsPaneToBeShownBeforeItOpensTheConversation() {
        openSession()
        val showing = CompletableDeferred<Unit>()
        test.core.holds["showPane"] = showing
        try {
            compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
            test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P2", "claude", "c2"))
            compose.awaitText("Reading the conversation from build-01")
            compose.onNodeWithContentDescription("Terminal").assertDoesNotExist()
            compose.onNodeWithContentDescription("Escape").assertDoesNotExist()
            assertTrue(p2 !in test.core.openChats)
        } finally {
            showing.complete(Unit)
        }
        waitFor { p2 in test.core.openChats }
        compose.onNodeWithContentDescription("Show terminal").performClick()
        waitFor { described("Terminal") }
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
    }

    @Test
    fun stackedChatsKeepBothTerminalGridsWhenTheirSharedExtraKeysStepAside() {
        openSplit()
        test.core.emit(
            MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "horizontal", true)),
        )
        compose.waitForIdle()
        // every resize each pane was given: switching views must add none, not even one it later undoes
        val before = panes.mapValues { it.value.sizes.toList() }
        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P1", "claude", "c1"))
        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P2", "codex", "c2"))
        waitFor {
            compose.onAllNodesWithContentDescription("Chat with The agent").fetchSemanticsNodes().size == 2
        }
        assertEquals(setOf(p1, p2), test.core.openChats)
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Escape").assertDoesNotExist()
        assertEquals(before, panes.mapValues { it.value.sizes.toList() })
        compose.onNodeWithContentDescription("Show terminal").performClick()
        waitFor { described("Terminal") }
        compose.waitForIdle()
        assertEquals(before, panes.mapValues { it.value.sizes.toList() })
    }

    @Test
    fun aChatKeepsAFailedPanesRetryButReadingItNeverAsksForControl() {
        openSession()
        test.core.failNext("showPane", MobileException.Remote("pane not available"))
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        compose.awaitText("pane not available")
        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P2", "claude", "c2"))
        compose.waitForIdle()
        compose.onNodeWithText("Can't show this pane").assertExists()
        compose.onNodeWithText("Retry").performClick()
        waitFor { described("Chat with The agent") }
        test.core.emit(MobileEvent.PaneLead(p2, PaneRole.FOLLOWER, true))
        compose.waitForIdle()
        compose.onNodeWithText("Another device is driving this pane").assertDoesNotExist()
        compose.onNodeWithContentDescription("Show terminal").performClick()
        compose.awaitText("Another device is driving this pane")
        compose.onNodeWithText("Take control").performClick()
        assertTrue(test.core.calls.any { it.method == "takeLead" && it.args == listOf("m1", "s1", "P2") })
    }

    @Test
    fun anExternalPaneSelectionWithTheKeyboardDownLetsTheOldInputFocusGo() {
        openSplit()
        terminal(0).performClick()
        compose.waitForIdle()
        compose.onNode(isFocused() and hasContentDescription("Terminal")).assertExists()
        compose.runOnUiThread {
            compose.activity.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(
                compose.activity.window.decorView.windowToken,
                0,
            )
        }
        // As a notification does: select a pane without touching it or raising the keyboard.
        test.graph.opens.request(OpenTarget("m1", "s1", "P2"))
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        compose.onNode(isFocused() and hasContentDescription("Terminal")).assertDoesNotExist()
        terminal(1).performClick()
        compose.waitForIdle()
        panes.values.forEach { it.inputs.clear() }
        compose.onNodeWithContentDescription("Escape").performClick()
        waitFor { pressed("P2") == listOf("escape") }
        assertTrue(pressed("P1").isEmpty())
    }
}
