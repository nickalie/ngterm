package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class FakeMobileCoreTest {
    private val machine =
        Machine(
            id = "m1",
            label = "box",
            host = "box.example",
            port = 22u,
            user = "me",
            keyId = "k1",
            pinnedHostKey = null,
        )

    @Test
    fun createListAndEndSessionsOnAConnectedMachine() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val core = FakeMobileCore(sink = EventSinkOf(events))
            core.connect(machine)
            val session = core.createSession("m1", "work")
            assertEquals("work", session.name)
            assertEquals(listOf(session), core.listSessions("m1"))
            core.endSession("m1", session.id)
            assertTrue(core.listSessions("m1").isEmpty())
            assertEquals(
                listOf(
                    MobileEvent.MachineState("m1", MachineState.Connected),
                    MobileEvent.Tree("m1", listOf(session)),
                    MobileEvent.Tree("m1", listOf(session)),
                    MobileEvent.Tree("m1", emptyList()),
                    MobileEvent.Tree("m1", emptyList()),
                ),
                events,
            )
            assertEquals(
                listOf("connect", "createSession", "listSessions", "endSession", "listSessions"),
                core.calls.map { it.method },
            )
        }

    @Test
    fun connectReportsTheStateOnceAndDisconnectReportsNothing() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val core = FakeMobileCore(sink = EventSinkOf(events))
            core.connect(machine)
            core.connect(machine)
            core.disconnect("m1")
            assertEquals(listOf(MobileEvent.MachineState("m1", MachineState.Connected)), events)
        }

    @Test
    fun aBlankNameIsNone() =
        runTest {
            val core = FakeMobileCore()
            core.connect(machine)
            assertEquals("session 1", core.createSession("m1", " ").name)
        }

    @Test
    fun endingAnUnlistedSessionNeedsNoListingAndDeliversNoTree() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val core = FakeMobileCore(sink = EventSinkOf(events))
            core.connect(machine)
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            events.clear()
            core.endSession("m1", "s1")
            assertTrue(core.sessions.getValue("m1").isEmpty())
            assertTrue(events.isEmpty())
        }

    @Test
    fun endingASessionTheMachineDoesNotHaveIsARemoteError() =
        runTest {
            val core = FakeMobileCore()
            core.connect(machine)
            val error = runCatching { core.endSession("m1", "nope") }.exceptionOrNull()
            assertTrue(error is MobileException.Remote)
        }

    @Test
    fun attachTakesOnlyASessionOfTheLastListing() =
        runTest {
            val core = FakeMobileCore()
            core.connect(machine)
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            val unlisted = runCatching { core.attachSession("m1", "s1") }.exceptionOrNull()
            assertTrue(unlisted is MobileException.NotListed)
            core.listSessions("m1")
            core.attachSession("m1", "s1")
            core.disconnect("m1")
            core.connect(machine)
            val forgotten = runCatching { core.attachSession("m1", "s1") }.exceptionOrNull()
            assertTrue(forgotten is MobileException.NotListed)
        }

    @Test
    fun aMachineNotConnectedIsRefused() =
        runTest {
            val core = FakeMobileCore()
            val error = runCatching { core.listSessions("m1") }.exceptionOrNull()
            assertTrue(error is MobileException.NotConnected)
        }

    @Test
    fun aQueuedFailureIsThrownOnce() =
        runTest {
            val core = FakeMobileCore()
            core.failNext("connect", MobileException.Unreachable("no route"))
            val error = runCatching { core.connect(machine) }.exceptionOrNull()
            assertTrue(error is MobileException.Unreachable)
            core.connect(machine)
            assertEquals(setOf("m1"), core.connected)
        }

    @Test
    fun anUnlistedSessionIsRefused() {
        val core = FakeMobileCore()
        core.connected += "m1"
        assertThrows(MobileException.NotListed::class.java) {
            kotlinx.coroutines.runBlocking { core.attachSession("m1", "nope") }
        }
    }

    @Test
    fun aShownPaneIsHandedBackAsItIsUntilHiddenAndDetachingOrDisconnectingHidesIt() =
        runTest {
            val core = FakeMobileCore()
            core.paneFactory = { _, _, pane, _ -> FakePane(pane) }
            core.connect(machine)
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            core.listSessions("m1")
            val size = PaneSize(80u, 24u, 10u, 20u)
            val first = core.showPane("m1", "s1", "P1", size)
            assertSame(first, core.showPane("m1", "s1", "P1", size))
            core.hidePane("m1", "s1", "P1")
            assertNotSame(first, core.showPane("m1", "s1", "P1", size))
            core.showPane("m1", "s1", "P2", size)
            assertEquals(setOf(PaneRef("m1", "s1", "P1"), PaneRef("m1", "s1", "P2")), core.shownPanes)
            core.detachSession("m1", "s1")
            assertTrue(core.shownPanes.isEmpty())
            core.showPane("m1", "s1", "P1", size)
            core.disconnect("m1")
            assertTrue(core.shownPanes.isEmpty())
        }

    @Test
    fun aPaneAddedSinceTheLastListingIsUnknownUntilTheMachineIsListedAgain() =
        runTest {
            val core = FakeMobileCore()
            core.paneFactory = { _, _, pane, _ -> FakePane(pane) }
            core.connect(machine)
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            core.panes["s1"] = setOf("P1")
            core.listSessions("m1")
            val size = PaneSize(80u, 24u, 10u, 20u)
            core.showPane("m1", "s1", "P1", size)
            core.panes["s1"] = setOf("P1", "P2")
            val refused = runCatching { core.showPane("m1", "s1", "P2", size) }.exceptionOrNull()
            assertEquals(UNKNOWN_PANE, (refused as? MobileException.Remote)?.reason)
            core.listSessions("m1")
            core.showPane("m1", "s1", "P2", size)
            assertTrue(PaneRef("m1", "s1", "P2") in core.shownPanes)
        }

    @Test
    fun aFailedConnectReportsTheMachineUnreachable() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val core = FakeMobileCore(sink = EventSinkOf(events))
            core.failNext("connect", MobileException.Unreachable("no route"))
            runCatching { core.connect(machine) }
            assertEquals(listOf(MobileEvent.MachineState("m1", MachineState.Unreachable("no route"))), events)
        }

    @Test
    fun anEditedMachineConnectsAnew() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val core = FakeMobileCore(sink = EventSinkOf(events))
            core.connect(machine)
            core.connect(machine.copy(port = 2222u))
            assertEquals(2, events.count { it == MobileEvent.MachineState("m1", MachineState.Connected) })
        }

    @Test
    fun anUnknownHostKeyWaitsForTheAnswerAndTrustingItPinsIt() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val pins = MemoryHostKeys()
            val core = FakeMobileCore(sink = EventSinkOf(events), hosts = pins)
            core.hostKeys["box.example"] = "SHA256:new"
            val connect = async { core.connect(machine) }
            runCurrent()
            assertEquals(listOf(MobileEvent.HostKeyUnknown("m1", "SHA256:new")), events)
            assertEquals(mapOf("m1" to "SHA256:new"), core.waitingForHostKey)
            core.answerHostKey("m1", "SHA256:new", trust = true)
            connect.await()
            assertEquals(mapOf("m1" to "SHA256:new"), pins.pins)
            assertEquals(MobileEvent.MachineState("m1", MachineState.Connected), events.last())
            // pinned now: a later connect asks nothing
            core.disconnect("m1")
            core.connect(machine)
            assertEquals(1, events.count { it is MobileEvent.HostKeyUnknown })
        }

    @Test
    fun aRefusedOrMismatchedAnswerFailsTheConnect() =
        runTest {
            for (answer in listOf("SHA256:new" to false, "SHA256:other" to true)) {
                val events = mutableListOf<MobileEvent>()
                val pins = MemoryHostKeys()
                val core = FakeMobileCore(sink = EventSinkOf(events), hosts = pins)
                core.hostKeys["box.example"] = "SHA256:new"
                val connect = async { runCatching { core.connect(machine) } }
                runCurrent()
                core.answerHostKey("m1", answer.first, answer.second)
                val error = connect.await().exceptionOrNull()
                assertEquals(HOST_KEY_REFUSED, (error as MobileException.Unreachable).reason)
                assertEquals(MobileEvent.MachineState("m1", MachineState.Unreachable(HOST_KEY_REFUSED)), events.last())
                assertTrue(pins.pins.isEmpty())
            }
        }

    @Test
    fun aDisconnectCancelsAConnectWaitingOnItsHostKey() =
        runTest {
            val core = FakeMobileCore(hosts = MemoryHostKeys())
            core.hostKeys["box.example"] = "SHA256:new"
            val connect = async { runCatching { core.connect(machine) } }
            runCurrent()
            core.disconnect("m1")
            assertTrue(connect.await().exceptionOrNull() is MobileException.Cancelled)
        }

    @Test
    fun aChangedHostKeyReportsBothAndFails() =
        runTest {
            val events = mutableListOf<MobileEvent>()
            val pins = MemoryHostKeys().apply { pins["m1"] = "SHA256:old" }
            val core = FakeMobileCore(sink = EventSinkOf(events), hosts = pins)
            core.hostKeys["box.example"] = "SHA256:new"
            val error = runCatching { core.connect(machine) }.exceptionOrNull()
            assertEquals(HOST_KEY_CHANGED, (error as MobileException.Unreachable).reason)
            assertEquals(
                listOf(
                    MobileEvent.HostKeyChanged("m1", "SHA256:old", "SHA256:new"),
                    MobileEvent.MachineState("m1", MachineState.Unreachable(HOST_KEY_CHANGED)),
                ),
                events,
            )
            assertTrue(core.connected.isEmpty())
            assertEquals("SHA256:old", pins.pins["m1"])
        }

    private class MemoryHostKeys : HostKeyStore {
        val pins = mutableMapOf<String, String>()

        override fun pinned(machineId: String) = pins[machineId]

        override fun pin(
            machineId: String,
            fingerprint: String,
        ) {
            pins[machineId] = fingerprint
        }
    }

    private class EventSinkOf(
        private val events: MutableList<MobileEvent>,
    ) : EventSink {
        override fun onEvent(event: MobileEvent) {
            events += event
        }
    }
}
