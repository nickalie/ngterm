package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.ui.theme.CopperTerminal
import com.nickalie.ngterm.android.ui.theme.DarkScheme
import com.nickalie.ngterm.android.ui.theme.DraftingTerminal
import com.nickalie.ngterm.android.ui.theme.GhosttyTerminal
import com.nickalie.ngterm.android.ui.theme.LightScheme
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** The colors drawn by settings follow the chosen side and palette, including changes in the same composition. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SettingsThemeTest {
    @get:Rule val compose = createComposeRule()

    private fun colors(settings: () -> AppSettings) {
        compose.setContent {
            SettingsTheme(settings()) {
                Column {
                    Box(Modifier.size(20.dp).background(MaterialTheme.colorScheme.background).testTag("app"))
                    Box(Modifier.size(20.dp).background(NgtermTheme.terminal.background).testTag("terminal"))
                }
            }
        }
    }

    private fun drawn(tag: String): Color = compose.onNodeWithTag(tag).captureToImage().toPixelMap()[5, 5]

    @Test
    fun pinnedLightAndDarkIgnoreTheSystemAndEveryPaletteCanOverrideTheTheme() {
        var settings by mutableStateOf(AppSettings(theme = ThemeChoice.Light))
        colors { settings }
        for (theme in listOf(ThemeChoice.Light, ThemeChoice.Dark)) {
            for ((choice, terminal) in listOf(
                PaletteChoice.Theme to if (theme == ThemeChoice.Dark) CopperTerminal else DraftingTerminal,
                PaletteChoice.Copper to CopperTerminal,
                PaletteChoice.Drafting to DraftingTerminal,
                PaletteChoice.Ghostty to GhosttyTerminal,
            )) {
                compose.runOnIdle { settings = settings.copy(theme = theme, palette = choice) }
                assertEquals(
                    if (theme ==
                        ThemeChoice.Dark
                    ) {
                        DarkScheme.background
                    } else {
                        LightScheme.background
                    },
                    drawn("app"),
                )
                assertEquals(terminal.background, drawn("terminal"))
            }
        }
    }

    @Test
    @Config(qualifiers = "+notnight")
    fun followingALightSystemUsesDrafting() {
        colors { AppSettings() }
        assertEquals(LightScheme.background, drawn("app"))
        assertEquals(DraftingTerminal.background, drawn("terminal"))
    }

    @Test
    @Config(qualifiers = "+night")
    fun followingADarkSystemUsesCopper() {
        colors { AppSettings() }
        assertEquals(DarkScheme.background, drawn("app"))
        assertEquals(CopperTerminal.background, drawn("terminal"))
    }

    @Test
    fun wallpaperColorsKeepThePinnedTerminalPalette() {
        colors { AppSettings(theme = ThemeChoice.Dark, wallpaperColors = true, palette = PaletteChoice.Ghostty) }
        assertEquals(GhosttyTerminal.background, drawn("terminal"))
        assertEquals(dynamicDarkColorScheme(ApplicationProvider.getApplicationContext()).background, drawn("app"))
    }
}
