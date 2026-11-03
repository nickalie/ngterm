package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneClipboard
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteHud
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemoteStatus
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class LiveSessionsTest {
    private lateinit var core: FakeMobileCore
    private lateinit var connections: Connections
    private val writes = mutableListOf<String?>()
    private val key = SessionKey("m1", "s1")
    private val pane = PaneRef("m1", "s1", "P1")

    private class Frames : PaneFrames {
        val seen = mutableListOf<String>()

        override fun dirty() {
            seen += "dirty"
        }

        override fun attached() {
            seen += "attached"
        }
    }

    // the test's own coroutines, then the live sessions' (in the background scope, which advanceUntilIdle leaves be)
    private fun TestScope.settle() {
        advanceUntilIdle()
        runCurrent()
    }

    private fun TestScope.live(): LiveSessions {
        connections = Connections(this, { 0L }) { sink -> FakeMobileCore(sink).also { core = it } }
        return LiveSessions(backgroundScope, connections) { writes += it }
    }

    // m1 connected and listed, with s1 attached
    private suspend fun TestScope.attach() {
        core.sessions["m1"] = mutableListOf(fakeSession("s1"))
        connections.connect(Machine("m1", "build-01", "build-01.lan", 22u, "nick", "k1"))
        settle()
        connections.attach("m1", "s1")
        settle()
    }

    @Test
    fun aSessionsEventsLandInItsState() =
        runTest {
            val live = live()
            attach()
            val layout = RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)
            val status = RemoteStatus("active", false, "#5fbfaa", "circle", null, null)
            val hud =
                RemoteHud("Building", "step 2", "bar", null, null, null, "top", false, null, false, true, null, 1, null)
            core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
            core.emit(MobileEvent.SessionLayout("m1", "s1", layout))
            core.emit(MobileEvent.SessionStatus("m1", "s1", status, true))
            core.emit(MobileEvent.SessionContext("m1", "s1", "Rewrite the guide"))
            core.emit(MobileEvent.Hud("m1", "s1", hud))
            assertEquals(
                LiveSession(
                    StreamState.Connected,
                    status,
                    "Rewrite the guide",
                    layout,
                    hud,
                    statusSaid = true,
                    contextSaid = true,
                ),
                live.sessions.value[key],
            )
            core.emit(MobileEvent.Hud("m1", "s1", null))
            assertNull(live.sessions.value[key]?.hud)
        }

    @Test
    fun aStreamFailingBeforeTheSessionShowedListsTheMachineAgainAndAGoneSessionLeaves() =
        runTest {
            val live = live()
            attach()
            val listings = core.calls.count { it.method == "listSessions" }
            // ended elsewhere after the listing the attach went by
            core.sessions["m1"] = mutableListOf()
            core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Failed("no such session")))
            settle()
            assertEquals(listings + 1, core.calls.count { it.method == "listSessions" })
            assertNull(connections.attached.value["m1"]?.takeIf { it.isNotEmpty() })
            assertNull(live.sessions.value[key])
            // a stream that has shown its session failing is retried by the core, not listed again
            core.sessions["m1"] = mutableListOf(fakeSession("s1"))
            connections.listSessions("m1")
            connections.attach("m1", "s1")
            settle()
            core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1"), "P1", null, false)))
            val before = core.calls.count { it.method == "listSessions" }
            core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Failed("link down")))
            settle()
            assertEquals(before, core.calls.count { it.method == "listSessions" })
        }

    @Test
    fun aPanesEventsLandInItsStateAndAFreshTerminalClearsItsEnd() =
        runTest {
            val live = live()
            attach()
            core.emit(MobileEvent.PaneLead(pane, PaneRole.FOLLOWER, true))
            core.emit(MobileEvent.PaneTitle(pane, "vim"))
            assertEquals(
                PaneLive(PaneRole.FOLLOWER, covered = true, title = "vim"),
                live.sessions.value[key]
                    ?.panes
                    ?.get("P1"),
            )
            core.emit(MobileEvent.PaneEnded(pane, PaneEnd.Lost("link down")))
            assertEquals(
                PaneLive(PaneRole.FOLLOWER, covered = false, end = PaneEnd.Lost("link down"), title = "vim"),
                live.sessions.value[key]
                    ?.panes
                    ?.get("P1"),
            )
            core.emit(MobileEvent.PaneAttached(pane))
            assertNull(
                live.sessions.value[key]
                    ?.panes
                    ?.get("P1")
                    ?.end,
            )
        }

    @Test
    fun aRoutedPaneGetsItsFramesAndAnUnroutedOneNone() =
        runTest {
            val live = live()
            val frames = Frames()
            live.route(pane, frames)
            core.emit(MobileEvent.PaneDirty(pane))
            core.emit(MobileEvent.PaneAttached(pane))
            core.emit(MobileEvent.PaneDirty(PaneRef("m1", "s1", "P2")))
            live.route(pane, null)
            core.emit(MobileEvent.PaneDirty(pane))
            assertEquals(listOf("dirty", "attached"), frames.seen)
        }

    @Test
    fun aDetachedSessionsStateGoes() =
        runTest {
            val live = live()
            attach()
            core.emit(MobileEvent.SessionContext("m1", "s1", "notes"))
            settle()
            assertEquals("notes", live.sessions.value[key]?.context)
            connections.detach("m1", "s1")
            settle()
            assertNull(live.sessions.value[key])
        }

    @Test
    fun aProgramsWriteToTheStandardClipboardLandsAndTheOthersDont() =
        runTest {
            live()
            core.emit(MobileEvent.ClipboardWrite(pane, PaneClipboard.STANDARD, "copied"))
            core.emit(MobileEvent.ClipboardWrite(pane, PaneClipboard.PRIMARY, "primary"))
            core.emit(MobileEvent.ClipboardWrite(pane, PaneClipboard.SELECTION, "selection"))
            core.emit(MobileEvent.ClipboardWrite(pane, PaneClipboard.STANDARD, null))
            settle()
            assertEquals(listOf("copied", null), writes)
        }
}
