package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

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

    private class EventSinkOf(
        private val events: MutableList<MobileEvent>,
    ) : EventSink {
        override fun onEvent(event: MobileEvent) {
            events += event
        }
    }
}
