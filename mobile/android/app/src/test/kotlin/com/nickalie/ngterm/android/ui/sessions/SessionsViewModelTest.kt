package com.nickalie.ngterm.android.ui.sessions

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileException
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
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

private const val MISSING_CLI = "/bin/sh: 1: ngtermctl: not found"

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SessionsViewModelTest {
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

    private fun model(): SessionsViewModel =
        ViewModelProvider(models, viewModelFactory { initializer { SessionsViewModel(test.graph, "m1") } })[
            SessionsViewModel::class.java,
        ]

    // a connected machine m1 with [sessions], connected (and listed) before the screen opens
    private fun connected(vararg sessions: String) {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        test.core.sessions["m1"] = sessions.map { fakeSession(it) }.toMutableList()
        test.connections.connect(testMachine(keyId = key.id).toCore())
        eventually { test.connections.trees.value["m1"] != null }
    }

    private fun SessionsViewModel.ready(predicate: (MachineSessions) -> Boolean = { true }): MachineSessions =
        runBlocking {
            val ready = state.first { it is SessionsUiState.Ready && predicate(it.sessions) }
            (ready as SessionsUiState.Ready).sessions
        }

    private fun SessionsViewModel.ids(): List<String>? =
        (state.value as? SessionsUiState.Ready)?.sessions?.sessions?.map {
            it.id
        }

    private fun eventually(check: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!check()) {
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(10)
        }
    }

    private fun lists() = test.core.calls.count { it.method == "listSessions" }

    @Test
    fun openingAConnectedMachineListsItAfresh() {
        connected("s1")
        val before = lists()
        test.core.sessions["m1"]!! += fakeSession("s2")
        val model = model()
        model.opened()
        eventually { model.ids() == listOf("s1", "s2") }
        assertEquals(before + 1, lists())
    }

    @Test
    fun aMachineNotConnectedIsListedByItsConnect() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1"))
        val model = model()
        model.opened()
        assertEquals(Link.Offline, model.ready().link)
        assertEquals(0, lists())
        model.connect()
        val ready = model.ready { it.link == Link.Connected && it.sessions != null }
        assertEquals(listOf("s1"), ready.sessions?.map { it.id })
    }

    @Test
    fun newSessionCreatesItNamedOrNotAndClosesTheDialog() {
        connected()
        val model = model()
        model.openNamedSession()
        assertEquals(NewSessionState(), model.creating.value)
        model.create("  api  ")
        eventually { model.creating.value == null }
        model.openNamedSession()
        model.create(" ")
        eventually { model.creating.value == null }
        assertEquals(listOf("api", "session 2"), model.ready().sessions?.map { it.name })
        val names =
            test.core.calls
                .filter { it.method == "createSession" }
                .map { it.args[1] }
        assertEquals(listOf("api", null), names)
        assertEquals(setOf("fake-1", "fake-2"), test.connections.attached.value["m1"])
        assertEquals(SessionKey("m1", "fake-2"), model.landing.value)
    }

    @Test
    fun newSessionInOneTapCreatesAttachesAndLandsTheSession() {
        connected()
        val model = model()
        val gate = CompletableDeferred<Unit>().also { test.core.holds["createSession"] = it }
        model.newSession()
        eventually { test.core.calls.any { it.method == "createSession" } }
        assertEquals(true, model.busy.value)
        model.newSession()
        model.create("api")
        gate.complete(Unit)
        eventually { model.landing.value != null }
        assertEquals(SessionKey("m1", "fake-1"), model.landing.value)
        assertEquals(setOf("fake-1"), test.connections.attached.value["m1"])
        assertEquals(
            listOf(null),
            test.core.calls
                .filter { it.method == "createSession" }
                .map { it.args[1] },
        )
        assertEquals(false, model.busy.value)
        assertNull(model.creating.value)
        model.landed()
        assertNull(model.landing.value)
    }

    @Test
    fun aFailedOneTapNewSessionIsANoticeAndLandsNothing() {
        connected()
        val model = model()
        test.core.failNext("createSession", MobileException.Remote("build-01 does not run live sessions"))
        model.newSession()
        eventually { model.notice.value != null }
        assertEquals("build-01 does not run live sessions", model.notice.value)
        assertNull(model.landing.value)
        assertEquals(false, model.busy.value)
        test.core.failNext("attachSession", MobileException.Remote("no such session"))
        model.newSession()
        eventually { model.notice.value == "no such session" }
        assertNull(model.landing.value)
        assertEquals(listOf("session 1"), model.ready().sessions?.map { it.name })
        assertNull(test.connections.attached.value["m1"])
    }

    @Test
    fun aDialogCreateWhoseAttachFailsClosesTheDialogAndSaysWhy() {
        connected()
        val model = model()
        model.openNamedSession()
        test.core.failNext("attachSession", MobileException.Remote("no such session"))
        model.create("api")
        eventually { model.notice.value != null }
        assertEquals("no such session", model.notice.value)
        assertNull(model.creating.value)
        assertNull(model.landing.value)
    }

    @Test
    fun aFailedCreateStaysInTheDialogItBeganIn() {
        connected()
        val model = model()
        model.openNamedSession()
        test.core.failNext("createSession", MobileException.Remote("build-01 does not run live sessions"))
        model.create("api")
        eventually { model.creating.value?.failed != null }
        assertEquals(
            NewSessionState(running = false, failed = "build-01 does not run live sessions"),
            model.creating.value,
        )
        assertNull(model.notice.value)
        model.closeNewSession()
        assertNull(model.creating.value)
    }

    @Test
    fun aCreateWhoseDialogClosedFailsToTheSnackbarAndLeavesANewDialogAlone() {
        connected()
        val model = model()
        model.openNamedSession()
        test.core.failNext("createSession", MobileException.Remote("build-01 does not run live sessions"))
        val gate = CompletableDeferred<Unit>().also { test.core.holds["createSession"] = it }
        model.create("api")
        eventually { test.core.calls.any { it.method == "createSession" } }
        model.closeNewSession()
        model.openNamedSession()
        gate.complete(Unit)
        eventually { model.notice.value != null }
        assertEquals("build-01 does not run live sessions", model.notice.value)
        // the dialog opened since is not the failed create's
        assertEquals(NewSessionState(), model.creating.value)
    }

    @Test
    fun aCreateWhoseDialogClosedLeavesANewDialogOpenWhenItLands() {
        connected()
        val model = model()
        model.openNamedSession()
        val gate = CompletableDeferred<Unit>().also { test.core.holds["createSession"] = it }
        model.create("api")
        eventually { test.core.calls.any { it.method == "createSession" } }
        model.closeNewSession()
        model.openNamedSession()
        gate.complete(Unit)
        eventually { model.ready().sessions?.map { it.name } == listOf("api") }
        assertEquals(NewSessionState(), model.creating.value)
        assertNull(model.notice.value)
    }

    @Test
    fun endSessionRemovesItAndAFailureIsANotice() {
        connected("s1", "s2")
        val model = model()
        eventually { model.ids() == listOf("s1", "s2") }
        model.end("s1")
        eventually { model.ids() == listOf("s2") }
        assertEquals(emptySet<String>(), model.ready().ending)
        test.core.failNext("endSession", MobileException.Remote("no session with that id"))
        model.end("s2")
        eventually { model.notice.value != null }
        assertEquals("no session with that id", model.notice.value)
        assertEquals(listOf("s2"), model.ids())
        model.noticeShown()
        assertNull(model.notice.value)
    }

    @Test
    fun attachingPutsTheSessionInTheAttachedGroupAndEndingLetsItGo() {
        connected("s1", "s2")
        val model = model()
        model.attach("s2")
        eventually { model.ready().attached == setOf("s2") }
        val body = model.ready().body() as SessionsBody.Sessions
        assertEquals(listOf("s2"), body.attached.map { it.session.id })
        assertEquals(listOf("s1"), body.others.map { it.session.id })
        model.detach("s2")
        eventually { model.ready().attached.isEmpty() }
        model.attach("s1")
        eventually { model.ready().attached == setOf("s1") }
        model.end("s1")
        eventually { model.ready().attached.isEmpty() }
        assertEquals(1, test.core.calls.count { it.method == "detachSession" && it.args[1] == "s1" })
    }

    @Test
    fun aMissingNgtermctlAndAFailedRefreshAreShown() {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        test.core.failNext("listSessions", MobileException.Remote(MISSING_CLI))
        val model = model()
        model.connect()
        val missing = model.ready { it.failure != null }
        assertEquals(SessionsBody.MissingCli(MISSING_CLI), missing.body())
        // installed now: Retry lists it
        model.refresh()
        val listed = model.ready { it.sessions != null }
        assertNull(listed.failure)
        assertEquals(SessionsBody.Empty(null), listed.body())
        test.core.failNext("listSessions", MobileException.Remote("build-01 gave no answer"))
        model.refresh()
        assertEquals(SessionsBody.Empty("build-01 gave no answer"), model.ready { it.failure != null }.body())
    }

    @Test
    fun aRemovedMachineIsGone() {
        connected("s1")
        val model = model()
        model.ready()
        runBlocking { test.graph.machines.delete("m1") }
        runBlocking { model.state.first { it == SessionsUiState.Gone } }
    }
}
