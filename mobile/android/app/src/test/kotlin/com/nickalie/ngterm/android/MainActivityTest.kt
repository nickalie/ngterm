package com.nickalie.ngterm.android

import android.content.Intent
import android.content.res.Configuration
import android.os.Bundle
import android.os.Looper
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.core.view.WindowCompat
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.platform.openIntent
import com.nickalie.ngterm.android.ui.updates.FakeUpdates
import com.nickalie.ngterm.android.update.ACTION_INSTALL_UPDATE
import com.nickalie.ngterm.android.update.ACTION_SHOW_UPDATE
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.launchIn
import kotlinx.coroutines.flow.onEach
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
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.android.controller.ActivityController
import org.robolectric.annotation.Config
import java.io.File

/** Launch and notification intents pass through the real activity, settings collector and Compose app. */
@RunWith(AndroidJUnit4::class)
@Config(
    application = TestApplication::class,
    qualifiers = "w411dp-h891dp-mdpi",
    shadows = [StartupFunctionsShadow::class],
    instrumentedPackages = ["com.nickalie.ngterm.core.ffi"],
)
class MainActivityTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createComposeRule()
    private lateinit var test: TestGraph
    private var controller: ActivityController<MainActivity>? = null

    @Before fun setUp() {
        test = TestGraph(dir.root)
        ApplicationProvider.getApplicationContext<TestApplication>().testGraph = test.graph
    }

    @After fun tearDown() {
        controller?.pause()?.stop()?.destroy()
        test.close()
    }

    private fun launch(
        intent: Intent = Intent(),
        state: Bundle? = null,
    ) {
        controller =
            Robolectric
                .buildActivity(MainActivity::class.java, intent)
                .create(state)
                .start()
                .resume()
                .visible()
    }

    @Test
    fun firstLaunchDrawsTheOnboardingAndANewIntentReachesTheFrame() {
        launch()
        compose.awaitText("Set up a key")
        val target = OpenTarget("m1", "s1", "p1")
        controller!!.newIntent(notificationIntent(target))
        assertEquals(target, test.opens.pending.value)
        controller!!.newIntent(Intent())
        assertEquals(target, test.opens.pending.value)
    }

    @Test
    fun theInitialIntentIsHandledOnceAndRecreationDoesNotReplayIt() {
        val target = OpenTarget("m1", "s1", null)
        launch(notificationIntent(target))
        assertEquals(target, test.opens.pending.value)
        test.opens.consume(target)
        controller!!.pause().stop().destroy()
        launch(notificationIntent(target), Bundle())
        assertNull(test.opens.pending.value)
    }

    @Test
    fun eachLaunchLetsTheUpdatesCheckAndTheirNotificationsIntentsReachThem() {
        val updates = FakeUpdates()
        test.close()
        test = TestGraph(File(dir.root, "updates").apply { mkdirs() }, updates = updates)
        ApplicationProvider.getApplicationContext<TestApplication>().testGraph = test.graph
        launch(Intent(ACTION_SHOW_UPDATE))
        controller!!.newIntent(Intent(ACTION_INSTALL_UPDATE))
        assertEquals(listOf("appStarted", "opened:$ACTION_SHOW_UPDATE", "opened:$ACTION_INSTALL_UPDATE"), updates.calls)
        // a recreated activity neither checks again nor replays its intent
        controller!!.pause().stop().destroy()
        launch(Intent(ACTION_SHOW_UPDATE), Bundle())
        assertEquals(3, updates.calls.size)
    }

    @Test
    fun aStoredThemeDrawsTheAppAndChangesWhileTheActivityIsOpen() {
        test.seedKey()
        runBlocking { test.settings.update { it.copy(theme = ThemeChoice.Dark) } }
        launch()
        compose.awaitText("No machines yet")
        assertFalse(lightBarIcons())
        runBlocking { test.settings.update { it.copy(theme = ThemeChoice.Light) } }
        compose.waitUntil(10_000) {
            compose.waitForIdle()
            lightBarIcons()
        }
        assertTrue(lightBarIcons())
        compose.onNodeWithText("No machines yet").assertExists()
        compose.onNodeWithText("Settings").performClick()
        compose.awaitText("core test-core-version")
    }

    @Test
    fun unreadableSettingsStillAllowTheAppToOpenWithDefaults() {
        test.close()
        val broken = File(dir.root, "broken").apply { mkdirs() }
        File(broken, "settings.preferences_pb").writeBytes(byteArrayOf(-1))
        test = TestGraph(broken)
        ApplicationProvider.getApplicationContext<TestApplication>().testGraph = test.graph
        test.seedKey()
        launch()
        compose.awaitText("No machines yet")
    }

    @Test
    fun aRotationKeepsTheOpenSessionOnScreenWithoutAFalseBackgroundTransition() {
        val machine = testMachine(keyId = test.seedKey().id)
        test.seedMachine(machine)
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory = { _, _, _, _ -> FakePane() }
        test.connections.connect(machine.toCore())
        shadowOf(Looper.getMainLooper()).idle()
        runBlocking { test.connections.attach("m1", "s1") }
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
        test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1"), "P1", null, true)))
        launch(notificationIntent(OpenTarget("m1", "s1", null)))
        val key = SessionKey("m1", "s1")
        compose.waitUntil(10_000) {
            compose.waitForIdle()
            test.asks.onScreen.value == key
        }
        val shown = mutableListOf<SessionKey?>()
        val observer =
            test.asks.onScreen
                .onEach { shown += it }
                .launchIn(CoroutineScope(Dispatchers.Unconfined))
        try {
            controller!!.configurationChange(
                Configuration(controller!!.get().resources.configuration).apply {
                    orientation = Configuration.ORIENTATION_LANDSCAPE
                },
            )
            compose.waitForIdle()
            assertEquals(key, test.asks.onScreen.value)
            assertEquals(listOf(key), shown)
        } finally {
            observer.cancel()
        }
    }

    private fun lightBarIcons(): Boolean {
        val window = controller!!.get().window
        return WindowCompat.getInsetsController(window, window.decorView).isAppearanceLightStatusBars
    }

    private fun notificationIntent(target: OpenTarget): Intent =
        openIntent(ApplicationProvider.getApplicationContext(), target)
}
