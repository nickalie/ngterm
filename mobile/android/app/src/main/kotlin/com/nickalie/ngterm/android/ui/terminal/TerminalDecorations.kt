package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Path
import android.graphics.RectF
import kotlin.math.max

/** [cells] cells of row [row] from [column]. */
class CellBox(
    val row: Int,
    val column: Int,
    val cells: Int,
)

/**
 * The lines a run carries (strikethrough, overline, and the five underlines: single, double, curly, dotted, dashed,
 * in the underline color when the program set one) and the cursor's shapes, drawn at a thickness that follows the
 * text size.
 */
class TerminalDecorations(
    private val font: TerminalFont,
) {
    private val fill = Paint().apply { style = Paint.Style.FILL }
    private val stroke = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE }
    private val rect = RectF()
    private val wave = Path()

    /** A line's thickness: the underline's, the bar cursor's, the hollow cursor's edge. */
    private val line = max(1f, font.size / LINE_PER_SIZE)

    /** [run]'s lines on row [row], in [colors]. */
    fun lines(
        canvas: Canvas,
        row: Int,
        run: TerminalRun,
        colors: RunColors,
    ) {
        val top = font.top(row)
        val left = font.left(run.column)
        val right = font.left(run.column + run.span)
        fill.color = colors.foreground
        if (run.has(TerminalRun.ATTR_STRIKETHROUGH)) {
            val y = top + font.baseline - font.size * STRIKE_HEIGHT
            canvas.drawRect(left, y - line / 2, right, y + line / 2, fill)
        }
        if (run.has(TerminalRun.ATTR_OVERLINE)) canvas.drawRect(left, top, right, top + line, fill)
        if (run.underline == TerminalRun.UNDERLINE_NONE) return
        val color = if (run.underlineColor != 0) run.underlineColor else colors.foreground
        fill.color = color
        stroke.color = color
        stroke.strokeWidth = line
        // under the baseline, inside the cell: a double or curly line needs room for two below it
        val y = minOf(top + font.baseline + line * 2, top + font.cellHeight - line * UNDERLINE_ROOM)
        when (run.underline) {
            TerminalRun.UNDERLINE_DOUBLE -> {
                canvas.drawRect(left, y - line, right, y, fill)
                canvas.drawRect(left, y + line, right, y + line * 2, fill)
            }

            TerminalRun.UNDERLINE_CURLY -> {
                curly(canvas, left, right, y)
            }

            TerminalRun.UNDERLINE_DOTTED -> {
                dashes(canvas, left..right, y, line to line)
            }

            TerminalRun.UNDERLINE_DASHED -> {
                dashes(
                    canvas,
                    left..right,
                    y,
                    font.cellWidth / 2 to font.cellWidth / DASH_GAP,
                )
            }

            else -> {
                canvas.drawRect(left, y - line / 2, right, y + line / 2, fill)
            }
        }
    }

    private fun curly(
        canvas: Canvas,
        left: Float,
        right: Float,
        y: Float,
    ) {
        wave.reset()
        wave.moveTo(left, y)
        val step = font.cellWidth / 2
        var x = left
        var up = true
        while (x < right) {
            val next = minOf(x + step, right)
            val crest = if (up) y - line * WAVE_HEIGHT else y + line * WAVE_HEIGHT
            wave.quadTo((x + next) / 2, crest, next, y)
            up = !up
            x = next
        }
        canvas.drawPath(wave, stroke)
    }

    /** Dashes along [span] at [y], each `first` long with `second` between them. */
    private fun dashes(
        canvas: Canvas,
        span: ClosedFloatingPointRange<Float>,
        y: Float,
        pattern: Pair<Float, Float>,
    ) {
        val (dash, gap) = pattern
        var x = span.start
        while (x < span.endInclusive) {
            canvas.drawRect(x, y - line / 2, minOf(x + dash, span.endInclusive), y + line / 2, fill)
            x += dash + gap
        }
    }

    /**
     * The cursor as [shape] over [box] in [color]; for a block, the rectangle it filled, where the text under it is
     * to be drawn again in the cell's ground color, and `null` for the other shapes.
     */
    fun cursor(
        canvas: Canvas,
        shape: CursorShape,
        box: CellBox,
        color: Int,
    ): RectF? {
        val left = font.left(box.column)
        val right = font.left(box.column + box.cells)
        val top = font.top(box.row)
        val bottom = top + font.cellHeight
        fill.color = color
        stroke.color = color
        stroke.strokeWidth = line
        when (shape) {
            CursorShape.Block -> {
                rect.set(left, top, right, bottom)
                canvas.drawRect(rect, fill)
                return rect
            }

            CursorShape.Bar -> {
                canvas.drawRect(left, top, left + line * 2, bottom, fill)
            }

            CursorShape.Underline -> {
                canvas.drawRect(left, bottom - line * 2, right, bottom, fill)
            }

            CursorShape.HollowBlock -> {
                rect.set(left + line / 2, top + line / 2, right - line / 2, bottom - line / 2)
                canvas.drawRect(rect, stroke)
            }
        }
        return null
    }

    private companion object {
        /** A line is the text size over this, at least a pixel: about Ghostty's underline at its sizes. */
        const val LINE_PER_SIZE = 14f

        /** The strikethrough's height above the baseline, in text sizes: through the middle of the lowercase. */
        const val STRIKE_HEIGHT = 0.27f

        /** The lines' worth of room an underline leaves under it inside the cell. */
        const val UNDERLINE_ROOM = 2.5f

        /** A dashed underline's gap, in cells over this: a quarter cell after each half-cell dash. */
        const val DASH_GAP = 4

        /** A curly underline's crest above and below its line, in lines. */
        const val WAVE_HEIGHT = 1.5f
    }
}
