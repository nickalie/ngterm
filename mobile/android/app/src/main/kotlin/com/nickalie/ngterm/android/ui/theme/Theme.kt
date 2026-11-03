package com.nickalie.ngterm.android.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalContext

/** The tokens Material 3's theme has no slot for (DESIGN.md). */
@Immutable
data class NgtermTokens(
    val status: StatusColors,
    val mono: MonoTypography,
    val terminal: TerminalPalette,
    val spacing: Spacing = Spacing(),
    val motion: Motion = Motion(),
)

private val LocalNgtermTokens = staticCompositionLocalOf { NgtermTokens(DarkStatus, NgtermMono, CopperTerminal) }

/**
 * ngterm's theme: Material 3 with the app's palette, type and shapes, light or dark with the system. [dynamicColor]
 * takes the wallpaper's colors instead (Android 12 and later) for Material's roles; the status colors, the type and the
 * terminal palette stay the app's. The terminal's palette is [terminal] when Settings pins one, else Copper or
 * Drafting with the light or dark side.
 */
@Composable
fun NgtermTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = false,
    terminal: TerminalPalette? = null,
    content: @Composable () -> Unit,
) {
    val scheme =
        when {
            dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
                val context = LocalContext.current
                if (darkTheme) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            }

            darkTheme -> {
                DarkScheme
            }

            else -> {
                LightScheme
            }
        }
    val tokens =
        NgtermTokens(
            status = if (darkTheme) DarkStatus else LightStatus,
            mono = NgtermMono,
            terminal = terminal ?: if (darkTheme) CopperTerminal else DraftingTerminal,
        )
    CompositionLocalProvider(LocalNgtermTokens provides tokens) {
        MaterialTheme(
            colorScheme = scheme,
            typography = NgtermTypography,
            shapes = NgtermShapes,
            content = content,
        )
    }
}

/** The app's own tokens, as [NgtermTheme] provides them; Material's are `MaterialTheme`'s. */
object NgtermTheme {
    /** The status colors. */
    val status: StatusColors
        @Composable @ReadOnlyComposable
        get() = LocalNgtermTokens.current.status

    /** The mono type roles. */
    val mono: MonoTypography
        @Composable @ReadOnlyComposable
        get() = LocalNgtermTokens.current.mono

    /** The terminal's palette: the one Settings pinned, or the default for the theme's light or dark side. */
    val terminal: TerminalPalette
        @Composable @ReadOnlyComposable
        get() = LocalNgtermTokens.current.terminal

    /** The spacing grid. */
    val spacing: Spacing
        @Composable @ReadOnlyComposable
        get() = LocalNgtermTokens.current.spacing

    /** The motion tokens. */
    val motion: Motion
        @Composable @ReadOnlyComposable
        get() = LocalNgtermTokens.current.motion
}
