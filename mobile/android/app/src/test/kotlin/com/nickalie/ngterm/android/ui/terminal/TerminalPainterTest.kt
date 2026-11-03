package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Typeface
import androidx.compose.ui.geometry.Size
import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.GraphicsMode

/** The painter against the glyph cache it shares the font's paints with. */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class TerminalPainterTest {
    // a bold face wider than the monospace cell, so its glyphs are squeezed into their cells
    private fun font() =
        TerminalFont(
            TerminalTypefaces(
                regular = Typeface.MONOSPACE,
                bold = Typeface.create(Typeface.SANS_SERIF, Typeface.BOLD),
                italic = Typeface.MONOSPACE,
                boldItalic = Typeface.MONOSPACE,
            ),
            size = 32f,
        )

    private fun xs(layout: RowLayout) = layout.draws.map { it.x to it.scaleX }

    @Test
    fun aSqueezedGlyphLeavesTheSharedPaintAtItsOwnWidthForTheRowsLaidOutAfter() {
        val bold = FakeRun.BOLD
        val screen = TerminalScreen()
        screen.apply(
            RowBufferReader.read(
                RowBufferWriter(10, 2, cursor = FakeCursor(0, 1, visible = false))
                    .row(0, FakeRun(0, "WWW", attributes = bold))
                    .row(1, FakeRun(0, "il", attributes = bold))
                    .bytes(),
            ),
        )
        val font = font()
        val glyphs = GlyphCache(font)
        val painter = TerminalPainter(font, glyphs)
        val canvas = Canvas(Bitmap.createBitmap(400, 200, Bitmap.Config.ARGB_8888))
        painter.draw(canvas, screen, Size(400f, 200f), TerminalLook(selectionColor = 0, focused = true))

        assertTrue("the wide row is squeezed", glyphs.layout(screen.row(0)!!).draws.any { it.scaleX < 1f })
        assertEquals(1f, font.paint(TerminalRun.ATTR_BOLD).textScaleX)
        // the row laid out after the squeezed one is measured as on a fresh font
        assertEquals(xs(GlyphCache(font()).layout(screen.row(1)!!)), xs(glyphs.layout(screen.row(1)!!)))
    }
}
