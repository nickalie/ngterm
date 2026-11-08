package com.nickalie.ngterm.android.platform

import android.Manifest
import android.app.Application
import android.app.NotificationManager
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Alert
import com.nickalie.ngterm.android.data.AskDelivery
import com.nickalie.ngterm.android.data.OpenAsk
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.asks.EscapedReply
import com.nickalie.ngterm.android.ui.asks.testAsk
import com.nickalie.ngterm.android.ui.asks.testChoices
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** Notification replies follow the same form rules as the in-app question, including escaping and prefills. */
@RunWith(AndroidJUnit4::class)
@Config(application = Application::class)
class AskActionReceiverTest {
    @get:Rule val dir = TemporaryFolder()

    @Test
    fun replyingWithoutTypingUsesThePrefillAndCancelIgnoresTypedText() {
        val input = testAsk(input = RemoteAskInput(false, "release\nbranch", "Branch"))
        assertEquals(AskReply("yes", "release branch", null), notificationReply(input, "yes", null))
        assertEquals(EscapedReply, notificationReply(input, "no", "deploy anyway"))
        assertEquals(AskReply("yes", "", null), notificationReply(input, "yes", ""))
    }

    @Test
    fun aMultilineReplyKeepsItsLineBreaksAndPlainAndChoiceAsksIgnoreInlineText() {
        val multiline = testAsk(input = RemoteAskInput(true, "", null))
        assertEquals(AskReply("yes", "one\ntwo", null), notificationReply(multiline, "yes", "one\ntwo"))
        assertEquals(AskReply("yes", null, null), notificationReply(testAsk(), "yes", "ignored"))
        val choice = testAsk(choices = testChoices(checked = listOf("b")))
        assertEquals(AskReply("yes", null, listOf("b")), notificationReply(choice, "yes", "ignored"))
    }

    @Test
    fun anApplicationWithoutAGraphLeavesTheAsksNotificationUp() {
        val app = ApplicationProvider.getApplicationContext<Application>()
        shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        val manager = app.getSystemService(NotificationManager::class.java)
        // the very notification a stale ask's action would take down
        val alert = Alert.Ask(OpenAsk(SessionKey("m1", "s1"), testAsk()), "build-01", "docs-site", AskDelivery.Actions)
        Notifications(app).post(alert)
        assertNotNull(shadowOf(manager).getNotification(alert.tag, 1))
        val receiver = AskActionReceiver()
        receiver.onReceive(app, Intent(ACTION_ANSWER_ASK).putExtras(answer()))
        assertNotNull(shadowOf(manager).getNotification(alert.tag, 1))
    }

    @Test
    @Config(application = TestApplication::class)
    fun anUnrelatedBroadcastLeavesAStaleAsksNotificationUpEvenWithAGraph() {
        val app = ApplicationProvider.getApplicationContext<TestApplication>()
        val test = TestGraph(dir.root)
        try {
            app.testGraph = test.graph
            shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
            val manager = app.getSystemService(NotificationManager::class.java)
            val alert =
                Alert.Ask(
                    OpenAsk(SessionKey("m1", "s1"), testAsk()),
                    "build-01",
                    "docs-site",
                    AskDelivery.Actions,
                )
            Notifications(app).post(alert)
            assertNotNull(shadowOf(manager).getNotification(alert.tag, 1))
            val receiver = AskActionReceiver()
            receiver.onReceive(app, Intent("unrelated").putExtras(answer()))
            assertNotNull(shadowOf(manager).getNotification(alert.tag, 1))
            // The same extras with the real action must reach the stale-ask cancellation path.
            receiver.onReceive(app, Intent(ACTION_ANSWER_ASK).putExtras(answer()))
            assertNull(shadowOf(manager).getNotification(alert.tag, 1))
        } finally {
            test.close()
        }
    }

    // the extras of the "yes" action on A1's notification
    private fun answer() =
        Intent()
            .putExtra("machine", "m1")
            .putExtra("session", "s1")
            .putExtra("ask", "A1")
            .putExtra("button", "yes")
}
