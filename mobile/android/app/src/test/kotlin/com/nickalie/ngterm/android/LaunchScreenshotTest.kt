package com.nickalie.ngterm.android

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The app as it launches on a phone, rendered by Robolectric and compared with its golden (src/test/screenshots). */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], qualifiers = "w411dp-h891dp-xxhdpi")
class LaunchScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun launchLight() {
        compose.setContent {
            NgtermTheme { NgtermApp(coreVersion = "0.0.0", layout = NavLayout.Bar) }
        }
        compose.onRoot().captureRoboImage("src/test/screenshots/launch_light.png")
    }

    @Test
    @Config(qualifiers = "+night")
    fun launchDark() {
        compose.setContent {
            NgtermTheme { NgtermApp(coreVersion = "0.0.0", layout = NavLayout.Bar) }
        }
        compose.onRoot().captureRoboImage("src/test/screenshots/launch_dark.png")
    }
}
