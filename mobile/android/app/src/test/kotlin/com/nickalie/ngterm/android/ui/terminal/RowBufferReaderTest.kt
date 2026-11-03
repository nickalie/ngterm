package com.nickalie.ngterm.android.ui.terminal

import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakeRun
import com.nickalie.ngterm.core.fake.RowBufferWriter
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The reader against the Rust writer's golden fixture (crates/ngterm-mobile/tests/fixtures/rows-golden.bin, the
 * screen rows/tests.rs's `GOLDEN_BYTES` draws on a 12 × 4 grid), with the values the Rust side's own tests of the
 * fixture check; and the test fixtures' writer, which the views' fakes pack frames with, against the same bytes.
 */
class RowBufferReaderTest {
    private val golden: ByteArray = File(requireNotNull(System.getProperty("ngterm.rowsGolden"))).readBytes()

    private fun summary(run: TerminalRun) =
        listOf(run.column, run.span, run.kind, run.underline, run.attributes, run.foreground ushr 24, run.text)

    @Test
    fun theGoldenFixturesHeader() {
        val frame = RowBufferReader.read(golden)
        val header = frame.header
        assertTrue(header.full)
        assertFalse(header.stale)
        assertEquals(1L, header.seq)
        assertEquals(12 to 4, header.columns to header.rows)
        assertEquals(0xFFFFFFFF.toInt(), header.defaultForeground)
        assertEquals(0xFF282C34.toInt(), header.defaultBackground)
        assertEquals(0, header.cursorColor)
        val cursor = header.cursor
        assertEquals(10 to 3, cursor.column to cursor.row)
        assertEquals(CursorShape.Bar, cursor.shape)
        assertTrue(cursor.inView && cursor.visible && cursor.blinking)
        assertFalse(cursor.password || cursor.wideTail)
        assertEquals(listOf(SelectedSpan(0, 0, 3)), frame.selection)
        assertEquals(listOf(0, 1, 2, 3), frame.rows.map { it.index })
    }

    @Test
    fun theGoldenFixturesRows() {
        val rows = RowBufferReader.read(golden).rows
        val bold = TerminalRun.ATTR_BOLD
        val inverse = TerminalRun.ATTR_INVERSE
        assertEquals(
            listOf(
                listOf(0, 4, 0, 0, bold, 0xFF, "bold"),
                listOf(4, 1, 0, 0, 0, 0, " "),
                listOf(5, 2, 0, TerminalRun.UNDERLINE_SINGLE, 0, 0, "ul"),
            ),
            rows[0].runs.map(::summary),
        )
        assertEquals(0xFFCC6666.toInt(), rows[0].runs[0].foreground)
        assertEquals(0xFF010203.toInt(), rows[0].runs[2].background)
        val wide = TerminalRun.RUN_WIDE
        val cluster = TerminalRun.RUN_CLUSTER
        assertEquals(
            listOf(
                listOf(0, 2, wide, 0, 0, 0, "中"),
                listOf(2, 1, cluster, 0, 0, 0, "é"),
                listOf(3, 1, 0, 0, inverse, 0, "i"),
                listOf(4, 2, wide or cluster, 0, 0, 0, "👍🏽"),
            ),
            rows[1].runs.map(::summary),
        )
        assertTrue(rows[1].runs[0].wide && !rows[1].runs[0].cluster)
        assertTrue(rows[1].runs[3].wide && rows[1].runs[3].cluster)
        assertTrue(rows[2].wrapped)
        assertFalse(rows[3].wrapped)
        assertEquals(listOf(listOf(0, 12, 0, 0, 0, 0, "wrapping pas")), rows[2].runs.map(::summary))
        assertEquals(listOf(listOf(0, 10, 0, 0, 0, 0, "t the edge")), rows[3].runs.map(::summary))
    }

    /** The fakes' writer packs the golden screen byte for byte as the Rust writer does. */
    @Test
    fun theFixturesWriterPacksTheGoldenScreenAsRustDoes() {
        val packed =
            RowBufferWriter(12, 4, cursor = FakeCursor(10, 3, style = 1, blinking = true))
                .row(
                    0,
                    FakeRun(0, "bold", attributes = FakeRun.BOLD, foreground = 0xFFCC6666.toInt()),
                    FakeRun(4, " "),
                    FakeRun(5, "ul", underline = 1, background = 0xFF010203.toInt()),
                ).row(
                    1,
                    FakeRun(0, "中", kind = FakeRun.WIDE),
                    FakeRun(2, "é", kind = FakeRun.CLUSTER),
                    FakeRun(3, "i", attributes = FakeRun.INVERSE),
                    FakeRun(4, "👍🏽", kind = FakeRun.WIDE or FakeRun.CLUSTER),
                ).row(2, FakeRun(0, "wrapping pas"), wrapped = true)
                .text(3, "t the edge")
                .select(0, 0, 3)
                .bytes()
        assertArrayEquals(golden, packed)
    }

    @Test
    fun anotherMagicOrVersionOrACutBufferIsRefused() {
        val magic = golden.copyOf().also { it[0] = 'X'.code.toByte() }
        assertThrows(MalformedFrameException::class.java) { RowBufferReader.read(magic) }
        val version = golden.copyOf().also { it[4] = 2 }
        val refused = assertThrows(MalformedFrameException::class.java) { RowBufferReader.read(version) }
        assertEquals("row buffer version 2, not 1", refused.message)
        for (length in listOf(0, 39, 40, golden.size - 1)) {
            assertThrows(MalformedFrameException::class.java) { RowBufferReader.read(golden.copyOf(length)) }
        }
    }
}
