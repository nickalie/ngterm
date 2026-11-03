package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Canvas
import android.graphics.Paint
import androidx.compose.ui.geometry.Size
import androidx.core.graphics.withClip

/** How the view wants the screen drawn: the palette's selection color, and whether it has focus. */
class TerminalLook(
    val selectionColor: Int,
    val focused: Boolean,
)

/** A run's colors as drawn: defaults resolved, swapped for inverse, the text faded for faint. */
class RunColors(
    val foreground: Int,
    val background: Int,
)

/**
 * Draws a [TerminalScreen] onto a canvas, top left at the origin, as Ghostty's renderer draws a frame:
 * - the ground in the frame's default background, then each run's own background;
 * - the selection, in the palette's selection color, under the text, which keeps its colors;
 * - the text, from each row's [GlyphCache] layout; bold and italic pick the face, faint halves the alpha, inverse
 *   swaps the colors, invisible draws nothing;
 * - the lines and the cursor ([TerminalDecorations]).
 *
 * A run's default colors (`0`) are read from the header being drawn, never from an earlier one, so a change of the
 * defaults (OSC 10/11) recolors rows that were not sent again.
 */
class TerminalPainter(
    private val font: TerminalFont,
    private val glyphs: GlyphCache,
) {
    private val fill = Paint().apply { style = Paint.Style.FILL }
    private val decorations = TerminalDecorations(font)

    /** Draws [screen] filling [size] pixels. */
    fun draw(
        canvas: Canvas,
        screen: TerminalScreen,
        size: Size,
        look: TerminalLook,
    ) {
        val header = screen.header ?: return
        fill.color = header.defaultBackground
        canvas.drawRect(0f, 0f, size.width, size.height, fill)
        val rows = (0 until screen.rowCount).mapNotNull(screen::row)
        for (row in rows) backgrounds(canvas, row, header)
        selection(canvas, screen.selection, look.selectionColor)
        for (row in rows) text(canvas, row, header)
        for (row in rows) {
            for (run in row.runs) {
                if (!run.has(TerminalRun.ATTR_INVISIBLE)) decorations.lines(canvas, row.index, run, colors(run, header))
            }
        }
        cursor(canvas, screen, header, look.focused)
    }

    private fun backgrounds(
        canvas: Canvas,
        row: TerminalRow,
        header: TerminalHeader,
    ) {
        val top = font.top(row.index)
        for (run in row.runs) {
            val color = colors(run, header).background
            if (color == header.defaultBackground) continue
            fill.color = color
            canvas.drawRect(font.left(run.column), top, font.left(run.column + run.span), top + font.cellHeight, fill)
        }
    }

    private fun selection(
        canvas: Canvas,
        spans: List<SelectedSpan>,
        color: Int,
    ) {
        fill.color = color
        for (span in spans) {
            val top = font.top(span.row)
            canvas.drawRect(font.left(span.first), top, font.left(span.last + 1), top + font.cellHeight, fill)
        }
    }

    /** [row]'s text, each run in its colors, or all of it in [color]. */
    private fun text(
        canvas: Canvas,
        row: TerminalRow,
        header: TerminalHeader,
        color: Int? = null,
    ) {
        val baseline = font.top(row.index) + font.baseline
        for (draw in glyphs.layout(row).draws) {
            val run = draw.run
            if (run.has(TerminalRun.ATTR_INVISIBLE)) continue
            val paint = font.paint(run.attributes)
            paint.color = color ?: colors(run, header).foreground
            paint.textScaleX = draw.scaleX
            canvas.drawText(draw.text, draw.x, baseline, paint)
            // the paints are shared with GlyphCache, which measures at the face's own width
            paint.textScaleX = 1f
        }
    }

    private fun cursor(
        canvas: Canvas,
        screen: TerminalScreen,
        header: TerminalHeader,
        focused: Boolean,
    ) {
        val cursor = header.cursor
        if (!cursor.shown || cursor.row >= screen.rowCount) return
        val row = screen.row(cursor.row)
        // on a wide character the cursor covers both its columns, from the first
        val column = if (cursor.wideTail) cursor.column - 1 else cursor.column
        val under = row?.runs?.firstOrNull { column >= it.column && column < it.column + it.span }
        val cell = under?.let { colors(it, header) } ?: RunColors(header.defaultForeground, header.defaultBackground)
        val color = if (header.cursorColor != 0) header.cursorColor else cell.foreground
        val shape = if (focused) cursor.shape else CursorShape.HollowBlock
        val cells = if (under?.wide == true) 2 else 1
        val box = decorations.cursor(canvas, shape, CellBox(cursor.row, column, cells), color)
        if (box != null && row != null) {
            // the character under a block reads in the cell's ground color
            canvas.withClip(box) { text(this, row, header, cell.background) }
        }
    }

    /** [run]'s colors as drawn, its defaults from [header]. */
    private fun colors(
        run: TerminalRun,
        header: TerminalHeader,
    ): RunColors {
        val foreground = if (run.foreground == 0) header.defaultForeground else run.foreground
        val background = if (run.background == 0) header.defaultBackground else run.background
        return if (run.has(TerminalRun.ATTR_INVERSE)) {
            RunColors(faded(background, run), foreground)
        } else {
            RunColors(faded(foreground, run), background)
        }
    }

    private companion object {
        const val RGB = 0x00FFFFFF

        /** Faint text's alpha: half, as Ghostty's `faint-opacity` default. */
        const val FAINT_ALPHA = 0x80 shl 24

        fun faded(
            color: Int,
            run: TerminalRun,
        ): Int = if (run.has(TerminalRun.ATTR_FAINT)) (color and RGB) or FAINT_ALPHA else color
    }
}
