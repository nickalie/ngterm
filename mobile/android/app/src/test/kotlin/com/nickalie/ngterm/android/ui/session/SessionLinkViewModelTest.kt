package com.nickalie.ngterm.android.ui.session

import android.os.Looper
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
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

/** A session link tapped in the open session: which session it opens and attaches, and when it does neither. */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class SessionLinkViewModelTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val models = ViewModelStore()
    private val key = SessionKey("m1", "s1")

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

    private fun eventually(check: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 10_000
        while (!check()) {
            check(System.currentTimeMillis() < deadline) { "timed out" }
            Thread.sleep(10)
        }
    }

    @Test
    fun aSessionLinkAttachesTheSessionItNamesAndOpensIt() {
        attached("s1", "s2")
        val model = model()
        model.open(key)
        val linked = SessionKey("m1", "s2")
        model.openLinked(linked)
        eventually {
            model.state.value.open
                ?.key == linked
        }
        assertEquals(setOf("s1", "s2"), test.connections.attached.value["m1"])

        model.openLinked(key)
        eventually {
            model.state.value.open
                ?.key == key
        }
        assertEquals("an attached one is opened as it is", setOf("s1", "s2"), test.connections.attached.value["m1"])
    }

    @Test
    fun aSessionLinkToAnEndedSessionAttachesNothingUnlessItIsAttached() {
        attached("s1", "s2")
        val model = model()
        model.open(key)
        val rows = listOf(fakeSession("s1", name = "s1-name"), fakeSession("s2", name = "s2-name").copy(ended = true))
        test.core.emit(MobileEvent.Tree("m1", rows, null))
        eventually {
            test.connections.trees.value["m1"]
                ?.any { it.ended } == true
        }
        model.openLinked(SessionKey("m1", "s2"))
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(0, test.core.calls.count { it.method == "attachSession" && "s2" in it.args })
        assertEquals(setOf("s1"), test.connections.attached.value["m1"])
        assertEquals(
            key,
            model.state.value.open
                ?.key,
        )

        test.core.emit(MobileEvent.Tree("m1", rows.map { it.copy(ended = it.id == "s1") }, null))
        eventually {
            test.connections.trees.value["m1"]
                ?.first()
                ?.ended == true
        }
        model.close()
        model.openLinked(key)
        eventually {
            model.state.value.open
                ?.key == key
        }
    }

    @Test
    fun aSessionLinkFinishingAfterTheViewMovedOnLeavesItsSessionAttachedInTheList() {
        attached("s1", "s2")
        val model = model()
        model.open(key)
        val gate = CompletableDeferred<Unit>().also { test.core.holds["attachSession"] = it }
        model.openLinked(SessionKey("m1", "s2"))
        eventually { test.core.calls.any { it.method == "attachSession" } }
        model.close()
        gate.complete(Unit)
        eventually {
            "s2" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        assertEquals(null, model.state.value.open)
    }

    @Test
    fun aSessionLinkThatFailsToAttachIsANoticeAndTheViewStays() {
        attached("s1", "s2")
        val model = model()
        model.open(key)
        test.core.failNext("attachSession", MobileException.Remote("the session is gone"))
        model.openLinked(SessionKey("m1", "s2"))
        eventually { model.notice.value != null }
        assertEquals("the session is gone", model.notice.value)
        assertEquals(
            key,
            model.state.value.open
                ?.key,
        )
    }
}
