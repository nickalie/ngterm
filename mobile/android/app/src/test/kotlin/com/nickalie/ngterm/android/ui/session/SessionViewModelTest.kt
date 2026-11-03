package com.nickalie.ngterm.android.ui.session

import android.os.Looper
import androidx.compose.ui.unit.sp
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemoteStatus
import kotlinx.coroutines.CompletableDeferred
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
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SessionViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val models = ViewModelStore()
    private val key = SessionKey("m1", "s1")
    private val size = PaneSize(80u, 24u, 10u, 20u)

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

    private fun model(): SessionViewModel =
        ViewModelProvider(models, viewModelFactory { initializer { SessionViewModel(test.graph) } })[
            SessionViewModel::class.java,
        ]

    // m1 connected with [sessions] listed, and the first attached
    private fun attached(vararg sessions: String) {
        val stored = testMachine(keyId = test.seedKey().id)
        test.seedMachine(stored)
        test.core.sessions["m1"] = sessions.map { fakeSession(it, name = "$it-name") }.toMutableList()
        test.core.paneFactory = { _, _, pane, _ -> FakePane(pane) }
        test.connections.connect(stored.toCore())
        eventually { test.connections.trees.value["m1"] != null }
        runBlocking { test.connections.attach("m1", sessions.first()) }
    }

    private fun layout(vararg panes: String) =
        test.core.emit(
            MobileEvent.SessionLayout("m1", "s1", RemoteLayout(panes.toList(), panes.first(), "vertical", true)),
        )

    private fun eventually(check: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!check()) {
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(10)
        }
    }

    @Test
    fun theAttachedSessionsAreGroupedByMachineWithTheStreamsStatusOverTheListings() {
        attached("s1", "s2")
        val model = model()
        eventually { model.state.value.sessions.size == 1 }
        val session =
            model.state.value.groups
                .single()
                .sessions
                .single()
        assertEquals(
            "build-01",
            model.state.value.groups
                .single()
                .machine,
        )
        assertEquals("s1-name", session.name)
        assertNull(session.status)
        test.core.emit(
            MobileEvent.SessionStatus("m1", "s1", RemoteStatus("active", false, "#5fbfaa", "circle", null, null), true),
        )
        eventually {
            model.state.value.sessions
                .single()
                .status != null
        }
        assertEquals(
            AgentStatus("active", "#5fbfaa", "circle"),
            model.state.value.sessions
                .single()
                .status,
        )
    }

    @Test
    fun anOpenSessionFocusesItsPrimaryPaneUntilAnotherIsFocused() {
        attached("s1")
        val model = model()
        model.open(key)
        layout("P1", "P2")
        eventually { (model.state.value.open as? OpenSession.Live)?.focused == "P1" }
        model.focus(key, "P2")
        eventually { (model.state.value.open as? OpenSession.Live)?.focused == "P2" }
        // a layout without the focused pane falls back to its primary
        layout("P1")
        eventually { (model.state.value.open as? OpenSession.Live)?.focused == "P1" }
    }

    @Test
    fun aSessionThatLeavesTheListingWhileOpenIsGone() {
        attached("s1")
        val model = model()
        model.open(key)
        eventually { model.state.value.open is OpenSession.Live }
        test.core.sessions["m1"]?.clear()
        runBlocking { test.connections.listSessions("m1") }
        eventually { model.state.value.open is OpenSession.Gone }
        assertEquals(OpenSession.Gone(key, "s1-name", "build-01"), model.state.value.open)
        model.close()
        eventually { model.state.value.open == null }
    }

    @Test
    fun aPaneGoneIsTheSessionGone() {
        attached("s1")
        val model = model()
        model.open(key)
        test.core.emit(MobileEvent.PaneEnded(PaneRef("m1", "s1", "P1"), PaneEnd.Gone))
        eventually { model.state.value.open is OpenSession.Gone }
    }

    @Test
    fun detachingClosesTheViewAndHidesItsPanes() {
        attached("s1")
        val model = model()
        model.open(key)
        layout("P1")
        model.show(PaneRef("m1", "s1", "P1"), size)
        eventually { model.panes.value.isNotEmpty() }
        model.detach(key)
        eventually { model.state.value.open == null && model.panes.value.isEmpty() }
        assertTrue(test.core.shownPanes.isEmpty())
    }

    @Test
    fun theSettingsTextSizeAndFontReachEveryShownPaneOverAPinch() {
        runBlocking { test.settings.update { it.copy(textSize = 15) } }
        attached("s1")
        val model = model()
        layout("P1")
        val p1 = PaneRef("m1", "s1", "P1")
        eventually { runBlocking { test.settings.settings.first() }.textSize == 15 }
        model.show(p1, size)
        eventually { p1 in model.panes.value }
        val state = model.panes.value.getValue(p1)
        eventually { state.textSize == 15.sp }
        // a pinch sizes this pane alone, until Settings sets a size again
        state.textSize = 20.sp
        runBlocking { test.settings.update { it.copy(font = FontChoice.SystemMono) } }
        eventually { state.face == FontChoice.SystemMono }
        assertEquals(20.sp, state.textSize)
        runBlocking { test.settings.update { it.copy(textSize = 18) } }
        eventually { state.textSize == 18.sp }
        assertEquals(18.sp, state.defaultTextSize)
    }

    @Test
    fun aPaneIsShownOnceAndHiddenWhenTheLayoutDropsIt() {
        attached("s1")
        val model = model()
        layout("P1", "P2")
        val p2 = PaneRef("m1", "s1", "P2")
        model.show(p2, size)
        model.show(p2, size)
        eventually { p2 in model.panes.value }
        val state = model.panes.value[p2]
        model.show(p2, size)
        assertSame(state, model.panes.value[p2])
        assertEquals(1, test.core.calls.count { it.method == "showPane" })
        layout("P1")
        eventually { p2 !in model.panes.value }
        assertTrue(test.core.calls.any { it.method == "hidePane" && it.args == listOf("m1", "s1", "P2") })
    }

    @Test
    fun aPaneThatCantBeShownSaysWhyAndATakeControlReachesTheCore() {
        attached("s1")
        val model = model()
        val p1 = PaneRef("m1", "s1", "P1")
        test.core.failNext("showPane", MobileException.Remote("no pane P1"))
        model.show(p1, size)
        eventually { model.paneFailures.value[p1] == "no pane P1" }
        model.show(p1, size)
        eventually { p1 in model.panes.value && p1 !in model.paneFailures.value }
        model.takeLead(p1)
        assertTrue(test.core.calls.any { it.method == "takeLead" && it.args == listOf("m1", "s1", "P1") })
    }

    @Test
    fun aPaneASplitAddedSinceTheLastListingIsShownAfterTheMachineIsListedAgain() {
        // the core finds a pane through the machine's last listing, which has only the pane the session started with
        test.core.panes["s1"] = setOf("P1")
        attached("s1")
        val model = model()
        layout("P1")
        val p1 = PaneRef("m1", "s1", "P1")
        model.show(p1, size)
        eventually { p1 in model.panes.value }
        test.core.panes["s1"] = setOf("P1", "P2")
        layout("P1", "P2")
        val p2 = PaneRef("m1", "s1", "P2")
        model.show(p2, size)
        eventually { p2 in model.panes.value || p2 in model.paneFailures.value }
        assertEquals(null, model.paneFailures.value[p2])
        assertTrue(p2 in model.panes.value)
    }

    @Test
    fun aRetryReadsTheMachinesSessionsAgainBeforeShowingThePane() {
        attached("s1")
        val model = model()
        val p1 = PaneRef("m1", "s1", "P1")
        test.core.failNext("showPane", MobileException.Remote("no pane P1"))
        model.show(p1, size)
        eventually { model.paneFailures.value[p1] == "no pane P1" }
        val before = test.core.calls.size
        model.retry(p1, size)
        eventually { p1 in model.panes.value }
        assertEquals(
            listOf("listSessions", "showPane"),
            test.core.calls
                .drop(before)
                .map { it.method },
        )
    }

    @Test
    fun aPaneThatFailedWhileItsMachineReconnectedIsTriedAgainOnceItIsBack() {
        attached("s1")
        val model = model()
        val p1 = PaneRef("m1", "s1", "P1")
        test.core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(5_000u)))
        eventually { test.connections.links.value["m1"] is Link.Reconnecting }
        test.core.failNext("showPane", MobileException.NotConnected("m1"))
        model.show(p1, size)
        eventually { p1 in model.paneFailures.value }
        test.core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
        eventually { p1 !in model.paneFailures.value }

        // a show begun while the machine was down, failing after it came back, is stale: it records nothing, and the
        // pane is tried again on the link that's back, since its slot won't ask again
        val p2 = PaneRef("m1", "s1", "P2")
        test.core.emit(MobileEvent.MachineState("m1", MachineState.Reconnecting(5_000u)))
        eventually { test.connections.links.value["m1"] is Link.Reconnecting }
        val gate = CompletableDeferred<Unit>()
        test.core.holds["showPane"] = gate
        test.core.failNext("showPane", MobileException.NotConnected("m1"))
        model.show(p2, size)
        eventually { test.core.calls.any { it.method == "showPane" && it.args[2] == "P2" } }
        test.core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
        eventually { test.connections.links.value["m1"] == Link.Connected }
        shadowOf(Looper.getMainLooper()).idle()
        gate.complete(Unit)
        shadowOf(Looper.getMainLooper()).idle()
        eventually { p2 in model.panes.value || p2 in model.paneFailures.value }
        assertTrue(p2 !in model.paneFailures.value)
        assertEquals(2, test.core.calls.count { it.method == "showPane" && it.args[2] == "P2" })
    }

    @Test
    fun aPaneShownAfterItsSessionDetachedWhileItWasOnItsWayIsHiddenAgain() {
        attached("s1")
        val model = model()
        val p1 = PaneRef("m1", "s1", "P1")
        // a Retry waits on the machine's listing while the session is detached; the core still shows the pane then
        val gate = CompletableDeferred<Unit>()
        test.core.holds["listSessions"] = gate
        val listed = test.core.calls.count { it.method == "listSessions" }
        model.retry(p1, size)
        eventually { test.core.calls.count { it.method == "listSessions" } > listed }
        model.detach(key)
        eventually { test.connections.attached.value["m1"] == null }
        gate.complete(Unit)
        eventually { test.core.calls.any { it.method == "hidePane" && it.args == listOf("m1", "s1", "P1") } }
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(test.core.shownPanes.isEmpty())
        assertTrue(p1 !in model.panes.value)
    }

    @Test
    fun endingTheOpenSessionClosesItsViewAndAFailureIsANotice() {
        attached("s1", "s2")
        runBlocking { test.connections.attach("m1", "s2") }
        val model = model()
        test.core.failNext("endSession", MobileException.Remote("refused"))
        model.open(SessionKey("m1", "s2"))
        model.end(SessionKey("m1", "s2"))
        eventually { model.notice.value == "refused" }
        model.end(SessionKey("m1", "s2"))
        eventually { model.state.value.open == null }
        assertEquals(
            listOf("s1"),
            model.state.value.sessions
                .map { it.key.sessionId },
        )
    }

    @Test
    fun aPanesShowFailureGoesWithItsSessionsDetach() {
        attached("s1")
        val model = model()
        val p1 = PaneRef("m1", "s1", "P1")
        test.core.failNext("showPane", MobileException.Remote("no pane P1"))
        model.show(p1, size)
        eventually { model.paneFailures.value[p1] == "no pane P1" }
        model.detach(key)
        eventually { model.paneFailures.value.isEmpty() }
        runBlocking { test.connections.attach("m1", "s1") }
        eventually {
            test.connections.attached.value["m1"]
                ?.contains("s1") == true
        }
        assertTrue(model.paneFailures.value.isEmpty())
    }

    @Test
    fun theNextSessionAndPaneWrapAroundBothWays() {
        val a = SessionKey("m1", "a")
        val b = SessionKey("m1", "b")
        val c = SessionKey("m1", "c")
        assertEquals(b, listOf(a, b, c).cyclicNeighbor(a, forward = true))
        assertEquals(c, listOf(a, b, c).cyclicNeighbor(a, forward = false))
        assertEquals(a, listOf(a, b, c).cyclicNeighbor(c, forward = true))
        assertNull(listOf(a).cyclicNeighbor(a, forward = true))
        assertEquals("P2", listOf("P1", "P2", "P3").cyclicNeighbor("P1", forward = true))
        assertEquals("P3", listOf("P1", "P2", "P3").cyclicNeighbor("P1", forward = false))
        assertEquals("P2", listOf("P1", "P2", "P3").cyclicNeighbor(null, forward = true))
    }
}
