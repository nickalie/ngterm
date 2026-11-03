package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import com.nickalie.ngterm.android.data.AppSettings
import com.nickalie.ngterm.android.data.PaletteChoice
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.ui.theme.CopperTerminal
import com.nickalie.ngterm.android.ui.theme.DraftingTerminal
import com.nickalie.ngterm.android.ui.theme.GhosttyTerminal
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalPalette

/** Whether [settings] want the dark side: pinned, or the system's. */
@Composable
fun isDark(settings: AppSettings): Boolean =
    when (settings.theme) {
        ThemeChoice.System -> isSystemInDarkTheme()
        ThemeChoice.Light -> false
        ThemeChoice.Dark -> true
    }

/** The palette this choice pins, or `null` for the theme's own (Copper when dark, Drafting when light). */
fun PaletteChoice.pinned(): TerminalPalette? =
    when (this) {
        PaletteChoice.Theme -> null
        PaletteChoice.Copper -> CopperTerminal
        PaletteChoice.Drafting -> DraftingTerminal
        PaletteChoice.Ghostty -> GhosttyTerminal
    }

/** [NgtermTheme] as [settings] set it: light or dark, the wallpaper's colors, and the terminal's palette. */
@Composable
fun SettingsTheme(
    settings: AppSettings,
    content: @Composable () -> Unit,
) {
    NgtermTheme(
        darkTheme = isDark(settings),
        dynamicColor = settings.wallpaperColors,
        terminal = settings.palette.pinned(),
        content = content,
    )
}
