package com.nickalie.ngterm.android.ui.sessions

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileException
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

// the stores and the fake core answer on their own threads: wait for what they bring
private const val WAIT_MS = 10_000L

/**
 * A machine's sessions end to end on the fake core: open the machine, create a session, end it. Robolectric's default
 * window: under a phone's qualifiers its layout of a text field in a dialog never settles (the screenshot tests show
 * the dialog at phone sizes).
 */
@RunWith(AndroidJUnit4::class)
class SessionsFlowTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() = test.close()

    private fun app() {
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = NavLayout.Bar) }
        }
    }

    private fun gone(text: String) {
        compose.waitUntil(WAIT_MS) {
            compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isEmpty()
        }
    }

    // [beforeOpen] runs once the machine is connected and listed (or failed to list), before its sessions open (and
    // list it again)
    private fun openMachine(beforeOpen: () -> Unit = {}) {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        app()
        compose.waitUntil(WAIT_MS) {
            test.connections.trees.value["m1"] != null || test.connections.treeFailures.value["m1"] != null
        }
        beforeOpen()
        compose.onNodeWithText("build-01").performClick()
    }

    @Test
    fun aNamedSessionFromALongPressOpensThenEndItAndItIsGone() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        openMachine()
        compose.awaitText("On build-01")
        compose.onNodeWithText("build").assertExists()
        compose.onNodeWithText("New session").performTouchInput { longClick() }
        compose.onNodeWithText("New session on build-01").assertExists()
        compose.onNodeWithText("Name").performTextInput("api-refactor")
        compose.onNodeWithText("Create session").performClick()
        compose.awaitText("Opening api-refactor")
        gone("New session on build-01")
        assertEquals(setOf("fake-1"), test.connections.attached.value["m1"])
        compose.onNodeWithContentDescription("More options for api-refactor").performClick()
        compose.onNodeWithText("End session").performClick()
        compose.onNodeWithText("End api-refactor?").assertExists()
        compose.onNode(hasText("End session") and hasAnyAncestor(isDialog())).performClick()
        compose.awaitText("Nothing attached")
        assertEquals(listOf("s1"), test.core.sessions["m1"]?.map { it.id })
    }

    @Test
    fun anEmptyMachineOffersNewSessionAndBackReturnsToTheMachines() {
        openMachine()
        compose.awaitText("No sessions on build-01")
        // the empty state's own New session, beside the top row's
        assertEquals(2, compose.onAllNodesWithText("New session").fetchSemanticsNodes().size)
        compose.onNodeWithContentDescription("Back").performClick()
        compose.awaitText("Add machine")
    }

    @Test
    fun aMissingNgtermctlPointsToTheReadmeAndRetryLists() {
        val missing = MobileException.Remote("/bin/sh: 1: ngtermctl: not found")
        // the connect's listing fails, and so does the one the screen makes when it opens
        test.core.failNext("listSessions", missing)
        openMachine { test.core.failNext("listSessions", missing) }
        compose.awaitText("ngtermctl isn't on build-01")
        compose.onNodeWithText("Open the README").assertExists()
        compose.onNodeWithText("Retry").performClick()
        compose.awaitText("No sessions on build-01")
    }

    @Test
    fun aSessionTappedIsAttachedAndItsViewOpens() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"), fakeSession("s2", name = "docs"))
        openMachine()
        compose.awaitText("docs")
        compose.onNodeWithText("docs").performClick()
        // the session view opens under Sessions, waiting for the stream's first layout
        compose.awaitText("Opening docs")
        assertEquals(setOf("s2"), test.connections.attached.value["m1"])
        compose.onNodeWithContentDescription("More options for docs").performClick()
        compose.onNodeWithText("Detach").performClick()
        compose.awaitText("Nothing attached")
        assertEquals(null, test.connections.attached.value["m1"])
    }
}
