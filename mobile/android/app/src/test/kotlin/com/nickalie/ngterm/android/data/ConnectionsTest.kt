package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.HOST_KEY_CHANGED
import com.nickalie.ngterm.core.fake.SIGN_IN_UNFINISHED
import com.nickalie.ngterm.core.fake.TailscaleSignIn
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancelChildren
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
    private val machine = Machine("m1", "build-01", "build-01.lan", 22u, "nick", "k1")
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
    fun aConnectCancelledBySuspendIsListedWhenTheResumedConnectionComes() =
        runTest {
            val connections = connections()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            core.failNext("connect", MobileException.Cancelled())
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(null, connections.trees.value["m1"])
            // the resume's connect has no caller in the app: the core only reports it (the fake connects as it would)
            core.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
        }

    @Test
    fun aConnectCancelledAfterTheResumedConnectionReportedIsStillListed() =
        runTest {
            val connections = connections()
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            core.failNext("connect", MobileException.Cancelled())
            val gate = CompletableDeferred<Unit>()
            core.holds["connect"] = gate
            connections.connect(machine)
            runCurrent()
            // the resume connects and reports Connected while the cancelled call has yet to return to the app
            core.connect(machine)
            runCurrent()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(null, connections.trees.value["m1"])
            gate.complete(Unit)
            advanceUntilIdle()
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
            assertEquals(1, core.calls.count { it.method == "listSessions" })
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
    fun aSignInShowsWhileTheConnectWaitsAndTheConnectedStateEndsIt() =
        runTest {
            val connections = connections()
            val signingIn = mutableListOf<Boolean>()
            launch { connections.signingIn.collect { signingIn += it } }
            core.signIns["build-01.lan"] = TailscaleSignIn
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.SignIn(TailscaleSignIn.message, TailscaleSignIn.url), connections.links.value["m1"])
            // connecting again as it is joins the waiting connect: the sign-in stays
            connections.connect(machine)
            assertEquals(Link.SignIn(TailscaleSignIn.message, TailscaleSignIn.url), connections.links.value["m1"])
            core.finishSignIn("m1")
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listOf(false, true, false), signingIn)
            coroutineContext.cancelChildren()
        }

    @Test
    fun aSignInNotFinishedIsUnreachableAndADisconnectTakesItAway() =
        runTest {
            val connections = connections()
            core.signIns["build-01.lan"] = TailscaleSignIn
            connections.connect(machine)
            advanceUntilIdle()
            core.finishSignIn("m1", signedIn = false)
            advanceUntilIdle()
            assertEquals(Link.Unreachable(SIGN_IN_UNFINISHED), connections.links.value["m1"])
            connections.connect(machine)
            advanceUntilIdle()
            assertEquals(Link.SignIn(TailscaleSignIn.message, TailscaleSignIn.url), connections.links.value["m1"])
            connections.disconnect("m1")
            advanceUntilIdle()
            assertEquals(null, connections.links.value["m1"])
            assertEquals(emptySet<String>(), core.waitingForSignIn)
        }

    @Test
    fun anEditedMachineWaitingForASignInConnectsAnew() =
        runTest {
            val connections = connections()
            core.signIns["build-01.lan"] = TailscaleSignIn
            connections.connect(machine)
            advanceUntilIdle()
            connections.connect(machine.copy(host = "build-02.lan"))
            assertEquals(Link.Connecting, connections.links.value["m1"])
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            // the overtaken connect, which the real core cancels, is let go here
            core.finishSignIn("m1")
            advanceUntilIdle()
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
    fun aListingDroppedWhileReconnectingIsRedoneOnceTheMachineIsBack() =
        runTest {
            val connections = connections()
            core.failNext("listSessions", MobileException.Remote("build-01 gave no answer"))
            val gate = CompletableDeferred<Unit>().also { core.holds["listSessions"] = it }
            connections.connect(machine)
            runCurrent()
            // the connection drops before the first listing answers, which then fails while it is down
            core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(4_000u)))
            gate.complete(Unit)
            advanceUntilIdle()
            assertEquals(null, connections.treeFailures.value["m1"])
            assertEquals(null, connections.trees.value["m1"])
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
            // a Connected that changes nothing lists nothing more
            val listings = core.calls.count { it.method == "listSessions" }
            core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
            advanceUntilIdle()
            assertEquals(listings, core.calls.count { it.method == "listSessions" })
        }

    @Test
    fun aReconnectThatHasToSignInAgainListsTheMachineOnceItIsBack() =
        runTest {
            val connections = connections()
            connections.connect(machine)
            advanceUntilIdle()
            // the connection drops, and the core's reconnect meets a check that ran out while it was down
            core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(4_000u)))
            core.emit(MobileEvent.SignInRequested("m1", TailscaleSignIn.message, TailscaleSignIn.url))
            assertEquals(
                Link.SignIn(TailscaleSignIn.message, TailscaleSignIn.url, reconnecting = true),
                connections.links.value["m1"],
            )
            // a second link while it still waits keeps it a reconnect
            core.emit(MobileEvent.SignInRequested("m1", TailscaleSignIn.message, TailscaleSignIn.url))
            assertEquals(true, (connections.links.value["m1"] as Link.SignIn).reconnecting)
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            val listings = core.calls.count { it.method == "listSessions" }
            core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
            advanceUntilIdle()
            assertEquals(Link.Connected, connections.links.value["m1"])
            assertEquals(listings + 1, core.calls.count { it.method == "listSessions" })
            assertEquals(listOf(fakeSession("s1")), connections.trees.value["m1"])
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
            // renamed: the connection and the attachment stay
            connections.connect(machine.copy(label = "build-01 (lab)"))
            advanceUntilIdle()
            assertEquals(setOf("s1"), connections.attached.value["m1"])
            assertEquals(0, core.calls.count { it.method == "detachSession" })
            // edited: the core replaces the connection and ends the streams; the new listing still has s1
            connections.connect(machine.copy(label = "build-01 (lab)", port = 2222u))
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
