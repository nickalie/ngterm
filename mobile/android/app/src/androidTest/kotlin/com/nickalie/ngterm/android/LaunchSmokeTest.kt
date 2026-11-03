package com.nickalie.ngterm.android

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.core.ffi.coreVersion
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The app launches on a device, loads the mobile core's native library through JNA and shows its version. */
@RunWith(AndroidJUnit4::class)
class LaunchSmokeTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    @Test
    fun launchesAndShowsTheCoreVersion() {
        val version = coreVersion()
        assertTrue("the core reports a version", version.isNotBlank())
        compose.onNodeWithText("core $version").assertIsDisplayed()
    }
}
