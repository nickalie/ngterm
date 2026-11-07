package com.nickalie.ngterm.android.ui.sessions

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.lifecycle.ViewModelProvider
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf

private const val ROUTE_WAIT_MS = 10_000L

/** The machine session route's failure recovery and menus, through real stores and view models. */
@RunWith(RobolectricTestRunner::class)
class SessionsRouteCoverageTest {
    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val opened = mutableListOf<SessionKey>()
    private var backs = 0
    private var reviews = 0

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
    }

    @After
    fun tearDown() = test.close()

    private fun route() {
        compose.setContent {
            NgtermTheme {
                SessionsRoute(test.graph, "m1", { backs++ }, { reviews++ }, { opened += it })
            }
        }
        compose.awaitText("build-01")
    }

    private fun connect() {
        test.connections.connect(testMachine().toCore())
        compose.waitUntil(ROUTE_WAIT_MS) { test.connections.trees.value["m1"] != null }
    }

    private fun menu() = compose.onNodeWithContentDescription("More options for build").performClick()

    private fun confirmEnd() = compose.onNode(hasText("End session") and hasAnyAncestor(isDialog())).performClick()

    private fun gone(text: String) {
        compose.waitUntil(ROUTE_WAIT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isEmpty() }
    }

    private fun calls(method: String) = test.core.calls.count { it.method == method }

    @Test
    fun offlineRetryAndConnectRecoverWithoutOfferingNewSessionBeforeConnected() {
        test.online.value = false
        route()
        compose.awaitText("You're offline")
        compose.onNodeWithText("New session").assertDoesNotExist()
        test.online.value = true
        compose.awaitText("build-01 isn't connected")
        test.core.failNext("connect", MobileException.Unreachable("host refused connection"))
        compose.onNodeWithText("Connect").performClick()
        compose.awaitText("host refused connection")
        compose.onNodeWithText("New session").assertDoesNotExist()
        compose.onNodeWithText("Retry").performClick()
        compose.awaitText("No sessions on build-01")
        assertEquals(2, calls("connect"))
    }

    @Test
    fun retryWhileTheDeviceIsOfflineStillAsksTheCoreToConnect() {
        test.online.value = false
        route()
        compose.awaitText("You're offline")
        test.core.failNext("connect", MobileException.Unreachable("network unavailable"))
        compose.onNodeWithText("Retry").performClick()
        compose.awaitText("network unavailable")
        assertEquals(1, calls("connect"))
    }

    @Test
    fun aDroppedConnectionCountsDownAndRetryNowConnects() {
        connect()
        route()
        compose.runOnIdle { test.core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(4000u))) }
        compose.awaitText("Trying again in 4 s.")
        compose.onNodeWithText("New session").assertDoesNotExist()
        test.now += 4000L
        compose.mainClock.advanceTimeBy(4000L)
        compose.awaitText("Trying again now.")
        compose.onNodeWithText("Retry now").performClick()
        compose.awaitText("No sessions on build-01")
        assertEquals(2, calls("connect"))
    }

    @Test
    fun aFailedFirstListingRetriesTheListAndAMissingCliOpensTheServerInstructions() {
        test.core.failNext("listSessions", MobileException.Remote("session store unavailable"))
        test.connections.connect(testMachine().toCore())
        compose.waitUntil(ROUTE_WAIT_MS) { test.connections.treeFailures.value["m1"] != null }
        test.core.failNext("listSessions", MobileException.Remote("session store unavailable"))
        route()
        compose.awaitText("Can't list the sessions on build-01")
        compose.onNodeWithText("session store unavailable").assertExists()
        test.core.failNext("listSessions", MobileException.Remote("ngtermctl: command not found"))
        compose.onNodeWithText("Retry").performClick()
        compose.awaitText("ngtermctl isn't on build-01")
        compose.onNodeWithText("Open the README").performClick()
        assertEquals(SERVER_SETUP_URL, shadowOf(compose.activity).nextStartedActivity.dataString)
        compose.onNodeWithText("Retry").performClick()
        compose.awaitText("No sessions on build-01")
    }

    @Test
    fun reviewAndRemovalNavigateAwayFromTheMachine() {
        route()
        compose.runOnIdle { test.core.emit(MobileEvent.HostKeyChanged("m1", "old", "new")) }
        compose.awaitText("Review")
        compose.onNodeWithText("Review").performClick()
        assertEquals(1, reviews)
        runBlocking { test.graph.machines.delete("m1") }
        gone("Review")
        assertEquals(1, backs)
        compose.onNodeWithText("Review").assertDoesNotExist()
    }

    @Test
    fun attachFromTheMenuDetachAndChatPreferenceReachTheRealSessionState() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", "build"))
        connect()
        route()
        compose.awaitText("build")
        menu()
        compose.onNodeWithText("Chat view").assertIsOn().performClick()
        menu()
        compose.onNodeWithText("Chat view").assertIsOff().performClick()
        menu()
        compose.onNodeWithText("Chat view").assertIsOn()
        compose.onNodeWithText("Attach").performClick()
        compose.waitUntil(ROUTE_WAIT_MS) { opened.isNotEmpty() }
        assertEquals(listOf(SessionKey("m1", "s1")), opened)
        compose.awaitText("Attached")
        menu()
        compose.onNodeWithText("Detach").performClick()
        gone("Attached")
        assertEquals(null, test.connections.attached.value["m1"])
        assertEquals(listOf("s1"), test.core.sessions["m1"]?.map { it.id })
    }

    @Test
    fun aFailedAttachShowsItsReasonOnceAndCanBeRetried() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", "build"))
        connect()
        route()
        compose.awaitText("build")
        test.core.failNext("attachSession", MobileException.Remote("attach refused"))
        compose.onNodeWithText("build").performClick()
        compose.awaitText("attach refused")
        assertTrue(opened.isEmpty())
        compose.onNodeWithContentDescription("Dismiss").performClick()
        gone("attach refused")
        val vm = ViewModelProvider(compose.activity)["sessions:m1", SessionsViewModel::class.java]
        compose.waitUntil(ROUTE_WAIT_MS) { vm.notice.value == null }
        compose.onNodeWithText("build").performClick()
        compose.waitUntil(ROUTE_WAIT_MS) { opened.size == 1 }
        assertEquals(SessionKey("m1", "s1"), opened.single())
    }

    @Test
    fun endCanBeCancelledAndWhileItRunsTheRowSaysEndingAndDisablesAnotherEnd() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", "build").copy(status = "blocked"))
        connect()
        route()
        compose.awaitText("build")
        menu()
        compose.onNodeWithText("End session").performClick()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, calls("endSession"))
        val ending = CompletableDeferred<Unit>()
        test.core.holds["endSession"] = ending
        try {
            menu()
            compose.onNodeWithText("End session").performClick()
            confirmEnd()
            compose.awaitText("Ending")
            compose.onNodeWithContentDescription("Waiting").assertDoesNotExist()
            menu()
            compose.onNodeWithText("End session").assertIsNotEnabled()
            compose.onNodeWithText("End session").performClick()
            assertEquals(1, calls("endSession"))
        } finally {
            ending.complete(Unit)
        }
        compose.awaitText("No sessions on build-01")
        assertTrue(
            test.core.sessions["m1"]
                .orEmpty()
                .isEmpty(),
        )
    }

    @Test
    fun aFailedRefreshKeepsAnEmptyListUsableWithOfflineAndErrorBanners() {
        connect()
        route()
        compose.awaitText("No sessions on build-01")
        test.online.value = false
        compose.awaitText("You're offline. Connected machines reconnect when the network is back.")
        test.core.failNext("listSessions", MobileException.Remote("refresh refused"))
        runBlocking { test.connections.listSessions("m1") }
        compose.awaitText("The list may be out of date: refresh refused")
        compose.onNodeWithText("Retry").performClick()
        gone("The list may be out of date: refresh refused")
        compose.onAllNodesWithText("New session")[1].performClick()
        compose.awaitText("New session on build-01")
        compose.onNodeWithText("Cancel").performClick()
        gone("New session on build-01")
        assertEquals(0, calls("createSession"))
    }

    @Test
    fun restoringTheRouteDoesNotListAgainAndBackKeepsTheMachineConnected() {
        connect()
        val restoration = StateRestorationTester(compose)
        restoration.setContent {
            NgtermTheme { SessionsRoute(test.graph, "m1", { backs++ }, {}, {}) }
        }
        compose.awaitText("No sessions on build-01")
        val listed = calls("listSessions")
        restoration.emulateSavedInstanceStateRestore()
        compose.awaitText("No sessions on build-01")
        assertEquals(listed, calls("listSessions"))
        compose.onNodeWithContentDescription("Back").performClick()
        assertEquals(1, backs)
        assertEquals(Link.Connected, test.connections.links.value["m1"])
    }

    @Test
    fun rowsPreferContextThenWorkingDirectoryAndNameTheirCompanionsParent() {
        test.core.sessions["m1"] =
            mutableListOf(
                fakeSession("s1", "build").copy(context = "Compile the server", cwd = "/hidden"),
                fakeSession("s2", "review", parent = "s1").copy(context = "", cwd = "/companion"),
                fakeSession("s3", "shell").copy(context = "  ", cwd = "/work"),
                fakeSession("s4", "bare").copy(context = null, cwd = ""),
            )
        connect()
        route()
        compose.awaitText("Compile the server")
        compose.onNodeWithText("/hidden").assertDoesNotExist()
        compose.onNodeWithText("Companion of build").assertExists()
        compose.onNodeWithText("/companion").assertDoesNotExist()
        compose.onNodeWithText("/work").assertExists()
        compose.onNodeWithText("bare").assertExists()
        compose.onNodeWithText("shell").performClick()
        compose.waitUntil(ROUTE_WAIT_MS) { opened.isNotEmpty() }
        assertEquals(SessionKey("m1", "s3"), opened.single())
    }

    @Test
    fun connectingThenReadingExplainTheWaitBeforeTheFirstListArrives() {
        route()
        val connecting = CompletableDeferred<Unit>()
        val reading = CompletableDeferred<Unit>()
        test.core.holds["connect"] = connecting
        test.core.holds["listSessions"] = reading
        try {
            compose.onNodeWithText("Connect").performClick()
            compose.awaitText("Connecting to build-01")
            compose.onNodeWithText("New session").assertDoesNotExist()
            connecting.complete(Unit)
            compose.awaitText("Reading the sessions on build-01")
            compose.onNodeWithText("New session").assertExists()
        } finally {
            connecting.complete(Unit)
            reading.complete(Unit)
        }
        compose.awaitText("No sessions on build-01")
    }

    @Test
    fun anOfflineListKeepsItsRowsAndAFailedRefreshOffersRetryOverThem() {
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", "build"))
        connect()
        route()
        compose.awaitText("build")
        test.online.value = false
        compose.awaitText("You're offline. Connected machines reconnect when the network is back.")
        compose.onNodeWithText("build").assertExists()
        test.core.failNext("listSessions", MobileException.Remote("listing timed out"))
        runBlocking { test.connections.listSessions("m1") }
        compose.awaitText("The list may be out of date: listing timed out")
        compose.onNodeWithText("build").assertExists()
        compose.onNodeWithText("Retry").performClick()
        gone("The list may be out of date: listing timed out")
        compose.onNodeWithText("build").performClick()
        compose.waitUntil(ROUTE_WAIT_MS) { opened.isNotEmpty() }
        assertEquals(SessionKey("m1", "s1"), opened.single())
    }
}
