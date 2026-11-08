package com.nickalie.ngterm.android.ui.settings

import androidx.activity.ComponentActivity
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Settings controls persist their choices and show the stored selection through the real route and model. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w700dp-h1000dp-mdpi")
class SettingsControlsTest {
    @get:Rule val dir = TemporaryFolder()

    @get:Rule val compose = createAndroidComposeRule<ComponentActivity>()
    private lateinit var test: TestGraph

    @Before fun setUp() {
        test = TestGraph(dir.root)
        compose.setContent { NgtermTheme { SettingsRoute(test.graph, "controls-test") } }
        compose.awaitText("core controls-test")
    }

    @After fun tearDown() = test.close()

    private fun stored(): com.nickalie.ngterm.android.data.AppSettings {
        compose.waitForIdle()
        return runBlocking { test.settings.settings.first() }
    }

    private fun choose(label: String) {
        compose.onNodeWithText(label).performScrollTo().performClick()
    }

    @Test
    fun fontPaletteAndTextSizeChangesAreStoredAndUpdateThePreview() {
        choose("Device monospace")
        compose.waitUntil(10_000) { stored().font == FontChoice.SystemMono }
        choose("JetBrains Mono")
        compose.waitUntil(10_000) { stored().font == FontChoice.JetBrainsMono }
        for ((label, choice) in listOf(
            "Copper" to PaletteChoice.Copper,
            "Drafting" to PaletteChoice.Drafting,
            "Ghostty" to PaletteChoice.Ghostty,
            "Match the theme" to PaletteChoice.Theme,
        )) {
            choose(label)
            compose.waitUntil(10_000) { stored().palette == choice }
            compose.onNodeWithContentDescription("A sample of the terminal in $label at 13 sp").assertExists()
        }
        compose
            .onNodeWithContentDescription("Text size")
            .performScrollTo()
            .performSemanticsAction(SemanticsActions.SetProgress) { it(20f) }
        compose.waitUntil(10_000) { stored().textSize == 20 }
        compose.onNodeWithContentDescription("A sample of the terminal in Match the theme at 20 sp").assertExists()
    }

    @Test
    fun appearanceScreenOnAndExtraKeysChoicesAreStored() {
        for ((label, choice) in listOf(
            "Dark" to ThemeChoice.Dark,
            "Light" to ThemeChoice.Light,
            "Match the system" to ThemeChoice.System,
        )) {
            choose(label)
            compose.waitUntil(10_000) { stored().theme == choice }
        }
        compose
            .onNodeWithText("Keep the screen on")
            .performScrollTo()
            .assertIsOff()
            .performClick()
        compose.waitUntil(10_000) { stored().keepScreenOn }
        compose.onNodeWithText("Keep the screen on").assertIsOn()
        compose
            .onNodeWithText("Wallpaper colors")
            .performScrollTo()
            .assertIsOff()
            .performClick()
        compose.waitUntil(10_000) { stored().wallpaperColors }
        compose.onNodeWithText("Wallpaper colors").assertIsOn()
        for ((label, choice) in listOf(
            "Hidden" to ExtraKeysChoice.Hidden,
            "Always two lines" to ExtraKeysChoice.TwoLines,
            "Fit to the width" to ExtraKeysChoice.Fit,
        )) {
            choose(label)
            compose.waitUntil(10_000) { stored().extraKeys == choice }
        }
        assertEquals(ExtraKeysChoice.Fit, stored().extraKeys)
    }
}
