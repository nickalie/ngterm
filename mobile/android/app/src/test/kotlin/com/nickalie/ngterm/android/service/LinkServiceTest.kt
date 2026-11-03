package com.nickalie.ngterm.android.service

import android.Manifest
import android.app.Notification
import android.content.Intent
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.fakeSession
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * The foreground service on the fake core: in the foreground while sessions are attached, counting and naming them;
 * Detach all detaches every one and the service stops; [LinkKeeper] starts it as the first session is attached.
 */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class LinkServiceTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val app = ApplicationProvider.getApplicationContext<TestApplication>()
    private lateinit var test: TestGraph

    @Before
    fun setUp() {
        shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        test = TestGraph(dir.root)
        app.testGraph = test.graph
    }

    @After
    fun tearDown() = test.close()

    private fun idle() = shadowOf(Looper.getMainLooper()).idle()

    // build-01 connected with docs-site and api-refactor attached
    private fun attachTwo() {
        test.core.sessions["m1"] =
            mutableListOf(fakeSession("s1", name = "docs-site"), fakeSession("s2", name = "api-refactor"))
        test.connections.connect(testMachine().toCore())
        idle()
        runBlocking {
            test.connections.attach("m1", "s1")
            test.connections.attach("m1", "s2")
        }
        idle()
    }

    @Test
    fun itRunsInTheForegroundNamingTheAttachedSessionsUntilDetachAll() {
        attachTwo()
        val controller = Robolectric.buildService(LinkService::class.java).create().startCommand(0, 1)
        idle()
        val service = controller.get()
        val shown = shadowOf(service).lastForegroundNotification
        assertEquals("2 sessions attached", shown.extras.getString(Notification.EXTRA_TITLE))
        assertEquals("docs-site, api-refactor", shown.extras.getCharSequence(Notification.EXTRA_TEXT))

        service.onStartCommand(Intent(app, LinkService::class.java).setAction(LinkService.ACTION_DETACH_ALL), 0, 2)
        idle()
        assertEquals(emptyMap<String, Set<String>>(), test.connections.attached.value)
        assertEquals(2, test.core.calls.count { it.method == "detachSession" })
        assertTrue(shadowOf(service).isStoppedBySelf)
    }

    @Test
    fun aServiceCreatedWithNothingAttachedGetsToTheForegroundBeforeItStops() {
        // the sessions went back to none between the keeper's start and the service's creation
        val controller = Robolectric.buildService(LinkService::class.java).create()
        idle()
        val service = controller.get()
        // stopping before startForeground would crash the app
        assertFalse(shadowOf(service).isStoppedBySelf)
        controller.startCommand(0, 1)
        idle()
        assertNotNull(shadowOf(service).lastForegroundNotification)
        assertTrue(shadowOf(service).isStoppedBySelf)
    }

    @Test
    fun theKeeperStartsTheServiceAsTheFirstSessionIsAttached() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        val attached = MutableStateFlow<Map<String, Set<String>>>(emptyMap())
        var starts = 0
        LinkKeeper(scope, attached) { starts += 1 }
        idle()
        attached.value = mapOf("m1" to setOf("s1"))
        attached.value = mapOf("m1" to setOf("s1", "s2"))
        idle()
        assertEquals(1, starts)
        attached.value = emptyMap()
        idle()
        attached.value = mapOf("m2" to setOf("s3"))
        idle()
        assertEquals(2, starts)
        scope.cancel()
    }
}
