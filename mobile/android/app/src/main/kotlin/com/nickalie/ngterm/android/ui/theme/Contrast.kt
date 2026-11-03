package com.nickalie.ngterm.android.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import kotlin.math.max
import kotlin.math.min

/** WCAG's minimum for body text. */
const val TEXT_CONTRAST = 4.5f

/** WCAG's minimum for a glyph or a control's edge. */
const val GRAPHIC_CONTRAST = 3f

private const val LUMINANCE_FLARE = 0.05f
private const val STEPS = 20

/** WCAG 2's contrast ratio of two opaque colors, 1 to 21. */
fun contrastRatio(
    first: Color,
    second: Color,
): Float {
    val a = first.luminance() + LUMINANCE_FLARE
    val b = second.luminance() + LUMINANCE_FLARE
    return max(a, b) / min(a, b)
}

/**
 * [color] moved toward black or white, whichever [background] contrasts with more, by the least that makes it reach
 * [minimum] against [background]; [color] itself when it already does. For colors the app doesn't choose: an agent
 * status color the origin sends.
 */
fun readableOn(
    color: Color,
    background: Color,
    minimum: Float = TEXT_CONTRAST,
): Color = readableOn(color, listOf(background), minimum)

/**
 * [color] made to reach [minimum] against every one of [backgrounds] at once, as [readableOn] does for one: for a
 * color that may land on any of them (a badge on the ground, a card or a bar).
 */
fun readableOn(
    color: Color,
    backgrounds: List<Color>,
    minimum: Float = TEXT_CONTRAST,
): Color {
    fun reads(candidate: Color) = backgrounds.all { contrastRatio(candidate, it) >= minimum }
    if (reads(color)) return color
    // the end that contrasts with the worst of the grounds the most
    val target =
        listOf(Color.White, Color.Black).maxBy { end -> backgrounds.minOf { contrastRatio(end, it) } }
    return (1..STEPS)
        .asSequence()
        .map { lerp(color, target, it / STEPS.toFloat()) }
        .firstOrNull(::reads)
        ?: target
}

/**
 * The surfaces text and badges are put on: the ground and the containers up to the high one (cards, bars, banners
 * and dialogs). Every promise of DESIGN.md's color section holds on each of them.
 */
fun ColorScheme.textGrounds(): List<Color> =
    listOf(surface, surfaceContainerLowest, surfaceContainerLow, surfaceContainer, surfaceContainerHigh)

/** The color a status hex (`#rrggbb` or `rrggbb`, as the origin sends it) names, or `null` for anything else. */
fun parseHexColor(hex: String?): Color? =
    hex
        ?.removePrefix("#")
        ?.takeIf { digits ->
            digits.length == HEX_DIGITS &&
                digits.all { it.isDigit() || it.lowercaseChar() in 'a'..'f' }
        }?.let { Color(OPAQUE or it.toLong(radix = 16)) }

private const val HEX_DIGITS = 6
private const val OPAQUE = 0xFF000000
