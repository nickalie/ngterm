package com.nickalie.ngterm.android

import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The app as it launches on a phone with its key made and no machines yet, rendered by Robolectric and compared with
 * its golden (src/test/screenshots).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [36], qualifiers = "w411dp-h891dp-xxhdpi")
class LaunchScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        test.seedKey()
    }

    @After
    fun tearDown() = test.close()

    private fun launch(golden: String) {
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "0.0.0", layout = NavLayout.Bar) }
        }
        compose.awaitText("No machines yet")
        compose.onRoot().captureRoboImage("src/test/screenshots/$golden.png")
    }

    @Test
    fun launchLight() = launch("launch_light")

    @Test
    @Config(qualifiers = "+night")
    fun launchDark() = launch("launch_dark")
}
