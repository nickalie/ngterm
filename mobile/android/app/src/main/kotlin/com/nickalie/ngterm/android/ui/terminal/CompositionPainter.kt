package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Canvas
import android.graphics.Paint
import android.icu.lang.UCharacter
import android.icu.lang.UProperty
import android.icu.text.BreakIterator
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.toArgb
import androidx.core.graphics.withTranslation
import com.nickalie.ngterm.android.ui.theme.TerminalPalette

/**
 * One character of the text waiting at the cursor, in the [span] cells from [row] and [column] it will take; empty for
 * cells it leaves blank.
 */
internal data class HeldCell(
    val text: String,
    val row: Int,
    val column: Int,
    val span: Int,
)

/**
 * The text waiting at a terminal's cursor ([TerminalState.composing]) placed as the terminal will place it once typed:
 * each character in its cells from the cursor on, onto the next row when it doesn't fit before the right edge. [lift]
 * is how many rows the screen moves up to make room for rows it takes below the last, as the terminal would scroll.
 */
internal class HeldLayout(
    val cells: List<HeldCell>,
    val lift: Int,
) {
    companion object {
        val None = HeldLayout(emptyList(), 0)

        /** [text] laid out from [header]'s cursor; [None] while the cursor is out of view. */
        fun of(
            text: String,
            header: TerminalHeader,
        ): HeldLayout {
            val cursor = header.cursor
            if (text.isEmpty() || !cursor.inView || header.columns < 1) return None
            var row = cursor.row
            var column = cursor.column
            val cells = mutableListOf<HeldCell>()
            for (cluster in clusters(text)) {
                val span = cluster.cells().coerceAtMost(header.columns)
                if (column + span > header.columns) {
                    // a wide character the row has no room for leaves the rest of it blank, as Ghostty's spacer does
                    if (column < header.columns) cells += HeldCell("", row, column, header.columns - column)
                    row++
                    column = 0
                }
                cells += HeldCell(cluster, row, column, span)
                column += span
            }
            return HeldLayout(cells, (row - header.rows + 1).coerceIn(0, header.rows))
        }

        private fun clusters(text: String): List<String> {
            val breaks = BreakIterator.getCharacterInstance().also { it.setText(text) }
            val clusters = mutableListOf<String>()
            var start = breaks.first()
            var end = breaks.next()
            while (end != BreakIterator.DONE) {
                clusters += text.substring(start, end)
                start = end
                end = breaks.next()
            }
            return clusters
        }

        // Ghostty's width: the first code point's, wide for an emoji a variation selector asks to show as one
        private fun String.cells(): Int {
            val width = UCharacter.getIntPropertyValue(codePointAt(0), UProperty.EAST_ASIAN_WIDTH)
            val wide = width == UCharacter.EastAsianWidth.WIDE || width == UCharacter.EastAsianWidth.FULLWIDTH
            return if (wide || EMOJI_PRESENTATION in this) 2 else 1
        }

        private const val EMOJI_PRESENTATION = '\uFE0F'
    }
}

/**
 * Draws a terminal's screen with [painter] and what waits at its cursor over it, as [HeldLayout] places it: each
 * character on the terminal's ground in its text color, centered in its cells (squeezed into them when wider), which
 * it covers whole, and underlined in the cursor's color.
 */
internal class CompositionPainter(
    private val painter: TerminalPainter,
    private val font: TerminalFont,
    /** The underline's thickness in pixels. */
    private val underline: Float,
) {
    private val fill = Paint()

    /** Draws [state]'s screen and what waits at its cursor on [canvas], filling [size] pixels. */
    fun draw(
        canvas: Canvas,
        state: TerminalState,
        size: Size,
        look: TerminalLook,
        palette: TerminalPalette,
    ) {
        val header = state.screen.header
        val held = header?.let { HeldLayout.of(state.composing, it) } ?: HeldLayout.None
        val lift = held.lift * font.cellHeight.toFloat()
        canvas.withTranslation(y = -lift) {
            painter.draw(this, state.screen, size.copy(height = size.height + lift), look)
            if (header != null) held(this, held, header, palette)
        }
    }

    private fun held(
        canvas: Canvas,
        layout: HeldLayout,
        header: TerminalHeader,
        palette: TerminalPalette,
    ) {
        val paint = font.paint(0)
        for (cell in layout.cells) {
            val left = font.left(cell.column)
            val room = font.left(cell.column + cell.span) - left
            val top = font.top(cell.row)
            val bottom = top + font.cellHeight
            fill.color = header.defaultBackground
            canvas.drawRect(left, top, left + room, bottom, fill)
            if (cell.text.isEmpty()) continue
            val width = paint.measureText(cell.text)
            paint.color = header.defaultForeground
            paint.textScaleX = if (width > room) room / width else 1f
            val x = if (width > room) left else left + (room - width) / 2
            canvas.drawText(cell.text, x, top + font.baseline, paint)
            paint.textScaleX = 1f
            fill.color = palette.cursor.toArgb()
            canvas.drawRect(left, bottom - underline, left + room, bottom, fill)
        }
    }
}
