package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.HOST_KEY_CHANGED
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

private class MemoryPins : HostKeyStore {
    val pins = mutableMapOf<String, String>()

    override fun pinned(machineId: String) = pins[machineId]

    override fun pin(
        machineId: String,
        fingerprint: String,
    ) {
        pins[machineId] = fingerprint
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class ConnectionsTest {
    private val machine = Machine("m1", "build-01", "build-01.lan", 22u, "nick", "k1", null)
    private val pins = MemoryPins()
    private lateinit var core: FakeMobileCore
    private var now = 5_000L

    private fun TestScope.connections() =
        Connections(this, { now }) { sink -> FakeMobileCore(sink, pins).also { core = it } }

    @Test
    fun aConnectGoesFromConnectingToConnectedAndListsTheSessions() =
        runTest {
            val connections = connections()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            connections.connect(machine)
            assertEquals(Link.Connecting, connections.links.value["m1"])
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
            connections.disconnect("m1")
            assertEquals(null, connections.links.value["m1"])
            assertEquals(null, connections.trees.value["m1"])
        }

    @Test
    fun aFailedConnectIsUnreachableWithTheCoresReason() =
        runTest {
            val connections = connections()
            core.failNext("connect", MobileException.Unreachable("build-01.lan:22: connection refused"))
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.Unreachable("build-01.lan:22: connection refused"), connections.links.value["m1"])
        }

    @Test
    fun anUnknownHostKeyWaitsForTheAnswerAndTrustingItConnects() =
        runTest {
            val connections = connections()
            core.hostKeys["build-01.lan"] = "SHA256:new"
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.HostKeyUnknown("SHA256:new"), connections.links.value["m1"])
            connections.answerHostKey("m1", "SHA256:new", trust = true)
            assertEquals(Link.Connecting, connections.links.value["m1"])
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(mapOf("m1" to "SHA256:new"), pins.pins)
        }

    @Test
    fun aDeclinedHostKeyLeavesTheMachineOfflineNotFailed() =
        runTest {
            val connections = connections()
            core.hostKeys["build-01.lan"] = "SHA256:new"
            connections.connect(machine)
            advanceUntilIdle()
            connections.answerHostKey("m1", "SHA256:new", trust = false)
            advanceUntilIdle()
            assertEquals(null, connections.links.value["m1"])
            assertEquals(emptyMap<String, String>(), pins.pins)
        }

    @Test
    fun aChangedHostKeyStaysChangedThroughTheFailureThatFollowsIt() =
        runTest {
            val connections = connections()
            pins.pins["m1"] = "SHA256:old"
            core.hostKeys["build-01.lan"] = "SHA256:new"
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.HostKeyChanged("SHA256:old", "SHA256:new"), connections.links.value["m1"])
        }

    @Test
    fun aReconnectCountsDownFromTheClock() =
        runTest {
            val connections = connections()
            connections.connect(machine)
            advanceUntilIdle()
            core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(4_000u)))
            assertEquals(Link.Reconnecting(9_000L), connections.links.value["m1"])
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable(HOST_KEY_CHANGED)))
            assertEquals(Link.Unreachable(HOST_KEY_CHANGED), connections.links.value["m1"])
        }

    @Test
    fun aFailedListingIsKeptUntilOneLands() =
        runTest {
            val connections = connections()
            core.failNext("listSessions", MobileException.Remote("/bin/sh: 1: ngtermctl: not found"))
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals("/bin/sh: 1: ngtermctl: not found", connections.treeFailures.value["m1"])
            assertEquals(null, connections.trees.value["m1"])
            connections.listSessions("m1")
            assertEquals(null, connections.treeFailures.value["m1"])
            assertEquals(emptyList<Any>(), connections.trees.value["m1"])
            // a machine that isn't connected fails no listing: its link says why
            connections.disconnect("m1")
            connections.listSessions("m1")
            assertEquals(null, connections.treeFailures.value["m1"])
        }

    @Test
    fun anAttachedSessionIsLetGoWhenItEndsLeavesTheListingOrTheMachineDisconnects() =
        runTest {
            val connections = connections()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"), fakeSession("s2"), fakeSession("s3"))
            connections.connect(machine)
            advanceUntilIdle()
            connections.attach("m1", "s1")
            connections.attach("m1", "s2")
            assertEquals(setOf("s1", "s2"), connections.attached.value["m1"])
            connections.endSession("m1", "s1")
            assertEquals(setOf("s2"), connections.attached.value["m1"])
            // ended elsewhere: the next listing doesn't have it
            core.sessions["m1"]!!.removeAll { it.id == "s2" }
            connections.listSessions("m1")
            assertEquals(null, connections.attached.value["m1"])
            val detached = core.calls.filter { it.method == "detachSession" }.map { it.args[1] }
            assertEquals(listOf("s1", "s2"), detached)
            connections.attach("m1", "s3")
            connections.disconnect("m1")
            assertEquals(null, connections.attached.value["m1"])
        }

    @Test
    fun aFailedListingOvertakenByANewerOneIsDropped() =
        runTest {
            val connections = connections()
            connections.connect(machine)
            advanceUntilIdle()
            // the older listing fails only after a newer one has landed
            core.failNext("listSessions", MobileException.Remote("build-01 gave no answer"))
            val gate = CompletableDeferred<Unit>().also { core.holds["listSessions"] = it }
            launch { connections.listSessions("m1") }
            runCurrent()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            connections.listSessions("m1")
            gate.complete(Unit)
            advanceUntilIdle()
            assertEquals(null, connections.treeFailures.value["m1"])
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
        }

    @Test
    fun anEditedMachineLetsGoOfItsAttachedSessions() =
        runTest {
            val connections = connections()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"), fakeSession("s2"))
            connections.connect(machine)
            advanceUntilIdle()
            connections.attach("m1", "s1")
            // connected again as it is: the attachment stays
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(setOf("s1"), connections.attached.value["m1"])
            // edited: the core replaces the connection and ends the streams; the new listing still has s1
            connections.connect(machine.copy(label = "build-01 (lab)"))
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listOf("s1", "s2"), connections.trees.value["m1"]?.map { it.id })
            assertEquals(null, connections.attached.value["m1"])
            assertEquals(listOf("s1"), core.calls.filter { it.method == "detachSession" }.map { it.args[1] })
        }

    @Test
    fun createAndEndAreTheCoresAndABlankNameIsNone() =
        runTest {
            val connections = connections()
            connections.connect(machine)
            advanceUntilIdle()
            val created = connections.createSession("m1", "  ")
            assertEquals(listOf(created), connections.trees.value["m1"])
            assertEquals(null, core.calls.last { it.method == "createSession" }.args[1])
            connections.createSession("m1", " api ")
            assertEquals("api", core.calls.last { it.method == "createSession" }.args[1])
            connections.endSession("m1", created.id)
            assertEquals(listOf("api"), connections.trees.value["m1"]?.map { it.name })
        }
}
