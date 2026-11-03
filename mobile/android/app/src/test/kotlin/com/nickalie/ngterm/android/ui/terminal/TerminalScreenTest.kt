package com.nickalie.ngterm.android.ui.terminal

import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test

class TerminalScreenTest {
    private fun frame(writer: RowBufferWriter) = RowBufferReader.read(writer.bytes())

    @Test
    fun aPartialFrameReplacesOnlyTheRowsItBrings() {
        val screen = TerminalScreen()
        screen.apply(frame(RowBufferWriter(10, 3).text(0, "one").text(1, "two").text(2, "three")))
        val kept = screen.row(0)
        screen.apply(frame(RowBufferWriter(10, 3, seq = 2, full = false).text(1, "TWO")))
        assertEquals(listOf("one", "TWO", "three"), (0..2).map(screen::text))
        assertSame("an unsent row is the record held", kept, screen.row(0))
        assertEquals(2L, screen.seq)
    }

    @Test
    fun aFullFrameOrANewGridStartsFromBlankRows() {
        val screen = TerminalScreen()
        screen.apply(frame(RowBufferWriter(10, 3).text(0, "one").text(2, "three")))
        // a full frame leaves out the rows with nothing drawn
        screen.apply(frame(RowBufferWriter(10, 3, seq = 2).text(1, "two")))
        assertEquals(listOf("", "two", ""), (0..2).map(screen::text))
        assertNull(screen.row(0))
        // a grid of another size, even in a partial frame, holds nothing of the last
        screen.apply(frame(RowBufferWriter(12, 2, seq = 3, full = false).text(1, "x")))
        assertEquals(2, screen.rowCount)
        assertEquals(listOf("", "x"), (0..1).map(screen::text))
    }

    @Test
    fun aRowsTextPutsEachRunAtItsColumn() {
        val screen = TerminalScreen()
        screen.apply(
            frame(
                RowBufferWriter(12, 1)
                    .row(0, FakeRun(0, "中", kind = FakeRun.WIDE), FakeRun(2, "a"), FakeRun(6, "b"))
                    .select(0, 1, 4),
            ),
        )
        assertEquals("中a   b", screen.text(0))
        assertEquals(listOf(SelectedSpan(0, 1, 4)), screen.selection)
    }
}
