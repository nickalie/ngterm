package com.nickalie.ngterm.android.ui.session

import android.content.res.Configuration
import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.withKeysDown
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeInput
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneClipboard
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import java.util.concurrent.ConcurrentHashMap

private const val WAIT_MS = 10_000L

/**
 * The session view on the fake core, in the app: tapping a session on a machine's list attaches it and opens its
 * view; the panes are shown as their slots get a size; switching panes and changing the window's size class keep the
 * focused pane, where the extra keys type; a follower's Take control, a reconnecting pane, a program's clipboard write
 * and Detach. A window wide enough for the sidebar beside the session: the layout is set by the test, not the window.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class SessionViewTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val layout = mutableStateOf(NavLayout.Bar)
    private val panes = ConcurrentHashMap<String, FakePane>()
    private val p1 = PaneRef("m1", "s1", "P1")
    private val p2 = PaneRef("m1", "s1", "P2")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() = test.close()

    private fun waitFor(check: () -> Boolean) = compose.waitUntil(WAIT_MS, check)

    private fun shows(text: String) = compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    private fun described(text: String) =
        compose.onAllNodes(hasContentDescription(text)).fetchSemanticsNodes().isNotEmpty()

    // the app with docs-site (s1) on build-01 opened from the machine's list, its stream live with panes P1 and P2
    private fun openSession() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout.value) }
        }
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory =
            { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }).also { panes[pane] = it } }
        compose.onNodeWithText("build-01").performClick()
        compose.waitUntil(WAIT_MS) { shows("docs-site") }
        compose.onNodeWithText("docs-site").performClick()
        waitFor {
            "s1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
        test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)))
        compose.waitForIdle()
        waitFor { p1 in test.core.shownPanes }
    }

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
        test.core.emit(MobileEvent.ClipboardWrite(p1, PaneClipboard.STANDARD, "copied text"))
        waitFor { test.clipboardWrites.isNotEmpty() }
        assertEquals(listOf("copied text"), test.clipboardWrites.toList())
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
