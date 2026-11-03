package com.nickalie.ngterm.android.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color

// The palette DESIGN.md names: Graphite and Drafting are the grounds, Copper the brand's metal (the copper of
// resources/icons/ngterm.svg), Verdigris its patina, Brass and Oxide the two warnings. Every text and status pair is
// WCAG AA (ContrastTest checks them).

/** The brand's copper, as the launcher mark has it: a fill only, never text (too light for AA on Drafting). */
val Copper = Color(0xFFCF6E31)

private val Graphite = Color(0xFF1B2226)
private val GraphiteLowest = Color(0xFF161C1F)
private val GraphiteLow = Color(0xFF1F272B)
private val GraphiteMid = Color(0xFF242D32)
private val GraphiteHigh = Color(0xFF2A343A)
private val GraphiteHighest = Color(0xFF313C42)
private val Mist = Color(0xFFE3E8EA)
private val MistDim = Color(0xFFA9B4B9)

private val Drafting = Color(0xFFF2F4F5)
private val DraftingLowest = Color(0xFFFFFFFF)
private val DraftingLow = Color(0xFFECEFF0)
private val DraftingMid = Color(0xFFE6EAEB)
private val DraftingHigh = Color(0xFFE0E4E6)
private val DraftingHighest = Color(0xFFD9DFE1)
private val InkDim = Color(0xFF47535A)

private val CopperLight = Color(0xFFE3884F)
private val CopperDeep = Color(0xFF984A19)
private val VerdigrisLight = Color(0xFF5FBFAA)
private val VerdigrisDeep = Color(0xFF1A6A5D)
private val BrassLight = Color(0xFFD9B25A)
private val BrassDeep = Color(0xFF7A5A0A)
private val OxideLight = Color(0xFFF2867A)
private val OxideDeep = Color(0xFFA83226)
private val SlateLight = Color(0xFF9AA6AC)
private val SlateDeep = Color(0xFF535F65)

/** Material 3's roles over the dark ground. */
val DarkScheme: ColorScheme =
    darkColorScheme(
        primary = CopperLight,
        onPrimary = Color(0xFF2A1406),
        primaryContainer = Color(0xFF5A2E12),
        onPrimaryContainer = Color(0xFFFFD9C2),
        secondary = VerdigrisLight,
        onSecondary = Color(0xFF00382F),
        secondaryContainer = Color(0xFF1D4A42),
        onSecondaryContainer = Color(0xFFBDEDE1),
        tertiary = BrassLight,
        onTertiary = Color(0xFF3A2A00),
        tertiaryContainer = Color(0xFF574200),
        onTertiaryContainer = Color(0xFFFFE08F),
        error = OxideLight,
        onError = Color(0xFF4A0D07),
        errorContainer = Color(0xFF6B2119),
        onErrorContainer = Color(0xFFFFDAD4),
        background = Graphite,
        onBackground = Mist,
        surface = Graphite,
        onSurface = Mist,
        surfaceVariant = GraphiteHigh,
        onSurfaceVariant = MistDim,
        surfaceTint = CopperLight,
        surfaceDim = GraphiteLowest,
        surfaceBright = GraphiteHighest,
        surfaceContainerLowest = GraphiteLowest,
        surfaceContainerLow = GraphiteLow,
        surfaceContainer = GraphiteMid,
        surfaceContainerHigh = GraphiteHigh,
        surfaceContainerHighest = GraphiteHighest,
        inverseSurface = Mist,
        inverseOnSurface = Graphite,
        inversePrimary = CopperDeep,
        outline = Color(0xFF7D8A90),
        outlineVariant = Color(0xFF3A464C),
        scrim = Color(0xFF000000),
    )

/** Material 3's roles over the light ground. */
val LightScheme: ColorScheme =
    lightColorScheme(
        primary = CopperDeep,
        onPrimary = Color(0xFFFFFFFF),
        primaryContainer = Color(0xFFFFDCC8),
        onPrimaryContainer = Color(0xFF3A1A06),
        secondary = VerdigrisDeep,
        onSecondary = Color(0xFFFFFFFF),
        secondaryContainer = Color(0xFFC4EDE3),
        onSecondaryContainer = Color(0xFF00201A),
        tertiary = BrassDeep,
        onTertiary = Color(0xFFFFFFFF),
        tertiaryContainer = Color(0xFFFFE08F),
        onTertiaryContainer = Color(0xFF261A00),
        error = OxideDeep,
        onError = Color(0xFFFFFFFF),
        errorContainer = Color(0xFFFFDAD5),
        onErrorContainer = Color(0xFF410001),
        background = Drafting,
        onBackground = Graphite,
        surface = Drafting,
        onSurface = Graphite,
        surfaceVariant = DraftingHigh,
        onSurfaceVariant = InkDim,
        surfaceTint = CopperDeep,
        surfaceDim = DraftingHighest,
        surfaceBright = DraftingLowest,
        surfaceContainerLowest = DraftingLowest,
        surfaceContainerLow = DraftingLow,
        surfaceContainer = DraftingMid,
        surfaceContainerHigh = DraftingHigh,
        surfaceContainerHighest = DraftingHighest,
        inverseSurface = Graphite,
        inverseOnSurface = Mist,
        inversePrimary = CopperLight,
        outline = Color(0xFF6B787F),
        outlineVariant = Color(0xFFC3CCD0),
        scrim = Color(0xFF000000),
    )

/**
 * The status colors, one per [StatusTone]. Each reads as text (4.5:1) on the surfaces up to `surfaceContainerHigh`,
 * and none goes out alone: a [StatusTone]'s glyph and word always come with it.
 */
@Immutable
data class StatusColors(
    val online: Color,
    val connecting: Color,
    val offline: Color,
    val attention: Color,
    val error: Color,
    val idle: Color,
)

/** The status colors over the dark ground. */
val DarkStatus =
    StatusColors(
        online = VerdigrisLight,
        connecting = BrassLight,
        offline = SlateLight,
        attention = Color(0xFFF0904F),
        error = OxideLight,
        idle = MistDim,
    )

/** The status colors over the light ground. */
val LightStatus =
    StatusColors(
        online = VerdigrisDeep,
        connecting = BrassDeep,
        offline = SlateDeep,
        attention = CopperDeep,
        error = OxideDeep,
        idle = InkDim,
    )
