package com.nickalie.ngterm.android.platform

import android.Manifest
import android.app.Application
import android.app.Notification
import android.app.NotificationManager
import androidx.core.app.NotificationCompat
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.data.Alert
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.data.AskDelivery
import com.nickalie.ngterm.android.data.LostReason
import com.nickalie.ngterm.android.data.OpenAsk
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.askTag
import com.nickalie.ngterm.android.ui.asks.testAsk
import com.nickalie.ngterm.android.ui.asks.testChoices
import com.nickalie.ngterm.core.ffi.RemoteAsk
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The app's alerts as Android notifications: channels, words, actions, inline replies and where a tap leads. */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class NotificationsTest {
    private val app = ApplicationProvider.getApplicationContext<Application>()
    private val manager = app.getSystemService(NotificationManager::class.java)
    private val notifications = Notifications(app)
    private val key = SessionKey("m1", "s1")

    @Before
    fun grant() = shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)

    private fun posted(tag: String): Notification? = shadowOf(manager).getNotification(tag, 1)

    private fun askAlert(
        delivery: AskDelivery,
        ask: RemoteAsk = testAsk().copy(message = "The tests passed."),
    ) = Alert.Ask(OpenAsk(key, ask), "build-01", "docs-site", delivery)

    @Test
    fun anAlertGoesToItsMachinesChannelAndItsTapOpensItsSessionAndPane() {
        val alert = Alert.Status(key, "build-01", "docs-site", AlertKind.Waiting, "Rewrite the install guide", "P2")
        notifications.post(alert)
        val notification = posted(alert.tag)!!
        assertEquals("machine:m1", notification.channelId)
        assertEquals("build-01", manager.getNotificationChannel("machine:m1").name)
        assertEquals("docs-site is waiting for you", notification.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("Rewrite the install guide", notification.extras.getCharSequence(Notification.EXTRA_TEXT))
        val tap = shadowOf(notification.contentIntent).savedIntent
        assertEquals(OpenTarget("m1", "s1", "P2"), openTarget(tap))

        notifications.cancel(alert.tag)
        assertNull(posted(alert.tag))
    }

    @Test
    fun noticesAndLostMachinesSayWhatHappened() {
        val notice = Alert.Notice("notice:1", key, "build-01", "docs-site", "Build done", "All tests passed", null)
        notifications.post(notice)
        val posted = posted("notice:1")!!
        assertEquals("Build done", posted.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("docs-site", posted.extras.getString(Notification.EXTRA_SUB_TEXT))

        val lost = Alert.Lost("m1", "build-01", LostReason.HostKeyChanged)
        notifications.post(lost)
        val stop = posted(lost.tag)!!
        assertEquals("Lost build-01", stop.extras.getString(Notification.EXTRA_TITLE))
        assertEquals(OpenTarget("m1"), openTarget(shadowOf(stop.contentIntent).savedIntent))
    }

    @Test
    fun aButtonAskHasAnActionPerButton() {
        val alert = askAlert(AskDelivery.Actions)
        notifications.post(alert)
        val notification = posted(askTag(alert.open))!!
        assertEquals("Deploy to production?", notification.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("docs-site on build-01", notification.extras.getString(Notification.EXTRA_SUB_TEXT))
        assertEquals(listOf("yes", "no"), notification.actions.map { it.title.toString() })
        assertNull(notification.actions[0].remoteInputs)
        val intent = shadowOf(notification.actions[1].actionIntent).savedIntent
        assertEquals(AskAction("m1", "s1", "A1", "no"), askAction(intent))
    }

    @Test
    fun anInputAskRepliesInlineAndCancelsPlainly() {
        val ask = testAsk(input = RemoteAskInput(false, "", "Branch name"))
        val alert = askAlert(AskDelivery.Reply, ask)
        notifications.post(alert)
        val actions = posted(askTag(alert.open))!!.actions
        assertEquals("Branch name", actions[0].remoteInputs.single().label)
        assertEquals(KEY_ASK_REPLY, actions[0].remoteInputs.single().resultKey)
        // the cancel button escapes: nothing to type
        assertNull(actions[1].remoteInputs)
    }

    @Test
    fun aChoiceAskOpensTheApp() {
        val alert = askAlert(AskDelivery.OpenApp, testAsk(choices = testChoices()))
        notifications.post(alert)
        val notification = posted(askTag(alert.open))!!
        assertNull(notification.actions)
        assertEquals("Tap to answer in ngterm.", notification.extras.getCharSequence(Notification.EXTRA_TEXT))
        assertEquals(OpenTarget("m1", "s1", null), openTarget(shadowOf(notification.contentIntent).savedIntent))
        // it stays up until it is answered: a tap only opens the app
        assertEquals(0, notification.flags and Notification.FLAG_AUTO_CANCEL)
    }

    @Test
    fun nothingIsPostedWithoutThePermission() {
        shadowOf(app).denyPermissions(Manifest.permission.POST_NOTIFICATIONS)
        val alert = askAlert(AskDelivery.Actions)
        notifications.post(alert)
        assertNull(posted(askTag(alert.open)))
    }

    @Test
    fun theServicesNotificationCountsTheSessionsAndDetachesAll() {
        val link = notifications.link(listOf("docs-site", "api-refactor"))
        assertEquals("links", link.channelId)
        assertEquals("2 sessions attached", link.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("docs-site, api-refactor", link.extras.getCharSequence(Notification.EXTRA_TEXT))
        assertEquals(listOf("Detach all"), link.actions.map { it.title.toString() })
        assertEquals(NotificationCompat.CATEGORY_SERVICE, link.category)
        assertEquals(
            "1 session attached",
            notifications.link(listOf("docs-site")).extras.getString(Notification.EXTRA_TITLE),
        )
    }
}
