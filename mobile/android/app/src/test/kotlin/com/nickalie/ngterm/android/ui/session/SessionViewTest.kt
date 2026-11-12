package com.nickalie.ngterm.android.ui.session

import android.content.res.Configuration
import android.view.View
import android.view.ViewGroup
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.withKeysDown
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.key
import com.nickalie.ngterm.android.data.part
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneKeyAction
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteLayout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The session view on the fake core, in the app: tapping a session on a machine's list attaches it and opens its
 * view; the panes are shown as their slots get a size; switching panes and changing the window's size class keep the
 * focused pane, where the extra keys type; a follower's Take control, a reconnecting pane, a program's clipboard write
 * and Detach. A window wide enough for the sidebar beside the session: the layout is set by the test, not the window.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class SessionViewTest : SessionViewFixture() {
    @Test
    fun switchingPanesAndTheSizeClassKeepTheFocusedPaneWhereTheExtraKeysType() {
        openSession()
        // a phone shows one pane at a time: the second is shown once switched to
        assertTrue(p2 !in test.core.shownPanes)
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        waitFor { p2 in test.core.shownPanes }
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()

        layout.value = NavLayout.Sidebar
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsNotSelected()
        // the sidebar lists the open session
        compose.onNodeWithText("build-01").assertExists()

        waitFor { panes["P2"] != null }
        panes.getValue("P2").inputs.clear()
        compose.onNodeWithContentDescription("Escape").performClick()
        waitFor { panes.getValue("P2").inputs.any { it is FakeInput.Key && it.key.key == "escape" } }
        assertTrue(panes.getValue("P1").inputs.none { it is FakeInput.Key })

        layout.value = NavLayout.Bar
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
    }

    @Test
    fun theExtraKeysReachNoPaneWhileTheFocusedOneCantBeShown() {
        openSession()
        waitFor { panes["P1"] != null }
        panes.getValue("P1").inputs.clear()
        test.core.failNext("showPane", MobileException.Remote("no pane P2"))
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        waitFor { shows("no pane P2") }
        compose.onNodeWithContentDescription("Escape").performClick()
        compose.waitForIdle()
        assertTrue(panes.getValue("P1").inputs.none { it is FakeInput.Key })
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aHardwareChordMovesTheKeyboardFocusToTheNextPane() {
        // a hardware keyboard attached and open (Robolectric has no qualifier for it): the soft keyboard stays down
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession()
        layout.value = NavLayout.Sidebar
        waitFor { p2 in test.core.shownPanes && panes["P2"] != null }
        compose.onAllNodesWithContentDescription("Terminal")[0].performClick()
        compose.waitForIdle()
        compose.onNode(isFocused()).performKeyInput {
            withKeysDown(listOf(Key.CtrlLeft, Key.ShiftLeft)) { pressKey(Key.DirectionRight) }
        }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        panes.getValue("P1").inputs.clear()
        compose.onNode(isFocused()).performKeyInput { pressKey(Key.A) }
        waitFor { panes.getValue("P2").inputs.any { it is FakeInput.Key } }
        assertTrue(panes.getValue("P1").inputs.none { it is FakeInput.Key })
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aPaneChordInASessionOfOnePaneReachesTheProgram() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession(withLayout = false)
        test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1"), "P1", "vertical", true)))
        waitFor { p1 in test.core.shownPanes && panes["P1"] != null }
        terminal(0).performClick()
        compose.waitForIdle()
        panes.getValue("P1").inputs.clear()
        // the session's only pane: the chords have nowhere to go, so they are the program's
        chord(Key.DirectionLeft)
        chord(Key.DirectionRight)
        waitFor { pressed("P1").containsAll(listOf("arrow_left", "arrow_right")) }
    }

    // the split's two panes side by side, both shown
    private fun openSplit() {
        openSession()
        layout.value = NavLayout.Sidebar
        waitFor { panes["P1"] != null && panes["P2"] != null }
        compose.waitForIdle()
    }

    private fun terminal(index: Int) = compose.onAllNodesWithContentDescription("Terminal")[index]

    // the view the input method talks to: the window's compose view
    private fun composeView(): View {
        val views = ArrayDeque<View>(listOf(compose.activity.window.decorView))
        while (views.isNotEmpty()) {
            val view = views.removeFirst()
            if (view.javaClass.simpleName == "AndroidComposeView") return view
            if (view is ViewGroup) (0 until view.childCount).forEach { views += view.getChildAt(it) }
        }
        error("no compose view")
    }

    // the input method's connection as it opens one now; null when no terminal takes it
    private fun connect(): InputConnection? = composeView().onCreateInputConnection(EditorInfo())

    private fun clearInputs() = panes.values.forEach { it.inputs.clear() }

    // what reached [pane] from the keyboard: the text typed, and the keys pressed (not their releases)
    private fun typed(pane: String) =
        panes
            .getValue(pane)
            .inputs
            .filterIsInstance<FakeInput.Typed>()
            .joinToString("") { it.text }

    private fun pressed(pane: String) =
        panes
            .getValue(pane)
            .inputs
            .filterIsInstance<FakeInput.Key>()
            .filter { it.key.action != PaneKeyAction.RELEASE }
            .map { it.key.key }

    private fun nothingReached(pane: String) = typed(pane).isEmpty() && pressed(pane).isEmpty()

    // through the root, to whatever has the focus: a slot without a terminal holds it as a bare focus target, which
    // has no semantics node of its own
    private fun chord(arrow: Key) {
        compose.onRoot().performKeyInput {
            withKeysDown(listOf(Key.CtrlLeft, Key.ShiftLeft)) { pressKey(arrow) }
        }
        compose.waitForIdle()
    }

    private fun terminals() = compose.onAllNodesWithContentDescription("Terminal").fetchSemanticsNodes().size

    private fun tabAction(tab: String) =
        compose
            .onNodeWithContentDescription(tab)
            .fetchSemanticsNode()
            .config
            .getOrNull(SemanticsActions.OnClick)
            ?.label

    @Test
    fun aPaneTabTogglesBetweenTheSplitAndItsPaneAlone() {
        openSplit()
        assertEquals(2, terminals())
        assertEquals("Show this pane alone", tabAction("Pane 2 of 2"))
        // a tab in the split shows its pane alone, selected
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        compose.waitForIdle()
        assertEquals(1, terminals())
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        assertEquals("Show both panes", tabAction("Pane 2 of 2"))
        // the other tab shows the other pane alone instead
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        assertEquals(1, terminals())
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        // a trip to Settings and back finds the pane still alone
        compose.onNode(hasText("Settings") and isSelectable()).performClick()
        compose.waitForIdle()
        compose.onNode(hasText("Sessions") and isSelectable()).performClick()
        compose.waitForIdle()
        assertEquals(1, terminals())
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        // the pane's own tab brings the split back, the selection staying
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        assertEquals(2, terminals())
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
    }

    // a key typed into the focused node reaches pane [id]
    private fun keyReaches(id: String) {
        clearInputs()
        compose.onNode(isFocused()).performKeyInput { pressKey(Key.A) }
        waitFor { panes.getValue(id).inputs.any { it is FakeInput.Key } }
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aHardwareKeyboardKeepsTypingIntoTheSelectedPaneAsItsTabTogglesTheSplit() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSplit()
        terminal(0).performClick()
        compose.waitForIdle()
        keyReaches("P1")
        // its own tab shows it alone, then brings the split back: the selection stays, and so does the typing
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        assertEquals(1, terminals())
        keyReaches("P1")
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        assertEquals(2, terminals())
        keyReaches("P1")
        assertTrue(panes.getValue("P2").inputs.none { it is FakeInput.Key })
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aPaneThatCouldntBeShownShownAloneStillTakesTheChords() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession()
        waitFor { panes["P1"] != null }
        test.core.failNext("showPane", MobileException.Remote("no pane P2"))
        layout.value = NavLayout.Sidebar
        waitFor { shows("no pane P2") }
        terminal(0).performClick()
        compose.waitForIdle()
        // the failed pane's tab shows it alone: its terminal-less slot takes the focus, and the chords move on
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        assertEquals(0, terminals())
        chord(Key.DirectionLeft)
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        keyReaches("P1")
    }

    @Test
    fun aTabPickedWithTheSoftKeyboardDownThenARetapTypesEverythingIntoTheRetappedPane() {
        openSplit()
        terminal(1).performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        // the tab moves the selection (and shows its pane alone, a retap the split again); the keyboard stays down
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        compose.onNodeWithContentDescription("Pane 1 of 2").performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        // a tap on the pane that had the focus selects it again: the input method and the extra keys both go there
        terminal(1).performClick()
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        clearInputs()
        requireNotNull(connect()) { "no input connection after the tap" }.commitText("ls", 1)
        compose.onNodeWithContentDescription("Escape").performClick()
        compose.waitForIdle()
        assertEquals("ls", typed("P2"))
        assertEquals(listOf("escape"), pressed("P2"))
        assertTrue(nothingReached("P1"))
    }

    @Test
    fun anOldInputConnectionLandsItsCompositionInItsOwnPaneAndNothingAfterTheSelectionMoves() {
        openSplit()
        terminal(0).performClick()
        compose.waitForIdle()
        val old = requireNotNull(connect()) { "no input connection after the tap" }
        clearInputs()
        old.setComposingText("ab", 1)
        compose.onNodeWithContentDescription("Pane 2 of 2").performClick()
        compose.waitForIdle()
        // the composition pending at the switch is finished where it was typed
        assertEquals("ab", typed("P1"))
        old.commitText("zz", 1)
        old.setComposingText("q", 1)
        old.deleteSurroundingText(1, 0)
        old.performEditorAction(EditorInfo.IME_ACTION_NONE)
        old.sendKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, android.view.KeyEvent.KEYCODE_X))
        compose.waitForIdle()
        assertEquals("ab", typed("P1"))
        assertTrue(pressed("P1").isEmpty())
        assertTrue(nothingReached("P2"))
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun anUnavailablePaneTakesNoTypingWhileTheChordsStillMoveAndTakesItOnceItRecovers() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSession()
        waitFor { panes["P1"] != null }
        test.core.failNext("showPane", MobileException.Remote("no pane P2"))
        layout.value = NavLayout.Sidebar
        waitFor { shows("no pane P2") }
        terminal(0).performClick()
        compose.waitForIdle()
        chord(Key.DirectionRight)
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        clearInputs()
        compose.onRoot().performKeyInput { pressKey(Key.A) }
        connect()?.commitText("x", 1)
        compose.onNodeWithContentDescription("Escape").assertDoesNotExist()
        compose.waitForIdle()
        assertTrue(nothingReached("P1"))
        // the chords still move, away and back
        chord(Key.DirectionLeft)
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        chord(Key.DirectionRight)
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        clearInputs()
        // shown at last, it takes the keys
        compose.onNodeWithText("Retry").performClick()
        waitFor { panes["P2"] != null }
        compose.waitForIdle()
        compose.onNode(isFocused()).performKeyInput { pressKey(Key.A) }
        waitFor { pressed("P2").isNotEmpty() }
        assertTrue(nothingReached("P1"))
    }

    @Test
    fun aSwipeAcrossAPhonesTerminalMovesBetweenThePanes() {
        openSession()
        test.core.emit(
            MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2", "P3"), "P1", "vertical", true)),
        )
        waitFor { described("Pane 3 of 3") }
        // a swipe to the left goes forward, one to the right back, wrapping from the first to the last
        compose.onNodeWithContentDescription("Terminal").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 2 of 3").assertIsSelected()
        waitFor { p2 in test.core.shownPanes }
        compose.onNodeWithContentDescription("Terminal").performTouchInput { swipeRight() }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 1 of 3").assertIsSelected()
        compose.onNodeWithContentDescription("Terminal").performTouchInput { swipeRight() }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Pane 3 of 3").assertIsSelected()
    }

    @Test
    fun aFollowerTakesControlAndAReconnectingPaneSaysSo() {
        openSession()
        test.core.emit(MobileEvent.PaneLead(p1, PaneRole.FOLLOWER, true))
        waitFor { shows("Another device is driving this pane") }
        compose.onNodeWithText("Take control").performClick()
        assertTrue(test.core.calls.any { it.method == "takeLead" && it.args == listOf("m1", "s1", "P1") })

        test.core.emit(MobileEvent.PaneLead(p1, PaneRole.LEADER, false))
        waitFor { !shows("Another device is driving this pane") }
        test.core.emit(MobileEvent.PaneEnded(p1, PaneEnd.Lost("link down")))
        waitFor { shows("Reconnecting to build-01") }
        test.core.emit(MobileEvent.PaneAttached(p1))
        waitFor { !shows("Reconnecting to build-01") }
    }

    @Test
    fun aProgramsClipboardWriteReachesTheDevicesClipboard() {
        openSession()
        test.core.emit(MobileEvent.ClipboardWrite(p1, 1u, listOf(part("text/plain", "copied text"))))
        waitFor { test.core.calls.any { it.method == "completeClipboardWrite" } }
        assertEquals(
            listOf(listOf("text/plain" to "copied text".toByteArray().toList())),
            test.clipboardWrites.map { parts ->
                parts.map { it.key() }
            },
        )
    }

    @Test
    fun theBarSwitchesAPaneRunningAnAgentBetweenItsChatAndItsTerminal() {
        openSession()
        // no agent, no switch
        assertTrue(described("Show terminal").not() && described("Show chat").not())
        // an agent in the other pane: the switch is the focused pane's
        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P2", "codex", "t1"))
        compose.waitForIdle()
        assertTrue(described("Show terminal").not() && described("Show chat").not())

        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P1", "claude", "c1"))
        waitFor { described("Show terminal") }
        assertEquals(setOf(p1), test.core.openChats)
        compose.onNodeWithContentDescription("Show terminal").performClick()
        waitFor { described("Show chat") }
        assertTrue(test.core.openChats.isEmpty())
        compose.onNodeWithContentDescription("Show chat").performClick()
        waitFor { described("Show terminal") }
        assertEquals(setOf(p1), test.core.openChats)
    }

    @Test
    fun aPaneInChatViewDrawsItsChatInTheTerminalsPlaceAtTheSameGrid() {
        openSession()
        waitFor { panes["P1"]?.sizes?.isNotEmpty() == true }
        val sized = panes.getValue("P1").sizes.toList()
        assertTrue(described("Escape"))

        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P1", "claude", "c1"))
        waitFor { described("Chat with The agent") }
        // the chat stands in the terminal's place, and the extra keys step aside
        assertTrue(described("Terminal").not() && described("Escape").not())
        val events =
            listOf(
                ChatEvent("c", 0u, null, ChatBody.Conversation("claude", "c1", null, null)),
                ChatEvent("u1", 0u, null, ChatBody.User("Fix the install guide", 0u)),
                ChatEvent("live", 0u, null, ChatBody.Live),
            )
        test.core.emit(MobileEvent.Chat(p1, events))
        waitFor { described("You: Fix the install guide") }
        compose.onNodeWithContentDescription("Chat with Claude Code").assertExists()

        compose.onNodeWithContentDescription("Show terminal").performClick()
        waitFor { described("Terminal") && described("Escape") }
        // switching the view never resized the pane: its terminal kept its grid behind the chat
        assertEquals(sized, panes.getValue("P1").sizes.toList())
    }

    @Test
    @Config(qualifiers = "+qwerty")
    fun aHardwareChordMovesAwayFromAPaneShowingItsChatAndBack() {
        compose.activity.resources.configuration.hardKeyboardHidden = Configuration.HARDKEYBOARDHIDDEN_NO
        openSplit()
        waitFor { described("Terminal") }
        // the focused pane switches to its chat: the chat takes the focus in its terminal's place
        test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P1", "claude", "c1"))
        waitFor { described("Chat with The agent") }
        compose.waitForIdle()
        clearInputs()
        chord(Key.DirectionRight)
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        chord(Key.DirectionLeft)
        compose.onNodeWithContentDescription("Pane 1 of 2").assertIsSelected()
        // a chat types into no terminal, but its chords still move
        compose.onRoot().performKeyInput { pressKey(Key.A) }
        chord(Key.DirectionRight)
        compose.onNodeWithContentDescription("Pane 2 of 2").assertIsSelected()
        assertTrue(nothingReached("P1"))
    }

    @Test
    fun backClosesTheViewAndKeepsTheSessionAttached() {
        openSession()
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        waitFor { described("Escape").not() }
        // the attached list, not Machines: the session is still there, with its menu
        assertTrue(described("More options for docs-site"))
        assertTrue(
            "s1" in
                test.connections.attached.value["m1"]
                    .orEmpty(),
        )
    }

    @Test
    fun detachClosesTheViewAndHidesItsPanes() {
        openSession()
        compose.onNodeWithContentDescription("More options for docs-site").performClick()
        compose.onNodeWithText("Detach").performClick()
        waitFor {
            test.connections.attached.value["m1"]
                .isNullOrEmpty()
        }
        waitFor { described("Escape").not() }
        assertTrue(test.core.shownPanes.isEmpty())
        assertTrue(shows("Nothing attached"))
    }
}
