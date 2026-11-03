package com.nickalie.ngterm.android.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color

/** A terminal's colors: the ground, the text, the cursor, the selection and the 16 ANSI colors (0-7, then bright). */
@Immutable
data class TerminalPalette(
    val id: String,
    val background: Color,
    val foreground: Color,
    val cursor: Color,
    val selectionBackground: Color,
    val ansi: List<Color>,
) {
    init {
        require(ansi.size == ANSI_COLORS) { "a palette has $ANSI_COLORS ANSI colors, not ${ansi.size}" }
    }

    companion object {
        /** How many ANSI colors a palette sets; the 256-color cube and ramp above them are the terminal's own. */
        const val ANSI_COLORS = 16
    }
}

private const val OPAQUE = 0xFF000000

private fun palette(vararg rgb: Long) = rgb.map { Color(OPAQUE or it) }

/**
 * The dark default: the app's Graphite ground, so the terminal and the chrome around it are one surface, with a
 * copper cursor and verdigris for cyan.
 */
val CopperTerminal =
    TerminalPalette(
        id = "copper",
        background = Color(0xFF1B2226),
        foreground = Color(0xFFE3E8EA),
        cursor = Color(0xFFE3884F),
        selectionBackground = Color(0xFF3D4E57),
        ansi =
            palette(
                0x2A343A,
                0xE5675C,
                0x8CC084,
                0xE3B65A,
                0x6FA3C8,
                0xC08BB5,
                0x5FBFAA,
                0xC9D1D4,
                0x6F7C82,
                0xF2867A,
                0xA6D69B,
                0xF0CB78,
                0x8DBCE0,
                0xD6A5CB,
                0x84D6C4,
                0xF2F5F6,
            ),
    )

/** The light default: the Drafting ground with ink text; the ANSI colors are darkened to read on it. */
val DraftingTerminal =
    TerminalPalette(
        id = "drafting",
        background = Color(0xFFF6F8F8),
        foreground = Color(0xFF1B2226),
        cursor = Color(0xFF984A19),
        selectionBackground = Color(0xFFFFDCC8),
        ansi =
            palette(
                0x1B2226,
                0xB3362A,
                0x3D7A32,
                0x8A6410,
                0x2F64A0,
                0x8A4A85,
                0x1A6A5D,
                0x8B979C,
                0x5B676D,
                0xC8463A,
                0x4F8E43,
                0xA07818,
                0x3D78B8,
                0xA05C9A,
                0x237F70,
                0xA9B4B9,
            ),
    )

/**
 * Ghostty's defaults, as desktop ngterm starts with them: its `#282c34` ground and white text
 * (ngterm-core's `ghostty_config::Colors::default`), and libghostty-vt's built-in 16 colors (`terminal/color.zig`).
 */
val GhosttyTerminal =
    TerminalPalette(
        id = "ghostty",
        background = Color(0xFF282C34),
        foreground = Color(0xFFFFFFFF),
        cursor = Color(0xFFFFFFFF),
        selectionBackground = Color(0xFF3E4451),
        ansi =
            palette(
                0x1D1F21,
                0xCC6666,
                0xB5BD68,
                0xF0C674,
                0x81A2BE,
                0xB294BB,
                0x8ABEB7,
                0xC5C8C6,
                0x666666,
                0xD54E53,
                0xB9CA4A,
                0xE7C547,
                0x7AA6DA,
                0xC397D8,
                0x70C0B1,
                0xEAEAEA,
            ),
    )

/** The palettes the app ships, the dark default first. */
val BuiltInTerminalPalettes = listOf(CopperTerminal, DraftingTerminal, GhosttyTerminal)
