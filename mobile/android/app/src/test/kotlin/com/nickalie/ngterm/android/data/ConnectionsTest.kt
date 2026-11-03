package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.HOST_KEY_CHANGED
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
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
}
