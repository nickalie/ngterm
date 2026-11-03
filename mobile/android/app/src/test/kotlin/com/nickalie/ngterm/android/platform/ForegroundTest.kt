package com.nickalie.ngterm.android.platform

import android.app.Application
import androidx.activity.ComponentActivity
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric

/** The app is on screen while an activity is started; a configuration change is no trip to the background. */
@RunWith(AndroidJUnit4::class)
class ForegroundTest {
    @Test
    fun aStartedActivityIsVisibleAndARecreationNeverLeavesTheScreen() {
        val foreground = Foreground(ApplicationProvider.getApplicationContext<Application>())
        val seen = mutableListOf<Boolean>()
        // every value as it is set, not just the last
        val watch = CoroutineScope(Dispatchers.Unconfined)
        watch.launch { foreground.visible.collect { seen += it } }
        val controller = Robolectric.buildActivity(ComponentActivity::class.java)
        controller.setup()
        assertTrue(foreground.visible.value)
        // a rotation: the activity is stopped and made again
        controller.recreate()
        assertEquals(listOf(false, true), seen)
        controller.pause().stop()
        assertFalse(foreground.visible.value)
        assertEquals(listOf(false, true, false), seen)
        watch.cancel()
    }
}
