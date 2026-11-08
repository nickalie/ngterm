package com.nickalie.ngterm.android.ui.theme

import androidx.compose.material3.ColorScheme
import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** DESIGN.md's contrast promises, checked: WCAG AA for every text pair and status color, light and dark. */
class ContrastTest {
    private val sides = listOf("light" to (LightScheme to LightStatus), "dark" to (DarkScheme to DarkStatus))

    private val groundNames =
        listOf("surface", "surfaceContainerLowest", "surfaceContainerLow", "surfaceContainer", "surfaceContainerHigh")

    // the surfaces text and badges are put on, by name
    private fun ColorScheme.grounds() = groundNames.zip(textGrounds())

    private fun assertContrast(
        what: String,
        foreground: Color,
        background: Color,
        minimum: Float = TEXT_CONTRAST,
    ) {
        val ratio = contrastRatio(foreground, background)
        assertTrue("$what: $ratio < $minimum", ratio >= minimum)
    }

    @Test
    fun everyStatusColorReadsAsTextOnEverySurface() {
        for ((side, pair) in sides) {
            val (scheme, status) = pair
            for (tone in StatusTone.entries) {
                for ((name, ground) in scheme.grounds()) {
                    assertContrast("$side ${tone.name} on $name", tone.color(status), ground)
                }
            }
        }
    }

    @Test
    fun everyAgentStateDefaultReadsAsTextOnEverySurface() {
        for ((side, pair) in sides) {
            val (scheme, status) = pair
            for (state in AgentState.entries) {
                for ((name, ground) in scheme.grounds()) {
                    assertContrast("$side ${state.name} on $name", state.color(status), ground)
                }
            }
        }
    }

    @Test
    fun everyTextRoleReadsOnItsContainer() {
        for ((side, pair) in sides) {
            val scheme = pair.first
            val roles =
                listOf(
                    "onPrimary/primary" to (scheme.onPrimary to scheme.primary),
                    "onPrimaryContainer/primaryContainer" to (scheme.onPrimaryContainer to scheme.primaryContainer),
                    "onSecondary/secondary" to (scheme.onSecondary to scheme.secondary),
                    "onSecondaryContainer/secondaryContainer" to
                        (scheme.onSecondaryContainer to scheme.secondaryContainer),
                    "onTertiary/tertiary" to (scheme.onTertiary to scheme.tertiary),
                    "onTertiaryContainer/tertiaryContainer" to (scheme.onTertiaryContainer to scheme.tertiaryContainer),
                    "onError/error" to (scheme.onError to scheme.error),
                    "onErrorContainer/errorContainer" to (scheme.onErrorContainer to scheme.errorContainer),
                    "inverseOnSurface/inverseSurface" to (scheme.inverseOnSurface to scheme.inverseSurface),
                )
            for ((name, colors) in roles) assertContrast("$side $name", colors.first, colors.second)
            for ((name, ground) in scheme.grounds() + ("surfaceContainerHighest" to scheme.surfaceContainerHighest)) {
                assertContrast("$side onSurface on $name", scheme.onSurface, ground)
                assertContrast("$side onSurfaceVariant on $name", scheme.onSurfaceVariant, ground)
            }
            for ((name, ground) in scheme.grounds()) {
                // primary and secondary are text too: the selected destination, a text button, the Attached mark
                assertContrast("$side primary on $name", scheme.primary, ground)
                assertContrast("$side secondary on $name", scheme.secondary, ground)
                assertContrast("$side error on $name", scheme.error, ground)
                // a control's edge is a graphic
                assertContrast("$side outline on $name", scheme.outline, ground, GRAPHIC_CONTRAST)
            }
        }
    }

    @Test
    fun everyTerminalPaletteReadsAndKeepsItsCursorVisible() {
        for (palette in BuiltInTerminalPalettes) {
            assertContrast("${palette.id} foreground", palette.foreground, palette.background)
            assertContrast("${palette.id} cursor", palette.cursor, palette.background, GRAPHIC_CONTRAST)
            assertContrast("${palette.id} text on the selection", palette.foreground, palette.selectionBackground)
        }
    }

