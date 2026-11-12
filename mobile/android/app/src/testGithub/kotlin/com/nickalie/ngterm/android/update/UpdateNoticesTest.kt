package com.nickalie.ngterm.android.update

import android.Manifest
import android.app.Application
import android.app.Notification
import android.app.NotificationManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.work.ListenableWorker
import androidx.work.NetworkType
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.testing.TestListenableWorkerBuilder
import androidx.work.testing.WorkManagerTestInitHelper
import com.nickalie.ngterm.android.TestApplication
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import java.io.IOException

/** The update notification, and the background checks WorkManager runs. */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class UpdateNoticesTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val app = ApplicationProvider.getApplicationContext<TestApplication>()
    private val manager = app.getSystemService(NotificationManager::class.java)
    private val offer = UpdateOffer("0.3.0", 1_000, "${RELEASE_PAGES}tag/v0.3.0")

    private fun posted(): Notification? = shadowOf(manager).getNotification("update", 3)

    @Test
    fun aNewVersionIsAnnouncedWithInstallAndNotNow() {
        shadowOf(app as Application).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        val notices = UpdateNotifications(app)
        notices.available(offer)
        val notification = posted()!!
        assertEquals(UPDATES_CHANNEL, notification.channelId)
        assertEquals("App updates", manager.getNotificationChannel(UPDATES_CHANNEL).name)
        assertEquals("ngterm 0.3.0 is available", notification.extras.getString(Notification.EXTRA_TITLE))
        assertEquals(ACTION_SHOW_UPDATE, shadowOf(notification.contentIntent).savedIntent.action)
        val (install, skip) = notification.actions.toList()
        assertEquals("Install", install.title)
        assertEquals(ACTION_INSTALL_UPDATE, shadowOf(install.actionIntent).savedIntent.action)
        assertEquals("Not now", skip.title)
        val skipIntent = shadowOf(skip.actionIntent).savedIntent
        assertEquals(ACTION_SKIP_UPDATE, skipIntent.action)
        assertEquals("0.3.0", skipIntent.getStringExtra("version"))

        notices.clear()
        assertNull(posted())
    }

    @Test
    fun nothingIsPostedWhileTheUserHasNotificationsOff() {
        shadowOf(app as Application).denyPermissions(Manifest.permission.POST_NOTIFICATIONS)
        UpdateNotifications(app).available(offer)
        assertNull(posted())
    }

    @Test
    fun theBackgroundCheckRunsTwiceADayOnANetworkWhileTheBatteryLasts() {
        WorkManagerTestInitHelper.initializeTestWorkManager(app)
        val work = WorkManager.getInstance(app)
        val schedule = WorkSchedule(app)
        schedule.start()
        schedule.start()
        val info = work.getWorkInfosForUniqueWork(UPDATE_WORK).get().single()
        assertEquals(WorkInfo.State.ENQUEUED, info.state)
        assertEquals(CHECK_HOURS * 3_600_000, info.periodicityInfo!!.repeatIntervalMillis)
        assertTrue(info.constraints.requiresBatteryNotLow())
        assertEquals(NetworkType.CONNECTED, info.constraints.requiredNetworkType)
        schedule.stop()
        assertEquals(
            WorkInfo.State.CANCELLED,
            work
                .getWorkInfosForUniqueWork(UPDATE_WORK)
                .get()
                .single()
                .state,
        )
    }

    @Test
    fun theWorkerChecksAndIsRetriedWhenGitHubCantBeReached() {
        val worker = TestListenableWorkerBuilder<UpdateCheckWorker>(app).build()
        app.updates = null
        assertEquals(ListenableWorker.Result.success(), runBlocking { worker.doWork() })

        val rig = UpdateRig(dir.root)
        app.updates = rig.updates()
        rig.http.releases(releaseJson("0.3.0"))
        assertEquals(ListenableWorker.Result.success(), runBlocking { worker.doWork() })
        assertEquals(listOf("0.3.0"), rig.announced)
        rig.http.failure = IOException("offline")
        assertEquals(ListenableWorker.Result.retry(), runBlocking { worker.doWork() })
    }
}
