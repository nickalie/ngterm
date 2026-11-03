package com.nickalie.ngterm.android.ui.session

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemoteStatus
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
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