    @Test
    fun readableOnLeavesAReadableColorAloneAndFixesAnUnreadableOne() {
        val ground = DarkScheme.surface
        val readable = Color(0xFF5FBFAA)
        assertEquals(readable, readableOn(readable, ground))
        val dim = Color(0xFF222222)
        val fixed = readableOn(dim, ground)
        assertTrue(contrastRatio(fixed, ground) >= TEXT_CONTRAST)
        assertTrue("moved toward white on a dark ground", fixed.red > dim.red)
        val pale = readableOn(Color(0xFFF0F0F0), LightScheme.surface)
        assertTrue(contrastRatio(pale, LightScheme.surface) >= TEXT_CONTRAST)
    }

    @Test
    fun anOriginsColorIsMadeReadableOnEverySurfaceAtOnce() {
        // a grid over the color cube, and two colors a lift against the ground alone left unreadable on a card
        val levels = (0..GRID_STEPS).map { it / GRID_STEPS.toFloat() }
        val colors =
            levels.flatMap { r -> levels.flatMap { g -> levels.map { b -> Color(r, g, b) } } } +
                listOf("#f84050", "#f8f8f0").map { checkNotNull(parseHexColor(it)) }
        for ((side, pair) in sides) {
            val grounds = pair.first.grounds()
            for (color in colors) {
                val lifted = readableOn(color, grounds.map { it.second })
                for ((name, ground) in grounds) assertContrast("$side $color lifted on $name", lifted, ground)
            }
        }
    }

    @Test
    fun parseHexColorTakesTheOriginsSpellingsAndNothingElse() {
        assertEquals(Color(0xFFFF8800), parseHexColor("#ff8800"))
        assertEquals(Color(0xFFFF8800), parseHexColor("FF8800"))
        assertNull(parseHexColor(null))
        assertNull(parseHexColor("red"))
        assertNull(parseHexColor("#fff"))
        assertNull(parseHexColor("#gg0000"))
    }

    @Test
    fun aStatusShapeFollowsTheOriginsWireNames() {
        assertEquals(StatusShape.Star, StatusShape.fromWire("star"))
        assertEquals(StatusShape.Capsule, StatusShape.fromWire("capsule"))
        assertEquals(StatusShape.Circle, StatusShape.fromWire(null))
        assertEquals(StatusShape.Circle, StatusShape.fromWire("hexagon"))
        // the app's own glyphs are not wire names
        assertEquals(StatusShape.Circle, StatusShape.fromWire("ring"))
        assertEquals(StatusShape.Circle, StatusShape.fromWire("slash"))
        // the shapes upstream added in v0.41.0, triangle-down by its public name
        assertEquals(StatusShape.TriangleDown, StatusShape.fromWire("triangle-down"))
        assertEquals(StatusShape.Circle, StatusShape.fromWire("triangledown"))
        assertEquals(StatusShape.Play, StatusShape.fromWire("play"))
        assertEquals(StatusShape.Seal, StatusShape.fromWire("seal"))
        assertEquals(StatusShape.Bolt, StatusShape.fromWire("bolt"))
        assertEquals(StatusShape.Bell, StatusShape.fromWire("Bell"))
    }

    @Test
    fun everyWireShapeHasItsOwnGlyph() {
        val names = "circle square triangle triangle-down play diamond capsule seal star bolt bell"
        val wire = names.split(" ").map { StatusShape.fromWire(it) }
        assertEquals("each of upstream's eleven shapes reads as its own", wire.size, wire.toSet().size)
        assertEquals("and draws its own drawable", wire.size, wire.map { it.drawable }.toSet().size)
    }

    @Test
    fun everyToneHasItsOwnGlyph() {
        val shapes = StatusTone.entries.map { it.shape }
        assertEquals("no two tones share a glyph, so none rests on color", shapes.size, shapes.toSet().size)
    }
}

private const val GRID_STEPS = 16
