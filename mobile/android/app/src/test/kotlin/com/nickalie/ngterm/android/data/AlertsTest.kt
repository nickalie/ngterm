package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.asks.testAsk
import com.nickalie.ngterm.android.ui.asks.testChoices
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.FakeMobileCore
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.ClipboardWriteStatus
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import com.nickalie.ngterm.core.ffi.RemoteNotice
import com.nickalie.ngterm.core.ffi.RemoteStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
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
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

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
        val posted: MutableList<Alert> = Collections.synchronizedList(mutableListOf())
        val cancelled: MutableList<String> = Collections.synchronizedList(mutableListOf())

        // the tags in the order of their posts and cancels, and the post that waits (a race's window) when set
        val log: MutableList<Pair<String, Boolean>> = Collections.synchronizedList(mutableListOf())
        val posting = CountDownLatch(1)

        @Volatile var hold: CountDownLatch? = null

        override fun post(alert: Alert) {
            posting.countDown()
            hold?.await(HOLD_MS, TimeUnit.MILLISECONDS)
            posted += alert
            log += alert.tag to true
        }

        override fun cancel(tag: String) {
            cancelled += tag
            log += tag to false
        }

        fun up(tag: String): Boolean = log.lastOrNull { it.first == tag }?.second == true
    }

    private val sink = Sink()

    private fun TestScope.settle() {
        advanceUntilIdle()
        runCurrent()
    }

    // m1 (build-01) connected and listed with docs-site (s1), attached
    private suspend fun TestScope.attached(
        alertsScope: CoroutineScope = backgroundScope,
        prefs: Flow<Set<AlertKind>> = enabled,
    ) {
        connections = Connections(this, { 0L }) { s -> FakeMobileCore(s).also { core = it } }
        asks = Asks(backgroundScope, connections)
        val live = LiveSessions(backgroundScope, connections) { ClipboardWriteStatus.DONE }
        Alerts(alertsScope, connections, asks, flowOf(listOf(testMachine())), prefs, sink, live.sessions)
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
    fun aSessionRenamedOrGivenANewContextOnItsStreamIsPostedSo() =
        runTest {
            attached()
            core.emit(MobileEvent.SessionName("m1", "s1", "Fix Linux"))
            core.emit(MobileEvent.SessionContext("m1", "s1", "Sidebar rename drops the suffix"))
            core.emit(status("blocked"))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("", "No title", null, "notify")))
            val status = sink.posted.filterIsInstance<Alert.Status>().single()
            assertEquals("Fix Linux", status.session)
            assertEquals("Sidebar rename drops the suffix", status.context)
            assertEquals(
                "Fix Linux",
                sink.posted
                    .filterIsInstance<Alert.Notice>()
                    .single()
                    .title,
            )
            // a cleared context is a clear, not a gap the listing fills
            core.emit(MobileEvent.SessionContext("m1", "s1", null))
            core.emit(status("completed"))
            assertEquals(
                null,
                sink.posted
                    .filterIsInstance<Alert.Status>()
                    .last()
                    .context,
            )
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
    fun everyKindOffPostsNothing() =
        runTest {
            attached()
            enabled.value = emptySet()
            settle()
            core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            core.emit(status("blocked"))
            core.emit(status("completed"))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            core.emit(MobileEvent.HostKeyChanged("m1", "SHA256:old", "SHA256:new"))
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            settle()
            assertEquals(emptyList<Alert>(), sink.posted)
            // the question still waits, for the app's sheet
            assertEquals(1, asks.open.value.size)
        }

    @Test
    fun questionsTurnedOffAreTakenDownAndComeBackWhenTurnedOn() =
        runTest {
            attached()
            core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            settle()
            val posted = sink.posted.filterIsInstance<Alert.Ask>().single()
            enabled.value = AlertKind.entries.toSet() - AlertKind.Questions
            settle()
            assertTrue(askTag(posted.open) in sink.cancelled)
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertEquals(2, sink.posted.filterIsInstance<Alert.Ask>().size)
        }

    @Test
    fun aKindTurnedOffTakesItsNotificationsDown() =
        runTest {
            attached()
            core.emit(status("blocked"))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            settle()
            val notice = sink.posted.filterIsInstance<Alert.Notice>().single()
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            sink.cancelled.clear()
            enabled.value = setOf(AlertKind.Done, AlertKind.Notices, AlertKind.Errors, AlertKind.Questions)
            settle()
            assertEquals(listOf(statusTag(key)), sink.cancelled)
            enabled.value = setOf(AlertKind.Questions)
            settle()
            assertEquals(setOf(statusTag(key), notice.tag, lostTag("m1")), sink.cancelled.toSet())
        }

    @Test
    fun aKindTurnedOnAgainBringsBackWhatIsStillCurrent() =
        runTest {
            attached()
            core.emit(status("blocked"))
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            val notice = sink.posted.filterIsInstance<Alert.Notice>().single()
            enabled.value = setOf(AlertKind.Questions)
            settle()
            assertFalse(sink.up(statusTag(key)) || sink.up(lostTag("m1")) || sink.up(notice.tag))
            sink.posted.clear()
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertTrue(sink.up(statusTag(key)) && sink.up(lostTag("m1")))
            assertEquals(
                AlertKind.Waiting,
                sink.posted
                    .filterIsInstance<Alert.Status>()
                    .single()
                    .kind,
            )
            assertEquals(
                LostReason.Unreachable("refused"),
                sink.posted
                    .filterIsInstance<Alert.Lost>()
                    .single()
                    .reason,
            )
            // a notice is a message of its time: it does not come back
            assertFalse(sink.up(notice.tag))
        }

    @Test
    fun aMachineBackWhileErrorsWasOffStaysDownWhenItIsTurnedOn() =
        runTest {
            attached()
            enabled.value = AlertKind.entries.toSet() - AlertKind.Errors
            settle()
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertFalse(sink.up(lostTag("m1")))
            assertEquals(emptyList<Alert>(), sink.posted.filterIsInstance<Alert.Lost>())
        }

    @Test
    fun aMachineWithNothingAttachedLeftStaysDownWhenErrorsIsTurnedOn() =
        runTest {
            attached()
            core.emit(MobileEvent.MachineState("m1", MachineState.Unreachable("refused")))
            connections.detach("m1", "s1")
            settle()
            assertFalse(sink.up(lostTag("m1")))
            enabled.value = AlertKind.entries.toSet() - AlertKind.Errors
            settle()
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertFalse(sink.up(lostTag("m1")))
        }

    @Test
    fun aStatusThatCameWhileItsKindWasOffPostsWhenItIsTurnedOn() =
        runTest {
            attached()
            enabled.value = AlertKind.entries.toSet() - AlertKind.Waiting
            settle()
            core.emit(status("blocked"))
            assertFalse(sink.up(statusTag(key)))
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertTrue(sink.up(statusTag(key)))
            // a change of another kind does not post it again, and a status gone by then does not come back
            enabled.value = AlertKind.entries.toSet() - AlertKind.Notices
            settle()
            assertEquals(1, sink.posted.filterIsInstance<Alert.Status>().size)
            core.emit(status("active"))
            enabled.value = AlertKind.entries.toSet() - AlertKind.Waiting
            settle()
            enabled.value = AlertKind.entries.toSet()
            settle()
            assertFalse(sink.up(statusTag(key)))
        }

    @Test
    fun aStatusBroughtBackShowsTheSessionsNameAndContextOfNow() =
        runTest {
            attached()
            enabled.value = AlertKind.entries.toSet() - AlertKind.Waiting
            settle()
            core.emit(status("blocked"))
            core.emit(MobileEvent.SessionName("m1", "s1", "Fix Linux"))
            core.emit(MobileEvent.SessionContext("m1", "s1", "Sidebar rename drops the suffix"))
            settle()
            enabled.value = AlertKind.entries.toSet()
            settle()
            val status = sink.posted.filterIsInstance<Alert.Status>().single()
            assertEquals("Fix Linux", status.session)
            assertEquals("Sidebar rename drops the suffix", status.context)
        }

    @Test
    fun aKindTurnedOffWhileItsPostIsOnTheWayTakesItDown() =
        runTest {
            attached()
            val release = CountDownLatch(1)
            sink.hold = release
            // the core's thread posts waiting, and the user turns Waiting off before that post lands
            val emitter = thread { core.emit(status("blocked")) }
            sink.posting.await()
            enabled.value = AlertKind.entries.toSet() - AlertKind.Waiting
            val switcher = thread { settle() }
            val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(HOLD_MS)
            while (switcher.state != Thread.State.BLOCKED && System.nanoTime() < deadline) Thread.yield()
            assertEquals(Thread.State.BLOCKED, switcher.state)
            release.countDown()
            emitter.join()
            switcher.join()
            assertFalse(sink.up(statusTag(key)))
        }

    @Test
    fun anEventBeforeTheChoicesAreReadWaitsForThem() =
        runTest {
            // the alerts' own scope never runs: their collectors have not read the choices when the events come
            val idle = TestScope()
            attached(idle, flowOf(emptySet()))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            core.emit(status("blocked"))
            assertEquals(emptyList<Alert>(), sink.posted)
            idle.cancel()
        }

    @Test
    fun anEventBeforeTheChoicesAreReadPostsWhenTheySayOn() =
        runTest {
            val idle = TestScope()
            attached(idle, flowOf(setOf(AlertKind.Notices)))
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("t", "b", null, "notify")))
            assertEquals(1, sink.posted.filterIsInstance<Alert.Notice>().size)
            idle.cancel()
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
    fun detachingASessionTakesItsAgentsNoticesDown() =
        runTest {
            attached()
            core.emit(MobileEvent.Notify("m1", "s1", RemoteNotice("Build done", "All tests passed", null, "notify")))
            settle()
            val notice = sink.posted.filterIsInstance<Alert.Notice>().single()
            connections.detach("m1", "s1")
            settle()
            assertTrue(notice.tag in sink.cancelled)
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

    private companion object {
        const val HOLD_MS = 5_000L
    }
}
