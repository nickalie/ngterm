package com.nickalie.ngterm.android.ui.settings

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The screen's loading and failure states, and the API 26 layout without wallpaper controls. */
@RunWith(AndroidJUnit4::class)
class SettingsStatesTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun readingSettingsShowsProgressAndAFailureOffersRetry() {
        var state: SettingsUiState by mutableStateOf(SettingsUiState.Loading)
        val ready = SettingsUiState.Ready(AppSettings(), emptySet(), emptyList(), emptyList())
        val actions = SettingsActions({}, { _, _ -> }, {}, {}, {}, {}, {}, { state = ready })
        compose.setContent {
            NgtermTheme { SettingsScreen(state, "test", true, KeyWork(), actions) }
        }
        compose.onNodeWithContentDescription("Reading the settings").assertIsDisplayed()
        compose.runOnIdle { state = SettingsUiState.Failed("Storage unavailable") }
        compose.onNodeWithText("Can't read the settings").assertIsDisplayed()
        compose.onNodeWithText("Storage unavailable.", substring = true).assertIsDisplayed()
        compose.onNodeWithText("Retry").performClick()
        compose.onNodeWithText("core test").assertExists()
    }

    @Test
    @Config(sdk = [26], qualifiers = "w700dp-h1000dp-mdpi")
    fun android8OffersPinnedThemesButNoWallpaperColors() {
        val ready = SettingsUiState.Ready(AppSettings(), emptySet(), emptyList(), emptyList())
        val actions = SettingsActions({}, { _, _ -> }, {}, {}, {}, {}, {}, {})
        compose.setContent {
            NgtermTheme { SettingsScreen(ready, "test", true, KeyWork(), actions) }
        }
        compose.onNodeWithText("Dark").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Wallpaper colors").assertDoesNotExist()
    }
}
