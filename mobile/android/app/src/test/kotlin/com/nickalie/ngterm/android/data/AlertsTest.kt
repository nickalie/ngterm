package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.asks.testAsk
import com.nickalie.ngterm.android.ui.asks.testChoices
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import com.nickalie.ngterm.core.ffi.RemoteNotice
import com.nickalie.ngterm.core.ffi.RemoteStatus
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The core's events and the open asks mapped to notifications, through a sink that records them. */
@OptIn(ExperimentalCoroutinesApi::class)
class AlertsTest {
    private lateinit var core: FakeMobileCore
    private lateinit var connections: Connections
    private lateinit var asks: Asks
    private val enabled = MutableStateFlow(AlertKind.entries.toSet())
    private val key = SessionKey("m1", "s1")

    /** What the alerts did, in order: `post <alert>` or `cancel <tag>`. */
    private class Sink : AlertSink {
        val posted = mutableListOf<Alert>()
        val cancelled = mutableListOf<String>()

        override fun post(alert: Alert) {
            posted += alert
        }

        override fun cancel(tag: String) {
            cancelled += tag
        }
    }

    private val sink = Sink()

    private fun TestScope.settle() {
        advanceUntilIdle()
        runCurrent()
    }

    // m1 (build-01) connected and listed with docs-site (s1), attached
    private suspend fun TestScope.attached() {
        connections = Connections(this, { 0L }) { s -> FakeMobileCore(s).also { core = it } }
        asks = Asks(backgroundScope, connections)
        Alerts(backgroundScope, connections, asks, flowOf(listOf(testMachine())), enabled, sink)
        core.sessions["m1"] =
            mutableListOf(fakeSession("s1", name = "docs-site").copy(context = "Rewrite the install guide"))
        connections.connect(testMachine().toCore())
        settle()
        connections.attach("m1", "s1")
        settle()
    }

    private fun status(
        status: String,
        changed: Boolean = true,
    ) = MobileEvent.SessionStatus("m1", "s1", RemoteStatus(status, false, null, null, "P1", null), changed)

