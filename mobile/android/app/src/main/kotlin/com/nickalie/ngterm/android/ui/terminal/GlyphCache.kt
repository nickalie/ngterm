package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Paint
import android.graphics.Typeface
import android.text.TextPaint
import kotlin.math.abs
import kotlin.math.ceil

/**
 * The terminal's font at one size: a paint per text style (regular, bold, italic, bold italic) and the cell they
 * make. The cell is as wide as the font's advance, unrounded, so a run of the font's own glyphs drawn as one string
 * lands on the grid, and as tall as its ascent to descent, rounded up to whole pixels.
 *
 * A code point the font lacks (CJK, emoji, most symbols) falls back to the platform's fonts, which Android's text
 * stack does by itself for any typeface.
 */
class TerminalFont(
    typefaces: TerminalTypefaces,
    /** The text size in pixels. */
    val size: Float,
) {
    private val paints =
        listOf(typefaces.regular, typefaces.bold, typefaces.italic, typefaces.boldItalic).map { typeface ->
            TextPaint(Paint.ANTI_ALIAS_FLAG or Paint.SUBPIXEL_TEXT_FLAG).apply {
                this.typeface = typeface
                textSize = size
            }
        }

    /** A cell's width in pixels. */
    val cellWidth: Float = paints[0].measureText("M")

    /** A cell's height in whole pixels. */
    val cellHeight: Int

    /** The baseline's distance from a cell's top, in pixels. */
    val baseline: Float

    init {
        val metrics = paints[0].fontMetrics
        val height = metrics.descent - metrics.ascent
        cellHeight = ceil(height).toInt().coerceAtLeast(1)
        baseline = (cellHeight - height) / 2 - metrics.ascent
    }

    /** Row [row]'s top, in pixels. */
    fun top(row: Int): Float = (row * cellHeight).toFloat()

    /** Column [column]'s left edge, in pixels. */
    fun left(column: Int): Float = column * cellWidth

    /** The paint for a run's [attributes] (bold and italic pick the face). */
    fun paint(attributes: Int): TextPaint {
        val bold = if (attributes and TerminalRun.ATTR_BOLD != 0) 1 else 0
        val italic = if (attributes and TerminalRun.ATTR_ITALIC != 0) 2 else 0
        return paints[bold + italic]
    }
}

/** The four faces of the terminal's font. */
class TerminalTypefaces(
    val regular: Typeface,
    val bold: Typeface,
    val italic: Typeface,
    val boldItalic: Typeface,
)

/**
 * One draw of text: [text] at [x] pixels from the row's left on the baseline, squeezed by [scaleX] (1 when it fits),
 * in [run]'s style.
 */
class GlyphDraw(
    val run: TerminalRun,
    val text: String,
    val x: Float,
    val scaleX: Float,
)

/** A row's text, laid out on the grid: what to draw for each run, measured once per version of the row. */
class RowLayout(
    /** The row record it was laid out from: a row sent again is a new record, laid out anew. */
    val source: TerminalRow,
    val draws: List<GlyphDraw>,
)

/**
 * The shaped runs of each row, kept until the row changes. A run of the font's own glyphs is one string drawn at its
 * first column; a run that would drift off the grid (a fallback font's glyph of another width), a wide run and a
 * cluster are drawn a glyph at a time, each centered in its cells, or squeezed into them when wider. The measuring is
 * the expensive part, and it happens once per row record and font; the platform's text layout cache keeps the shaping
 * of each string between draws.
 */
class GlyphCache(
    private val font: TerminalFont,
) {
    private var layouts: Array<RowLayout?> = emptyArray()

    /** [row]'s layout, from the cache while [row] is the record it was made from. */
    fun layout(row: TerminalRow): RowLayout {
        if (row.index >= layouts.size) layouts = layouts.copyOf(row.index + 1)
        layouts[row.index]?.takeIf { it.source === row }?.let { return it }
        val made = RowLayout(row, row.runs.flatMap(::draws))
        layouts[row.index] = made
        return made
    }

    private fun draws(run: TerminalRun): List<GlyphDraw> {
        if (run.text.isEmpty() || run.text.isBlank()) return emptyList()
        val paint = font.paint(run.attributes)
        val left = run.column * font.cellWidth
        return when {
            run.cluster -> {
                listOf(glyph(run, run.text, left, run.span, paint))
            }

            run.wide -> {
                codePoints(run.text).mapIndexed {
                    i,
                    glyph,
                    ->
                    glyph(run, glyph, left + 2 * i * font.cellWidth, 2, paint)
                }
            }

            else -> {
                narrow(run, left, paint)
            }
        }
    }

    private fun narrow(
        run: TerminalRun,
        left: Float,
        paint: TextPaint,
    ): List<GlyphDraw> {
        val glyphs = codePoints(run.text)
        val expected = glyphs.size * font.cellWidth
        if (abs(paint.measureText(run.text) - expected) < DRIFT) return listOf(GlyphDraw(run, run.text, left, 1f))
        return glyphs.mapIndexedNotNull { i, glyph ->
            if (glyph == " ") null else glyph(run, glyph, left + i * font.cellWidth, 1, paint)
        }
    }

    private fun glyph(
        run: TerminalRun,
        text: String,
        left: Float,
        cells: Int,
        paint: TextPaint,
    ): GlyphDraw {
        val room = cells * font.cellWidth
        val width = paint.measureText(text)
        return if (width > room) {
            GlyphDraw(run, text, left, room / width)
        } else {
            GlyphDraw(run, text, left + (room - width) / 2, 1f)
        }
    }

    private companion object {
        /** How far, in pixels, a run's measured width may stray from its cells before it is drawn glyph by glyph. */
        const val DRIFT = 0.5f

        fun codePoints(text: String): List<String> {
            val glyphs = ArrayList<String>(text.length)
            var offset = 0
            while (offset < text.length) {
                val next = text.offsetByCodePoints(offset, 1)
                glyphs += text.substring(offset, next)
                offset = next
            }
            return glyphs
        }
    }
}
