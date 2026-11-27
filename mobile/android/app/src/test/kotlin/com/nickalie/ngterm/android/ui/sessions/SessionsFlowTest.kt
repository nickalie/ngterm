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
import androidx.compose.ui.test.performTextClearance
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitDescription
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.CatalogState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.runBlocking
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

    @Test
    fun theCatalogsRowsShowLiveAndAnEndedOrPageRowIsNotAttached() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        openMachine()
        compose.awaitText("build")
        val rows =
            listOf(
                fakeSession("s1", name = "build").copy(ended = true),
                fakeSession("p1", name = "notes.md").copy(page = "file"),
            )
        // the listing the screen makes as it opens may land after the catalog's rows: it brings the same ones
        test.core.sessions["m1"] = rows.toMutableList()
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Connected))
        test.core.emit(MobileEvent.Tree("m1", rows, null))
        compose.awaitDescription("Ended on build-01")
        compose.onNodeWithText("File page").assertExists()
        compose.onNodeWithText("build").performClick()
        compose.onNodeWithText("notes.md").performClick()
        compose.waitForIdle()
        assertEquals(null, test.connections.attached.value["m1"])
        assertEquals(0, test.core.calls.count { it.method == "attachSession" })
        compose.onNodeWithContentDescription("More options for notes.md").performClick()
        compose.onNodeWithText("Attach").assertDoesNotExist()
        compose.onNodeWithText("Duplicate session").assertDoesNotExist()
        compose.onNodeWithText("Rename session").assertExists()
        compose.onNodeWithText("End session").assertExists()
    }

    @Test
    fun anAttachedSessionThatEndedCanStillBeDetachedFromItsRow() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        openMachine()
        compose.awaitText("build")
        runBlocking { test.connections.attach("m1", "s1") }
        val rows = listOf(fakeSession("s1", name = "build").copy(ended = true))
        test.core.sessions["m1"] = rows.toMutableList()
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Connected))
        test.core.emit(MobileEvent.Tree("m1", rows, null))
        compose.awaitDescription("Ended on build-01")
        compose.onNodeWithContentDescription("More options for build").performClick()
        compose.onNodeWithText("Detach").performClick()
        compose.waitForIdle()
        assertEquals(null, test.connections.attached.value["m1"])
    }

    @Test
    fun theCatalogsStateShowsOverTheList() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        openMachine()
        compose.awaitText("build")
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Connecting))
        compose.awaitText("Connecting to build-01…")
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Disconnected("connection reset")))
        compose.awaitText("Disconnected: connection reset. Retrying.")
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Refused("build-01 runs an older ngterm")))
        compose.awaitText("build-01 runs an older ngterm")
        test.core.emit(MobileEvent.Catalog("m1", CatalogState.Connected))
        gone("build-01 runs an older ngterm")
        compose.onNodeWithText("build").assertExists()
    }

    @Test
    fun renameSessionRenamesItAndShowsWhyItFailed() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        openMachine()
        compose.awaitText("build")
        compose.onNodeWithContentDescription("More options for build").performClick()
        compose.onNodeWithText("Rename session").performClick()
        compose.onNodeWithText("Rename build").assertExists()
        test.core.failNext("renameSession", MobileException.Remote("build-01 did not run Rename"))
        compose.onNodeWithText("Name").performTextClearance()
        compose.onNodeWithText("Name").performTextInput("api")
        compose.onNode(hasText("Rename") and hasAnyAncestor(isDialog())).performClick()
        compose.awaitText("build-01 did not run Rename")
        compose.onNode(hasText("Rename") and hasAnyAncestor(isDialog())).performClick()
        gone("Rename build")
        compose.awaitText("api")
        assertEquals(
            listOf("m1", "s1", "api"),
            test.core.calls
                .last { it.method == "renameSession" }
                .args,
        )
    }
}
