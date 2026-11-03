package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.Machine
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
                listOf(MobileEvent.Tree("m1", listOf(session)), MobileEvent.Tree("m1", emptyList())),
                events,
            )
            assertEquals(
                listOf("connect", "createSession", "listSessions", "endSession", "listSessions"),
                core.calls.map { it.method },
            )
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
