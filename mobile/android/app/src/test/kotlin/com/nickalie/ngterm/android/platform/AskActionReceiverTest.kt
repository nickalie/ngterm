package com.nickalie.ngterm.android.platform

import android.app.Application
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.asks.EscapedReply
import com.nickalie.ngterm.android.ui.asks.testAsk
import com.nickalie.ngterm.android.ui.asks.testChoices
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Notification replies follow the same form rules as the in-app question, including escaping and prefills. */
@RunWith(AndroidJUnit4::class)
@Config(application = Application::class)
class AskActionReceiverTest {
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
    fun anUnrelatedBroadcastOrAnApplicationWithoutAGraphLeavesNotificationsAlone() {
        val app = ApplicationProvider.getApplicationContext<Application>()
        val notifications = Notifications(app)
        val manager = app.getSystemService(android.app.NotificationManager::class.java)
        manager.notify("unrelated", 1, notifications.link(listOf("docs-site")))
        val receiver = AskActionReceiver()
        receiver.onReceive(app, Intent("unrelated"))
        receiver.onReceive(
            app,
            Intent(ACTION_ANSWER_ASK)
                .putExtra("machine", "m1")
                .putExtra("session", "s1")
                .putExtra("ask", "A1")
                .putExtra("button", "yes"),
        )
        assertEquals(1, manager.activeNotifications.size)
    }
}
