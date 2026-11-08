package com.nickalie.ngterm.android.ui.machines

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.testMachine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File

/** Real storage failures and stale actions on the Machines home, over the native-core fake only. */
@RunWith(AndroidJUnit4::class)
@OptIn(ExperimentalCoroutinesApi::class)
class MachinesRecoveryTest {
    @get:Rule val dir = TemporaryFolder()
    private lateinit var test: TestGraph
    private val models = ViewModelStore()

    @Before fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        test = TestGraph(dir.root)
    }

    @After fun tearDown() {
        models.clear()
        test.close()
        Dispatchers.resetMain()
    }

    private fun model(): MachinesViewModel =
        ViewModelProvider(
            models,
            viewModelFactory { initializer { MachinesViewModel(test.graph) } },
        )[MachinesViewModel::class.java]

    private fun <T> StateFlow<T>.await(predicate: (T) -> Boolean): T =
        runBlocking { withTimeout(WAIT_MS) { first(predicate) } }

    private fun MachinesViewModel.ready() = state.await { it is MachinesUiState.Ready } as MachinesUiState.Ready

    @Test
    fun anUnreadableMachineStoreShowsItsReasonAndRetryReadsItAgain() {
        // A directory where DataStore expects a file: an actual file-system read error.
        val blocked = File(dir.root, "machines.preferences_pb")
        assertTrue(blocked.mkdir())
        val model = model()
        val failure = model.state.await { it is MachinesUiState.Failed } as MachinesUiState.Failed
        assertTrue(failure.reason, failure.reason.startsWith("Inoperable file:"))
        assertTrue(failure.reason, failure.reason.contains(blocked.canonicalPath))
        assertTrue(failure.hostKeyQuestions().isEmpty())
        assertTrue(failure.signIns().isEmpty())
        assertTrue(test.core.calls.none { it.method == "connect" })
        assertTrue(blocked.delete())
        model.retry()
        assertEquals(emptyList<MachineItem>(), model.ready().items)
    }

    @Test
    fun retryingTheInitialReadConnectsMachinesFromTheFirstSuccessfulList() {
        val blocked = File(dir.root, "machines.preferences_pb")
        assertTrue(blocked.mkdir())
        val model = model()
        model.state.await { it is MachinesUiState.Failed }
        assertTrue(blocked.delete())
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        model.retry()
        model.state.await { (it as? MachinesUiState.Ready)?.items?.size == 1 }
        test.connections.links.await { it["m1"] == Link.Connected }
    }

    @Test
    fun removingAnUnpinnedMachineCanBeUndoneOnceAndAnExpiredUndoDoesNothing() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        val model = model()
        model.state.await { (it as? MachinesUiState.Ready)?.items?.singleOrNull()?.link == Link.Connected }
        model.remove("m1")
        model.removed.await { it != null }
        assertNull(model.removed.value?.pin)
        model.undoRemove()
        model.state.await { (it as? MachinesUiState.Ready)?.items?.singleOrNull()?.link == Link.Connected }
        assertNull(test.pins.pinned("m1"))
        assertNull(model.removed.value)
        model.remove("m1")
        model.removed.await { it != null }
        model.forgetRemoval()
        model.undoRemove()
        assertNull(model.removed.value)
        assertNull(runBlocking { test.graph.machines.get("m1") })
    }

    @Test
    fun staleActionsForARemovedMachineDoNotAffectTheNextRemovalOrUndo() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        runBlocking { test.pins.set("gone", "SHA256:old") }
        val model = model()
        model.state.await { (it as? MachinesUiState.Ready)?.items?.singleOrNull()?.link == Link.Connected }
        model.pins.await { it["gone"] == "SHA256:old" }
        model.removePin("gone")
        model.pins.await { "gone" !in it }
        model.connect("gone")
        model.remove("gone")
        model.remove("m1")
        val removal = model.removed.await { it != null }
        assertEquals("m1", removal?.machine?.id)
        assertNull(runBlocking { test.graph.machines.get("m1") })
        model.undoRemove()
        model.state.await { (it as? MachinesUiState.Ready)?.items?.singleOrNull()?.link == Link.Connected }
        assertEquals(
            listOf("m1"),
            runBlocking {
                test.graph.machines.machines
                    .first()
            }.map { it.id },
        )
        val connected = test.core.calls.filter { it.method == "connect" }
        assertTrue(connected.all { it.args == listOf(testMachine(keyId = key.id).toCore()) })
        assertNull(test.pins.pinned("gone"))
    }

    private companion object {
        const val WAIT_MS = 10_000L
    }
}
