package com.nickalie.ngterm.android.ui.machines

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextReplacement
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.lifecycle.ViewModelProvider
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.FAKE_PUBLIC_KEY
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitDescription
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.AppKeys
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.testStore
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

private const val ROUTE_WAIT_MS = 10_000L

/** [inner], whose readers wait while [open] is false: a store read slower than the others. */
private class GatedStore(
    private val inner: DataStore<Preferences>,
    private val open: StateFlow<Boolean>,
) : DataStore<Preferences> {
    override val data: Flow<Preferences> =
        flow {
            open.first { it }
            emitAll(inner.data)
        }

    override suspend fun updateData(transform: suspend (Preferences) -> Preferences) = inner.updateData(transform)
}

/** Navigation and page lifetime over real stores and view models, with the core faked at FFI. */
@RunWith(RobolectricTestRunner::class)
class MachinesRouteTest {
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

    private fun route(onOpen: (SessionKey) -> Unit = {}) {
        compose.setContent { NgtermTheme { MachinesRoute(test.graph, onOpenSession = onOpen) } }
    }

    private fun seed() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
    }

    private fun edit() {
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNodeWithText("Edit").performClick()
        compose.awaitText("Edit machine")
    }

    private fun back() {
        compose.runOnIdle { compose.activity.onBackPressedDispatcher.onBackPressed() }
    }

    private fun gone(text: String) {
        compose.waitUntil(ROUTE_WAIT_MS) { compose.onAllNodesWithText(text).fetchSemanticsNodes().isEmpty() }
    }

    @Test
    fun aPublicKeyRoundTripAndRestorationKeepTheDraftAndClosingStartsTheNextAddFresh() {
        test.seedKey()
        val restoration = StateRestorationTester(compose)
        restoration.setContent { NgtermTheme { MachinesRoute(test.graph) } }
        compose.awaitText("Add machine")
        compose.onNodeWithText("Add machine").performClick()
        compose.onNode(hasText("Name") and hasSetTextAction()).performTextReplacement("unsaved")
        compose.onNodeWithText("Show public key").performScrollTo().performClick()
        compose.awaitText("Add this key to your machines")
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertExists()
        restoration.emulateSavedInstanceStateRestore()
        compose.onNodeWithText("Add this key to your machines").assertExists()
        compose.onNodeWithContentDescription("Back").performClick()
        compose.onNodeWithText("unsaved").performScrollTo().assertExists()
        back()
        compose.awaitText("Add machine")
        compose.onNodeWithText("Add machine").performClick()
        compose.onNodeWithText("unsaved").assertDoesNotExist()
        compose.onNodeWithContentDescription("Close").performClick()
        compose.awaitText("No machines yet")
    }

    @Test
    fun aRestoredPublicKeyPageWaitsForTheKeysToLoad() {
        val keysOpen = MutableStateFlow(true)
        val base = test.graph
        val keys =
            AppKeys(GatedStore(testStore(dir.root, "gated-keys"), keysOpen), test.vault, test.maker, "ngterm@test")
        val graph =
            AppGraph(
                base.machines,
                keys,
                base.pins,
                base.connections,
                base.clipboardReads,
                base.panes,
                base.chats,
                base.asks,
                base.notificationPrefs,
                base.settings,
                base.opens,
                base.online,
            )
        runBlocking { keys.generate() }
        val restoration = StateRestorationTester(compose)
        restoration.setContent { NgtermTheme { MachinesRoute(graph) } }
        compose.awaitText("Add machine")
        compose.onNodeWithText("Add machine").performClick()
        compose.onNodeWithText("Show public key").performScrollTo().performClick()
        compose.awaitText("Add this key to your machines")
        // A process restoration makes fresh view models; here the machines are read before the keys are.
        keysOpen.value = false
        compose.runOnIdle { compose.activity.viewModelStore.clear() }
        restoration.emulateSavedInstanceStateRestore()
        val viewModel = ViewModelProvider(compose.activity)[MachinesViewModel::class.java]
        compose.waitUntil(ROUTE_WAIT_MS) { viewModel.state.value !is MachinesUiState.Loading }
        compose.waitForIdle()
        keysOpen.value = true
        compose.awaitText("Add this key to your machines")
        compose.onNodeWithText(FAKE_PUBLIC_KEY).assertExists()
    }

    @Test
    fun removingTheMachineElsewhereClosesItsEditPage() {
        seed()
        route()
        compose.awaitDescription("Connected")
        edit()
        runBlocking { test.graph.machines.delete("m1") }
        compose.awaitText("No machines yet")
        compose.onNodeWithText("Edit machine").assertDoesNotExist()
    }

    @Test
    fun restoringAnEditWaitsForTheMachinesToLoadAndKeepsTheUnsavedDraft() {
        seed()
        val restoration = StateRestorationTester(compose)
        restoration.setContent { NgtermTheme { MachinesRoute(test.graph) } }
        compose.awaitDescription("Connected")
        edit()
        compose.onNode(hasText("Name") and hasSetTextAction()).performTextReplacement("unsaved rename")
        // A process restoration has saved fields and routes, but must make fresh view models and read the stores.
        compose.runOnIdle { compose.activity.viewModelStore.clear() }
        restoration.emulateSavedInstanceStateRestore()
        compose.awaitText("Edit machine")
        compose.onNodeWithText("unsaved rename").assertExists()
        compose.onNodeWithContentDescription("Close").performClick()
        compose.awaitText("build-01")
        compose.onNodeWithText("unsaved rename").assertDoesNotExist()
    }

    @Test
    fun aChangedKeyAnsweredElsewhereClosesItsReviewAndReturnsToSessions() {
        seed()
        route()
        compose.awaitDescription("Connected")
        compose.onNodeWithText("build-01").performClick()
        compose.awaitText("No sessions on build-01")
        compose.runOnIdle { test.core.emit(MobileEvent.HostKeyChanged("m1", "SHA256:old", "SHA256:new")) }
        compose.awaitText("Review")
        compose.onNodeWithText("Review").performClick()
        compose.awaitText("build-01's host key changed")
        compose.runOnIdle { test.core.emit(MobileEvent.MachineState("m1", MachineState.Connected)) }
        compose.awaitText("No sessions on build-01")
        compose.onNodeWithText("build-01's host key changed").assertDoesNotExist()
        back()
        compose.awaitText("Add machine")
    }

    @Test
    fun aRemovedMachineClosesItsChangedKeyPage() {
        val key = test.seedKey()
        runBlocking { test.pins.set("m1", "SHA256:old") }
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        test.seedMachine(testMachine(keyId = key.id))
        route()
        compose.awaitDescription("Host key changed")
        compose.onNodeWithText("Review").performClick()
        compose.awaitText("build-01's host key changed")
        runBlocking { test.graph.machines.delete("m1") }
        compose.awaitText("No machines yet")
        compose.onNodeWithText("build-01's host key changed").assertDoesNotExist()
    }

    @Test
    fun aKeyThatIsNoLongerAvailableReturnsToTheForm() {
        test.seedKey()
        test.seedMachine(testMachine(keyId = "removed-key"))
        test.core.failNext("connect", MobileException.Unreachable("key no longer available"))
        route()
        compose.awaitText("key no longer available")
        edit()
        compose.onNodeWithText("Show public key").performScrollTo().performClick()
        compose.awaitText("Edit machine")
        compose.onNodeWithText("Add this key to your machines").assertDoesNotExist()
        compose.onNodeWithContentDescription("Close").performClick()
        compose.awaitText("Add machine")
    }

    @Test
    fun disconnectAndConnectReachTheCoreAndOpeningASessionForwardsItsIdentity() {
        seed()
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "build"))
        var opened: SessionKey? = null
        route { opened = it }
        compose.awaitDescription("Connected")
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNodeWithText("Disconnect").performClick()
        compose.awaitDescription("Offline")
        assertEquals(false, "m1" in test.core.connected)
        compose.onNodeWithText("Connect").performClick()
        compose.awaitDescription("Connected")
        compose.onNodeWithText("build-01").performClick()
        compose.awaitText("build")
        compose.onNodeWithText("build").performClick()
        compose.waitUntil(ROUTE_WAIT_MS) { opened != null }
        assertEquals(SessionKey("m1", "s1"), opened)
    }

    @Test
    fun anExpiredRemovalCannotBeUndoneAndDoesNotReturnWhenTheDestinationComesBack() {
        seed()
        // a destination switch takes the route out of composition; its activity-scoped view model stays
        var shown by mutableStateOf(true)
        compose.setContent { NgtermTheme { if (shown) MachinesRoute(test.graph) } }
        compose.awaitDescription("Connected")
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNodeWithText("Remove").performClick()
        compose.awaitText("Removed build-01")
        compose.mainClock.advanceTimeBy(ROUTE_WAIT_MS)
        gone("Undo")
        val viewModel = ViewModelProvider(compose.activity)[MachinesViewModel::class.java]
        compose.waitUntil(ROUTE_WAIT_MS) { viewModel.removed.value == null }
        compose.runOnIdle { shown = false }
        compose.runOnIdle { shown = true }
        compose.awaitText("No machines yet")
        compose.onNodeWithText("Removed build-01").assertDoesNotExist()
        compose.onNodeWithText("Undo").assertDoesNotExist()
        compose.onNodeWithText("build-01").assertDoesNotExist()
        assertNull(runBlocking { test.graph.machines.get("m1") })
    }
}
