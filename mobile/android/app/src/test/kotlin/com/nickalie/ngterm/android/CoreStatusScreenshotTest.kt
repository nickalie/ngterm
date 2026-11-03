package com.nickalie.ngterm.android

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.CoreStatus
import com.nickalie.ngterm.android.ui.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The launch screen, rendered by Robolectric and compared with its golden (src/test/screenshots). */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], qualifiers = "w411dp-h891dp-xxhdpi")
class CoreStatusScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun coreStatusLight() {
        compose.setContent {
            NgtermTheme { CoreStatus(coreVersion = "0.0.0") }
        }
        compose.onRoot().captureRoboImage("src/test/screenshots/core_status_light.png")
    }

    @Test
    @Config(qualifiers = "+night")
    fun coreStatusDark() {
        compose.setContent {
            NgtermTheme { CoreStatus(coreVersion = "0.0.0") }
        }
        compose.onRoot().captureRoboImage("src/test/screenshots/core_status_dark.png")
    }
}
