package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** The terminal's font: the bundled JetBrains Mono (desktop ngterm's and Ghostty's), or the device's monospace. */
enum class FontChoice { JetBrainsMono, SystemMono }

/** The terminal's palette: the theme's (Copper when dark, Drafting when light), or one pinned whatever the theme. */
enum class PaletteChoice { Theme, Copper, Drafting, Ghostty }

/** Light or dark: the system's, or one pinned. */
enum class ThemeChoice { System, Light, Dark }

/**
 * The extra-keys row: [Fit] lays it out for the width (two lines of eight on a phone held upright, one line of
 * sixteen once each key gets 48dp), [TwoLines] keeps two lines at every width, [Hidden] leaves only the soft keyboard.
 */
enum class ExtraKeysChoice { Fit, TwoLines, Hidden }

/**
 * The whole sp the terminal's text may be, and its size until the user picks another: the one definition, which the
 * terminal's `TerminalTextSizes` (pinch and Ctrl+=/-) and `TerminalTextSize` derive from.
 */
val TerminalTextSizeRange = 8..32
const val DEFAULT_TEXT_SIZE = 13

/** The app's settings, as Settings shows them; each starts at its default. */
data class AppSettings(
    val textSize: Int = DEFAULT_TEXT_SIZE,
    val font: FontChoice = FontChoice.JetBrainsMono,
    val palette: PaletteChoice = PaletteChoice.Theme,
    val theme: ThemeChoice = ThemeChoice.System,
    /** The wallpaper's colors for the chrome (Android 12 and later); the status colors and the terminal stay ours. */
    val wallpaperColors: Boolean = false,
    val extraKeys: ExtraKeysChoice = ExtraKeysChoice.Fit,
    /** Keeps the screen on while an attached session is open. */
    val keepScreenOn: Boolean = false,
)

/**
 * The app's settings in [store] (Preferences DataStore). The notification switches are [NotificationPrefs]', and the
 * machines trusted with the clipboard [ClipboardTrust]'s; a value the app no longer knows reads as its default.
 */
class SettingsStore(
    private val store: DataStore<Preferences>,
) {
    /** The settings, on every change. */
    val settings: Flow<AppSettings> = store.data.map(::decode)

    /** Changes the settings by [change]. */
    suspend fun update(change: (AppSettings) -> AppSettings) {
        store.edit { prefs ->
            val now = change(decode(prefs))
            prefs[TEXT_SIZE] = now.textSize.coerceIn(TerminalTextSizeRange)
            prefs[FONT] = now.font.name
            prefs[PALETTE] = now.palette.name
            prefs[THEME] = now.theme.name
            prefs[WALLPAPER] = now.wallpaperColors
            prefs[EXTRA_KEYS] = now.extraKeys.name
            prefs[SCREEN_ON] = now.keepScreenOn
        }
    }

    private companion object {
        val TEXT_SIZE = intPreferencesKey("text_size")
        val FONT = stringPreferencesKey("font")
        val PALETTE = stringPreferencesKey("palette")
        val THEME = stringPreferencesKey("theme")
        val WALLPAPER = booleanPreferencesKey("wallpaper_colors")
        val EXTRA_KEYS = stringPreferencesKey("extra_keys")
        val SCREEN_ON = booleanPreferencesKey("keep_screen_on")

        fun decode(prefs: Preferences): AppSettings {
            val defaults = AppSettings()
            return AppSettings(
                textSize = prefs[TEXT_SIZE]?.coerceIn(TerminalTextSizeRange) ?: defaults.textSize,
                font = prefs[FONT].toEnum(defaults.font),
                palette = prefs[PALETTE].toEnum(defaults.palette),
                theme = prefs[THEME].toEnum(defaults.theme),
                wallpaperColors = prefs[WALLPAPER] ?: defaults.wallpaperColors,
                extraKeys = prefs[EXTRA_KEYS].toEnum(defaults.extraKeys),
                keepScreenOn = prefs[SCREEN_ON] ?: defaults.keepScreenOn,
            )
        }

        inline fun <reified T : Enum<T>> String?.toEnum(default: T): T =
            enumValues<T>().firstOrNull { it.name == this } ?: default
    }
}
