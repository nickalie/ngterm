package com.nickalie.ngterm.android.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.runtime.Immutable
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R

// Atkinson Hyperlegible Next for the chrome, Atkinson Hyperlegible Mono for what the user compares character by
// character (hosts, ports, fingerprints, key lines): one family, built by the Braille Institute to tell 0 from O and
// l from 1 from I. Both are the variable fonts (wght 200-800) Google Fonts ships, under the OFL
// (mobile/android/licenses/). The sizes follow the classic scale (11, 12, 14, 16, 18, 21, 24, 36, 48).

private fun variable(
    resource: Int,
    weight: FontWeight,
) = Font(resource, weight, variationSettings = FontVariation.Settings(FontVariation.weight(weight.weight)))

private val weights =
    listOf(FontWeight.Light, FontWeight.Normal, FontWeight.Medium, FontWeight.SemiBold, FontWeight.Bold)

/** The chrome's family. */
val Atkinson = FontFamily(weights.map { variable(R.font.atkinson_hyperlegible_next, it) })

/** The family for what is compared character by character. */
val AtkinsonMono = FontFamily(weights.map { variable(R.font.atkinson_hyperlegible_mono, it) })

private fun style(
    size: Int,
    line: Int,
    weight: FontWeight,
    tracking: Double = 0.0,
) = TextStyle(
    fontFamily = Atkinson,
    fontWeight = weight,
    fontSize = size.sp,
    lineHeight = line.sp,
    letterSpacing = tracking.sp,
)

/** Material 3's type roles on the scale. Display sizes are light, titles and labels semibold, body regular. */
val NgtermTypography =
    Typography(
        displayLarge = style(48, 56, FontWeight.Light, tracking = -0.5),
        displayMedium = style(36, 44, FontWeight.Light, tracking = -0.25),
        displaySmall = style(30, 38, FontWeight.Normal),
        headlineLarge = style(30, 38, FontWeight.Medium),
        headlineMedium = style(24, 32, FontWeight.Medium),
        headlineSmall = style(21, 28, FontWeight.Medium),
        titleLarge = style(18, 24, FontWeight.SemiBold),
        titleMedium = style(16, 22, FontWeight.SemiBold),
        titleSmall = style(14, 20, FontWeight.SemiBold),
        bodyLarge = style(16, 24, FontWeight.Normal),
        bodyMedium = style(14, 20, FontWeight.Normal),
        bodySmall = style(12, 16, FontWeight.Normal),
        labelLarge = style(14, 20, FontWeight.SemiBold),
        labelMedium = style(12, 16, FontWeight.SemiBold),
        labelSmall = style(11, 16, FontWeight.SemiBold, tracking = 0.2),
    )

/** The mono roles Material 3 has no slot for. */
@Immutable
data class MonoTypography(
    /** A host, a port, a fingerprint in a row or a card. */
    val small: TextStyle,
    /** The same, given room: a confirmation sheet, a key line. */
    val medium: TextStyle,
)

/** The mono roles on the scale. */
val NgtermMono =
    MonoTypography(
        small =
            TextStyle(
                fontFamily = AtkinsonMono,
                fontWeight = FontWeight.Normal,
                fontSize = 12.sp,
                lineHeight = 16.sp,
            ),
        medium =
            TextStyle(
                fontFamily = AtkinsonMono,
                fontWeight = FontWeight.Normal,
                fontSize = 14.sp,
                lineHeight = 20.sp,
            ),
    )
