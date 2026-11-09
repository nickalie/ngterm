package com.nickalie.ngterm.android.ui.machines

import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitDescription
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.platform.PinnedHostKeys
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

// how long a blocking read may take before the test calls it stuck
private const val BLOCKING_READ_WAIT_S = 30L

/** The machine flows end to end on the fake core: add, the first connect's host key, a changed key, removal. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp")
class FirstConnectTest {
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
            NgtermTheme {
                NgtermApp(
                    graph = test.graph,
                    coreVersion = "1.2.3",
                    layout = NavLayout.Bar,
                )
            }
        }
    }

    @Test
    fun addMachineThenConfirmItsFingerprintAndItIsConnected() {
        test.seedKey()
        test.core.hostKeys["build-01.lan"] = "SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E"
        app()
        compose.awaitText("Add machine")
        compose.onNodeWithText("Add machine").performClick()
        compose.onNodeWithText("Save").performClick()
        // Save with empty fields only shows what is missing
        compose.onNodeWithText("Enter the machine's hostname or IP address.").assertExists()
        compose.onNodeWithText("Name").performTextInput("build-01")
        compose.onNodeWithText("Host").performTextInput("build-01.lan")
        compose.onNodeWithText("User").performTextInput("nick")
        compose.onNodeWithText("Save").performClick()
        compose.awaitText("Trust build-01?")
        compose.onNodeWithText("SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E").assertExists()
        compose.onNodeWithText("Trust and connect").performClick()
        compose.awaitDescription("Connected")
        compose.onNodeWithText("nick@build-01.lan:22").assertExists()
        assertEquals("SHA256:q0Gm4d1Xb7pKc2yT9wVZ5eRj8sHnLfA3uMiO6tYvB1E", test.pins.pinned(machineId()))
    }

    @Test
    fun decliningTheFingerprintLeavesTheMachineNotConnected() {
        val key = test.seedKey()
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        test.seedMachine(testMachine(keyId = key.id))
        app()
        compose.awaitText("Trust build-01?")
        compose.onNodeWithText("Don't connect").performClick()
        compose.awaitText("Connect")
        compose.onNodeWithContentDescription("Offline").assertExists()
        assertNull(test.pins.pinned("m1"))
    }

    @Test
    fun aChangedHostKeyIsAHardStopUntilThePinIsDeliberatelyRemoved() {
        val key = test.seedKey()
        runBlocking { test.pins.set("m1", "SHA256:old") }
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        test.seedMachine(testMachine(keyId = key.id))
        app()
        compose.awaitDescription("Host key changed")
        compose.onNodeWithText("Review").performClick()
        compose.onNodeWithText("build-01's host key changed").assertExists()
        compose.onNodeWithText("SHA256:old").assertExists()
        compose.onNodeWithText("SHA256:new").assertExists()
        // the safe way out is the main one and changes nothing
        compose.onNodeWithText("Back to machines").performClick()
        compose.awaitDescription("Host key changed")
        assertEquals("SHA256:old", test.pins.pinned("m1"))
        compose.onNodeWithText("Review").performClick()
        compose.onNodeWithText("Remove pinned key").performClick()
        compose.awaitText("Remove the pinned key of build-01?")
        compose.onNodeWithText("Cancel").performClick()
        assertEquals("SHA256:old", test.pins.pinned("m1"))
        compose.onNodeWithText("Remove pinned key").performClick()
        compose.onNode(hasText("Remove pinned key") and hasAnyAncestor(isDialog())).performClick()
        compose.awaitText("Trust build-01?")
        assertNull(test.pins.pinned("m1"))
        compose.onNodeWithText("Trust and connect").performClick()
        compose.awaitDescription("Connected")
        assertEquals("SHA256:new", test.pins.pinned("m1"))
    }

    @Test
    fun aRemovedMachineComesBackWithUndo() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        app()
        compose.awaitDescription("Connected")
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNodeWithText("Remove").performClick()
        compose.awaitText("Removed build-01")
        compose.onNodeWithText("Undo").performClick()
        compose.awaitDescription("Connected")
        compose.onNodeWithText("build-01").assertExists()
    }

    @Test
    fun anEditKeepsTheMachineAndShowsItsPin() {
        val key = test.seedKey()
        runBlocking { test.pins.set("m1", "SHA256:pinned") }
        test.seedMachine(testMachine(keyId = key.id))
        app()
        compose.awaitDescription("Connected")
        compose.onNodeWithContentDescription("More options for build-01").performClick()
        compose.onNodeWithText("Edit").performClick()
        compose.onNodeWithText("Edit machine").assertExists()
        compose.onNodeWithText("SHA256:pinned").assertExists()
        compose.onNodeWithText("build-01.lan").performTextReplacement("build-02.lan")
        compose.onNodeWithText("Save").performClick()
        compose.awaitText("nick@build-02.lan:22")
    }

    @Test
    fun aBlockingPinReadDoesNotWaitForTheThreadCollectingAllPins() = assertBlockingReadCompletes { it.all.first() }

    @Test
    fun aBlockingPinReadDoesNotWaitForTheThreadGettingAPin() = assertBlockingReadCompletes { it.get("m1") }

    // The fake core reads a pin on the main thread. The app's first read of the store may hold DataStore's
    // first-read lock by then, and if it has to come back to the main thread to let go of it, the blocked thread waits
    // for itself. Holding the store's worker forces this interleaving if the first reader stays on main.
    private fun assertBlockingReadCompletes(firstRead: suspend (PinnedHostKeys) -> Unit) {
        val worker = Executors.newSingleThreadExecutor()
        val storeScope = CoroutineScope(worker.asCoroutineDispatcher() + SupervisorJob())
        val homeScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val store = PreferenceDataStoreFactory.create(scope = storeScope) { File(dir.root, "held.preferences_pb") }
        val pins = PinnedHostKeys(store)
        val main = Thread.currentThread()
        val answered = CountDownLatch(1)
        homeScope.launch { firstRead(pins) }
        // With a reader on main, its first step runs on the worker and queues its continuation for this thread.
        worker.submit {}.get()
        val hold = CountDownLatch(1)
        worker.execute { hold.await() }
        // Such a main-thread reader resumes here, taking the lock and queuing its file read behind the hold.
        shadowOf(Looper.getMainLooper()).idle()
        hold.countDown()
        val watchdog =
            thread(isDaemon = true) {
                if (!answered.await(BLOCKING_READ_WAIT_S, TimeUnit.SECONDS)) main.interrupt()
            }
        try {
            assertNull(pins.pinned("m1"))
        } catch (_: InterruptedException) {
            fail("the blocking pin read waited for the thread it had blocked")
        } finally {
            answered.countDown()
            watchdog.join()
            Thread.interrupted()
            homeScope.cancel()
            storeScope.cancel()
            worker.shutdown()
        }
    }

    private fun machineId(): String =
        runBlocking {
            test.graph.machines.machines
                .first()
                .single()
                .id
        }
}
