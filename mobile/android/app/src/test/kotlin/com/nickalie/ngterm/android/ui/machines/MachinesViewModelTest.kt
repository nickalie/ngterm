package com.nickalie.ngterm.android.ui.machines

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.fakeSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class MachinesViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    // the test's view model lives here, so tearing down stops its collectors before Dispatchers.Main is reset
    private val models = ViewModelStore()

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() {
        models.clear()
        test.close()
        Dispatchers.resetMain()
    }

    private fun model(): MachinesViewModel =
        ViewModelProvider(models, viewModelFactory { initializer { MachinesViewModel(test.graph) } })[
            MachinesViewModel::class.java,
        ]

    private fun MachinesViewModel.ready(predicate: (MachinesUiState.Ready) -> Boolean = { true }) =
        runBlocking { state.first { it is MachinesUiState.Ready && predicate(it) } as MachinesUiState.Ready }

    // what the view model does after a store's answer lands on the store's thread: wait for it
    private fun eventually(check: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!check()) {
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(10)
        }
    }

    private fun connects() = test.core.calls.count { it.method == "connect" }

    private fun MachinesUiState.Ready.link(id: String) = items.first { it.machine.id == id }.link

    @Test
    fun theFirstReadConnectsEveryMachineAndShowsItsSessions() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        test.seedMachine(testMachine(id = "m2", label = "studio", host = "studio.lan", keyId = key.id))
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1"))
        val model = model()
        val ready = model.ready { it.items.all { item -> item.link == Link.Connected && item.sessions != null } }
        assertEquals(listOf("build-01", "studio"), ready.items.map { it.machine.title })
        assertEquals(listOf(fakeSession("s1")), ready.items[0].sessions)
        assertEquals(2, test.core.calls.count { it.method == "connect" })
    }

    @Test
    fun aSavedMachineIsStoredAndConnected() {
        val key = test.seedKey()
        val model = model()
        model.ready { it.items.isEmpty() }
        model.save(MachineDraft(host = "build-01.lan", user = "nick", keyId = key.id), editing = null)
        val ready = model.ready { it.items.singleOrNull()?.link == Link.Connected }
        val machine = ready.items.single().machine
        assertEquals("nick@build-01.lan:22", machine.address)
        // edited: the core gets the new record and connects it anew
        model.save(MachineDraft.of(machine).copy(port = "2222"), editing = machine.id)
        model.ready {
            it.items
                .single()
                .machine.port == 2222
        }
        eventually { connects() == 2 }
    }

    @Test
    fun removingAMachineForgetsItAndItsPinUntilUndone() {
        val key = test.seedKey()
        test.seedMachine(testMachine(id = "a", keyId = key.id))
        test.seedMachine(testMachine(id = "b", label = "studio", keyId = key.id))
        runBlocking { test.pins.set("a", "SHA256:a") }
        val model = model()
        model.ready { it.items.size == 2 }
        model.remove("a")
        model.ready { it.items.map { item -> item.machine.id } == listOf("b") }
        // the undo is offered once the removal is done
        eventually { model.removed.value != null }
        assertEquals(
            "a",
            model.removed.value
                ?.machine
                ?.id,
        )
        assertNull(test.pins.pinned("a"))
        model.undoRemove()
        model.ready { it.items.map { item -> item.machine.id } == listOf("a", "b") }
        assertEquals("SHA256:a", test.pins.pinned("a"))
        eventually { connects() == 3 }
        assertNull(model.removed.value)
    }

    @Test
    fun aFirstConnectAsksAboutTheHostKeyAndTrustingItPinsIt() {
        val key = test.seedKey()
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        test.seedMachine(testMachine(keyId = key.id))
        val model = model()
        val asking = model.ready { it.link("m1") is Link.HostKeyUnknown }
        assertEquals(
            "SHA256:new",
            asking
                .hostKeyQuestions()
                .single()
                .second.fingerprint,
        )
        model.answerHostKey("m1", "SHA256:new", trust = true)
        model.ready { it.link("m1") == Link.Connected }
        assertEquals("SHA256:new", test.pins.pinned("m1"))
        // a later connect (Retry now, or Edit saved unchanged) sends the same record, so the core keeps the connection
        model.connect("m1")
        eventually { connects() == 2 }
        val records =
            test.core.calls
                .filter { it.method == "connect" }
                .map { it.args.single() }
        assertEquals(records[0], records[1])
    }

    @Test
    fun aChangedHostKeyStopsUntilThePinIsRemovedThenAsksAboutTheNewKey() {
        val key = test.seedKey()
        runBlocking { test.pins.set("m1", "SHA256:old") }
        test.core.hostKeys["build-01.lan"] = "SHA256:new"
        test.seedMachine(testMachine(keyId = key.id))
        val model = model()
        model.ready { it.link("m1") == Link.HostKeyChanged("SHA256:old", "SHA256:new") }
        assertEquals("SHA256:old", test.pins.pinned("m1"))
        model.removePin("m1")
        model.ready { it.link("m1") == Link.HostKeyUnknown("SHA256:new") }
        assertNull(test.pins.pinned("m1"))
    }

    @Test
    fun theDeviceGoingOfflineShowsOnTheList() {
        test.seedKey()
        val model = model()
        model.ready { !it.offline }
        test.online.value = false
        model.ready { it.offline }
    }
}