    @Test
    fun anAgentsNoticeIsPostedWithItsSessionAndPane() =
        runTest {
            attached()
            core.emit(
                MobileEvent.Notify("m1", "s1", RemoteNotice("Build done", "All 214 tests passed", "P2", "notify")),
            )
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("", "No title", null, "notify")))
            val notices = sink.posted.filterIsInstance<Alert.Notice>()
            assertEquals(2, notices.size)
            val first = notices[0]
            assertEquals("build-01", first.machine)
            assertEquals("docs-site", first.session)
            assertEquals("Build done", first.title)
            assertEquals("All 214 tests passed", first.body)
            assertEquals(OpenTarget("m1", "s1", "P2"), first.target)
            // an untitled notice takes the session's name; each is its own notification
            assertEquals("docs-site", notices[1].title)
            assertTrue(notices[0].tag != notices[1].tag)
        }

    @Test
    fun waitingAndDoneArePostedAndAnyOtherChangeTakesThemDown() =
        runTest {
            attached()
            core.emit(status("blocked"))
            core.emit(status("completed"))
            core.emit(status("active"))
            core.emit(status("blocked", changed = false))
            val statuses = sink.posted.filterIsInstance<Alert.Status>()
            assertEquals(listOf(AlertKind.Waiting, AlertKind.Done), statuses.map { it.kind })
            assertEquals("Rewrite the install guide", statuses[0].context)
            assertEquals(OpenTarget("m1", "s1", "P1"), statuses[0].target)
            assertEquals(statusTag(key), statuses[0].tag)
            assertTrue(statusTag(key) in sink.cancelled)
        }

    @Test
    fun aKindTurnedOffIsNotPosted() =
        runTest {
            attached()
            enabled.value = setOf(AlertKind.Done)
            settle()
            core.emit(status("blocked"))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            assertEquals(emptyList<Alert>(), sink.posted)
            core.emit(status("completed"))
            assertEquals(listOf(AlertKind.Done), sink.posted.filterIsInstance<Alert.Status>().map { it.kind })
        }

    @Test
    fun aStatusOfTheSessionOnScreenIsNotPosted() =
        runTest {
            attached()
            asks.showing(key)
            settle()
            core.emit(status("blocked"))
            assertEquals(emptyList<Alert>(), sink.posted)
            // a status posted before the session came on screen goes when it does
            asks.showing(null)
            settle()
            core.emit(status("completed"))
            sink.cancelled.clear()
            asks.showing(key)
            settle()
            assertTrue(statusTag(key) in sink.cancelled)
        }

    @Test
    fun aMachineLostWithSessionsAttachedIsPostedUntilItConnects() =
        runTest {
            attached()
            core.emit(MobileEvent.HostKeyChanged("m1", "SHA256:old", "SHA256:new"))
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("host key changed")))
            assertEquals(
                listOf(Alert.Lost("m1", "build-01", LostReason.HostKeyChanged)),
                sink.posted.filterIsInstance<Alert.Lost>(),
            )
            core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
            assertTrue(lostTag("m1") in sink.cancelled)
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("connection refused")))
            assertEquals(
                Alert.Lost("m1", "build-01", LostReason.Unreachable("connection refused")),
                sink.posted.last(),
            )
            // a machine with nothing attached is lost quietly
            sink.posted.clear()
            core.emit(MobileEvent.MachineState("m2", MachineState.Unreachable("connection refused")))
            assertEquals(emptyList<Alert>(), sink.posted)
        }

    @Test
    fun anAskOutOfSightIsPostedAsTheAskRuleSays() =
        runTest {
            attached()
            core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            settle()
            val posted = sink.posted.filterIsInstance<Alert.Ask>().single()
            assertEquals(AskDelivery.Actions, posted.delivery)
            assertEquals("docs-site", posted.session)
            // on screen it is the sheet's: the notification goes, and comes back when the session leaves the screen
            asks.showing(key)
            settle()
            assertTrue(askTag(posted.open) in sink.cancelled)
            asks.showing(null)
            settle()
            assertEquals(2, sink.posted.filterIsInstance<Alert.Ask>().size)
            // dismissed by the core: taken down
            sink.cancelled.clear()
            core.emit(MobileEvent.AskDismiss("m1", "s1", "A1"))
            settle()
            assertEquals(listOf(askTag(posted.open)), sink.cancelled)
            assertEquals(emptyList<OpenAsk>(), asks.open.value)
        }

    @Test
    fun answeringAnAskSendsItOnceAndTakesItsNotificationDown() =
        runTest {
            attached()
            core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            settle()
            assertTrue(asks.answer(key, "A1", AskReply("yes", null, null)))
            assertFalse(asks.answer(key, "A1", AskReply("yes", null, null)))
            settle()
            val answers = core.calls.filter { it.method == "answerAsk" }
            assertEquals(listOf(FakeCall("answerAsk", listOf("m1", "s1", "A1", AskReply("yes", null, null)))), answers)
            assertTrue(sink.cancelled.any { it.startsWith("ask:m1:s1:A1") })
        }

    @Test
    fun detachingASessionDropsItsAsksAndItsStatus() =
        runTest {
            attached()
            core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            core.emit(status("blocked"))
            settle()
            connections.detach("m1", "s1")
            settle()
            assertEquals(emptyList<OpenAsk>(), asks.open.value)
            assertTrue(statusTag(key) in sink.cancelled)
            assertTrue(sink.cancelled.any { it.startsWith("ask:") })
        }

    @Test
    fun anAsksDeliveryFollowsItsForm() {
        assertEquals(AskDelivery.Actions, deliveryOf(testAsk("yes", "no", "later")))
        assertEquals(AskDelivery.OpenApp, deliveryOf(testAsk("a", "b", "c", "d")))
        assertEquals(
            AskDelivery.OpenApp,
            deliveryOf(testAsk().copy(buttons = emptyList(), defaultId = null, cancelId = null)),
        )
        assertEquals(AskDelivery.Reply, deliveryOf(testAsk(input = RemoteAskInput(false, "", null))))
        assertEquals(AskDelivery.OpenApp, deliveryOf(testAsk(choices = testChoices())))
    }
}
