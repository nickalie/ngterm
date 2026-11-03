package com.nickalie.ngterm.android.ui.asks

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Intent
import android.os.Bundle
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.core.app.RemoteInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Alert
import com.nickalie.ngterm.android.data.Alerts
import com.nickalie.ngterm.android.data.AskDelivery
import com.nickalie.ngterm.android.data.OpenAsk
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.platform.KEY_ASK_REPLY
import com.nickalie.ngterm.android.platform.Notifications
import com.nickalie.ngterm.android.platform.openTarget
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.RemoteAsk
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val WAIT_MS = 10_000L

/**
 * An ask answered in the app on the fake core: as a sheet while its session is on screen (a choice list with its
 * free-form row, a text field, a dismissal), and from its notification while it isn't (an inline reply through the
 * receiver), and a notification's tap opening its session and pane. Robolectric's default window: under a phone's
 * qualifiers its layout of a text field in a dialog never settles (Task 14).
 */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class AskFlowTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val layout = mutableStateOf(NavLayout.Bar)
    private val key = SessionKey("m1", "s1")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        val app = compose.activity.application as TestApplication
        app.testGraph = test.graph
        shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        Alerts(
            scope,
            test.connections,
            test.asks,
            test.graph.machines.machines,
            test.notificationPrefs.enabled,
            Notifications(app),
        )
    }

    @After
    fun tearDown() {
        scope.cancel()
        test.close()
    }

    private fun waitFor(check: () -> Boolean) = compose.waitUntil(WAIT_MS, check)

    private fun shows(text: String) = compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    private fun answers() = test.core.calls.filter { it.method == "answerAsk" }

    private fun answer(reply: AskReply) = FakeCall("answerAsk", listOf("m1", "s1", "A1", reply))

    // the app with docs-site (s1) on build-01 attached; opened in its view when [open], the Machines list otherwise
    private fun attach(open: Boolean) {
        val appKey = test.seedKey()
        test.seedMachine(testMachine(keyId = appKey.id))
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout.value) }
        }
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory = { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }) }
        compose.onNodeWithText("build-01").performClick()
        waitFor { shows("docs-site") }
        if (open) {
            compose.onNodeWithText("docs-site").performClick()
            waitFor {
                "s1" in
                    test.connections.attached.value["m1"]
                        .orEmpty()
            }
            compose.runOnIdle {
                test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
                test.core.emit(
                    MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)),
                )
            }
            waitFor { test.asks.onScreen.value == key }
        } else {
            compose.runOnIdle { runBlocking { test.connections.attach("m1", "s1") } }
        }
    }

    private fun ask(ask: RemoteAsk) = compose.runOnIdle { test.core.emit(MobileEvent.Ask("m1", "s1", ask)) }

    @Test
    fun aChoiceAskOnScreenIsAnsweredFromItsSheet() {
        attach(open = true)
        ask(testAsk(choices = testChoices(other = "Something else")).copy(message = "Which branch?"))
        waitFor { shows("Deploy to production?") }
        compose.onNodeWithText("Which branch?").assertExists()
        // the free-form row takes the pick while it holds text, and gives it back when cleared
        compose.onNode(hasSetTextAction()).performTextInput("release")
        compose.onNode(hasSetTextAction()).performTextReplacement("")
        compose.onNodeWithText("B").performClick()
        compose.onNodeWithText("yes").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(listOf(answer(AskReply("yes", null, listOf("b")))), answers())
        waitFor { !shows("Deploy to production?") }
    }

    @Test
    fun anInputAskTakesItsTextAndTheCancelButtonEscapes() {
        attach(open = true)
        ask(testAsk(input = RemoteAskInput(false, "main", "Branch")))
        waitFor { shows("main") }
        compose.onNode(hasSetTextAction()).performTextReplacement("release/2.0")
        compose.onNodeWithText("yes").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(answer(AskReply("yes", "release/2.0", null)), answers().single())

        ask(testAsk(input = RemoteAskInput(false, "main", null)).copy(id = "A2"))
        waitFor { shows("main") }
        compose.onNodeWithText("no").performClick()
        waitFor { answers().size == 2 }
        assertEquals(FakeCall("answerAsk", listOf("m1", "s1", "A2", EscapedReply)), answers().last())
    }

    @Test
    fun anAskTheCoreDismissesClosesItsSheet() {
        attach(open = true)
        ask(testAsk())
        waitFor { shows("Deploy to production?") }
        compose.runOnIdle { test.core.emit(MobileEvent.AskDismiss("m1", "s1", "A1")) }
        waitFor { !shows("Deploy to production?") }
        assertEquals(emptyList<FakeCall>(), answers())
    }

    @Test
    fun anAsksDraftOutlivesItsSheetLeavingTheScreen() {
        attach(open = true)
        ask(testAsk(input = RemoteAskInput(false, "main", "Branch")))
        waitFor { shows("main") }
        compose.onNode(hasSetTextAction()).performTextReplacement("release/2.0")
        // the app goes to the background, or another session is opened, and comes back
        compose.runOnIdle { test.asks.showing(null) }
        waitFor { !shows("Deploy to production?") }
        compose.runOnIdle { test.asks.showing(key) }
        waitFor { shows("release/2.0") }
        compose.onNodeWithText("yes").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(answer(AskReply("yes", "release/2.0", null)), answers().single())
    }

    @Test
    fun anActionOfAnAskNoLongerOpenTakesItsNotificationDown() {
        // a notification left from a process the system ended: its ask is open no more
        val app = compose.activity.application
        val stale = Alert.Ask(OpenAsk(key, testAsk()), "build-01", "docs-site", AskDelivery.Actions)
        Notifications(app).post(stale)
        val manager = compose.activity.getSystemService(NotificationManager::class.java)
        val notification: Notification = shadowOf(manager).getNotification(stale.tag, 1)
        notification.actions[0].actionIntent.send()
        shadowOf(Looper.getMainLooper()).idle()
        assertNull(shadowOf(manager).getNotification(stale.tag, 1))
        assertEquals(emptyList<FakeCall>(), answers())
    }

    @Test
    fun aNotificationsTapForASessionNoLongerAttachedStaysOnMachines() {
        attach(open = false)
        compose.runOnIdle { test.connections.detach("m1", "s1") }
        compose.runOnIdle { test.opens.request(OpenTarget("m1", "s1", "P2")) }
        compose.waitForIdle()
        waitFor { test.opens.pending.value == null }
        // the machine's listing, not an empty Sessions
        assertTrue(shows("docs-site"))
        assertEquals(emptySet<PaneRef>(), test.core.shownPanes)
    }

    @Test
    fun anAskOutOfSightIsAnsweredFromItsNotificationsInlineReply() {
        attach(open = false)
        ask(testAsk(input = RemoteAskInput(false, "", "Branch")))
        compose.waitForIdle()
        // the session isn't on screen: no sheet, a notification with an inline reply
        assertEquals(false, shows("Deploy to production?"))
        val manager = compose.activity.getSystemService(NotificationManager::class.java)
        val tag = "ask:m1:s1:A1"
        val notification: Notification = shadowOf(manager).getNotification(tag, 1)
        val reply = notification.actions[0]
        assertEquals(KEY_ASK_REPLY, reply.remoteInputs.single().resultKey)

        val typed = Intent()
        RemoteInput.addResultsToIntent(
            arrayOf(RemoteInput.Builder(KEY_ASK_REPLY).build()),
            typed,
            Bundle().apply { putCharSequence(KEY_ASK_REPLY, "line one\nline two") },
        )
        reply.actionIntent.send(compose.activity, 0, typed)
        shadowOf(Looper.getMainLooper()).idle()
        compose.waitForIdle()
        // a line field's answer has no line break
        assertEquals(listOf(answer(AskReply("yes", "line one line two", null))), answers())
        assertNull(shadowOf(manager).getNotification(tag, 1))
    }

    @Test
    fun aNotificationsTapOpensItsSessionAndPane() {
        attach(open = false)
        ask(testAsk(choices = testChoices()))
        compose.waitForIdle()
        val manager = compose.activity.getSystemService(NotificationManager::class.java)
        val notification: Notification = shadowOf(manager).getNotification("ask:m1:s1:A1", 1)
        val target = openTarget(shadowOf(notification.contentIntent).savedIntent)!!
        compose.runOnIdle {
            test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
            test.core.emit(
                MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)),
            )
            test.opens.request(target.copy(paneId = "P2"))
        }
        // the session view opens on the pane, and the ask, now on screen, is its sheet
        compose.waitForIdle()
        waitFor { PaneRef("m1", "s1", "P2") in test.core.shownPanes }
        // a phone shows the focused pane alone: the first was never shown
        assertFalse(PaneRef("m1", "s1", "P1") in test.core.shownPanes)
        waitFor { shows("Deploy to production?") }
        assertNull(shadowOf(manager).getNotification("ask:m1:s1:A1", 1))
    }
}
