package com.nickalie.ngterm.android.ui.terminal

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * A run of cells of one style, left to right ([RowBufferReader]'s run record). Colors are `0xAARRGGBB`; `0` in
 * [foreground] or [background] is the terminal's default, resolved from the header of the frame being drawn
 * ([TerminalHeader.defaultForeground], [TerminalHeader.defaultBackground]), never cached.
 */
data class TerminalRun(
    val column: Int,
    val span: Int,
    val kind: Int,
    val underline: Int,
    val attributes: Int,
    val foreground: Int,
    val background: Int,
    /** `0`: the foreground. */
    val underlineColor: Int,
    val text: String,
) {
    /** Each glyph covers two columns. */
    val wide: Boolean get() = kind and RUN_WIDE != 0

    /** The text is one grapheme cluster: one glyph at [column], [span] columns wide. */
    val cluster: Boolean get() = kind and RUN_CLUSTER != 0

    /** Whether the attribute [bit] (one of the `ATTR_` constants) is set. */
    fun has(bit: Int): Boolean = attributes and bit != 0

    companion object {
        const val RUN_WIDE = 1
        const val RUN_CLUSTER = 2

        const val ATTR_BOLD = 1
        const val ATTR_ITALIC = 2
        const val ATTR_FAINT = 4
        const val ATTR_BLINK = 8
        const val ATTR_INVERSE = 16
        const val ATTR_INVISIBLE = 32
        const val ATTR_STRIKETHROUGH = 64
        const val ATTR_OVERLINE = 128
        const val ATTR_HYPERLINK = 256

        const val UNDERLINE_NONE = 0
        const val UNDERLINE_SINGLE = 1
        const val UNDERLINE_DOUBLE = 2
        const val UNDERLINE_CURLY = 3
        const val UNDERLINE_DOTTED = 4
        const val UNDERLINE_DASHED = 5
    }
}

/** One changed row: its index in the grid, whether its text continues on the next row, and its runs. */
class TerminalRow(
    val index: Int,
    val wrapped: Boolean,
    val runs: List<TerminalRun>,
)

/** A row the selection crosses, from [first] to [last] column, both included. */
data class SelectedSpan(
    val row: Int,
    val first: Int,
    val last: Int,
)

/** The cursor's shape, as the header's cursor style byte has it. */
enum class CursorShape {
    Block,
    Bar,
    Underline,
    HollowBlock,
}

/** Where the cursor is and how it is drawn ([inView] and [visible] both, or there is none to draw). */
data class TerminalCursor(
    val column: Int,
    val row: Int,
    val shape: CursorShape,
    val inView: Boolean,
    val visible: Boolean,
    val blinking: Boolean,
    val password: Boolean,
    /** On the second column of a wide character. */
    val wideTail: Boolean,
) {
    /** Whether there is a cursor to draw. */
    val shown: Boolean get() = inView && visible

    companion object {
        /** No cursor, before the first frame. */
        val None = TerminalCursor(0, 0, CursorShape.Block, false, false, false, false, false)
    }
}

/** A frame's header: its seq, its grid, the default colors and the cursor. */
data class TerminalHeader(
    /** Pass to the next `frame` call. */
    val seq: Long,
    /** Every row follows ([FRAME_FULL]): draw from scratch. */
    val full: Boolean,
    /** The terminal was busy and this is the frame made last: ask again soon. */
    val stale: Boolean,
    val columns: Int,
    val rows: Int,
    val defaultForeground: Int,
    val defaultBackground: Int,
    /** `0`: none set, the cursor follows the cell's text. */
    val cursorColor: Int,
    val cursor: TerminalCursor,
)

/** What one `Pane.frame` call brings: the header, the rows that changed, and the whole selection. */
class FrameUpdate(
    val header: TerminalHeader,
    val rows: List<TerminalRow>,
    val selection: List<SelectedSpan>,
)

/** A buffer that does not follow the layout. */
class MalformedFrameException(
    message: String,
) : IllegalArgumentException(message)

/**
 * Reads the packed row buffer `Pane.frame` returns, as `crates/ngterm-mobile/src/rows.rs`'s module docs lay it out
 * (version 1, magic `NGRB`, little-endian). Its test decodes the Rust writer's golden fixture
 * (`crates/ngterm-mobile/tests/fixtures/rows-golden.bin`), which keeps the two in step.
 */
object RowBufferReader {
    /** The layout's version this reader follows. */
    const val VERSION = 1

    const val FRAME_FULL = 1
    const val FRAME_STALE = 2

    const val CURSOR_IN_VIEW = 1
    const val CURSOR_VISIBLE = 2
    const val CURSOR_BLINKING = 4
    const val CURSOR_PASSWORD = 8
    const val CURSOR_WIDE_TAIL = 16

    const val ROW_WRAPPED = 1

    private val MAGIC = byteArrayOf('N'.code.toByte(), 'G'.code.toByte(), 'R'.code.toByte(), 'B'.code.toByte())
    private const val RUN_FIXED_BYTES = 24
    private const val ROW_FIXED_BYTES = 5
    private const val SPAN_BYTES = 6

    /**
     * Decodes [bytes].
     *
     * @throws MalformedFrameException for another magic or version, or a buffer cut short.
     */
    fun read(bytes: ByteArray): FrameUpdate {
        val buffer = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
        val header = header(buffer)
        val rowCount = buffer.u16()
        val rows = List(rowCount) { row(buffer) }
        buffer.need(2)
        val spans = buffer.u16()
        buffer.need(spans * SPAN_BYTES)
        val selection = List(spans) { SelectedSpan(buffer.u16(), buffer.u16(), buffer.u16()) }
        return FrameUpdate(header, rows, selection)
    }

    private fun header(buffer: ByteBuffer): TerminalHeader {
        buffer.need(HEADER_BYTES)
        val magic = ByteArray(MAGIC.size).also { buffer.get(it) }
        if (!magic.contentEquals(MAGIC)) throw MalformedFrameException("not a row buffer")
        val version = buffer.u16()
        if (version != VERSION) throw MalformedFrameException("row buffer version $version, not $VERSION")
        val flags = buffer.u16()
        val seq = buffer.long
        val columns = buffer.u16()
        val rows = buffer.u16()
        val foreground = buffer.int
        val background = buffer.int
        val cursorColor = buffer.int
        val cursorColumn = buffer.u16()
        val cursorRow = buffer.u16()
        val style = buffer.u8()
        val cursorFlags = buffer.u8()
        val cursor =
            TerminalCursor(
                column = cursorColumn,
                row = cursorRow,
                shape = CursorShape.entries.getOrElse(style) { CursorShape.Block },
                inView = cursorFlags and CURSOR_IN_VIEW != 0,
                visible = cursorFlags and CURSOR_VISIBLE != 0,
                blinking = cursorFlags and CURSOR_BLINKING != 0,
                password = cursorFlags and CURSOR_PASSWORD != 0,
                wideTail = cursorFlags and CURSOR_WIDE_TAIL != 0,
            )
        return TerminalHeader(
            seq = seq,
            full = flags and FRAME_FULL != 0,
            stale = flags and FRAME_STALE != 0,
            columns = columns,
            rows = rows,
            defaultForeground = foreground,
            defaultBackground = background,
            cursorColor = cursorColor,
            cursor = cursor,
        )
    }

    private fun row(buffer: ByteBuffer): TerminalRow {
        buffer.need(ROW_FIXED_BYTES)
        val index = buffer.u16()
        val flags = buffer.u8()
        val runs = buffer.u16()
        return TerminalRow(index, flags and ROW_WRAPPED != 0, List(runs) { run(buffer) })
    }

    private fun run(buffer: ByteBuffer): TerminalRun {
        buffer.need(RUN_FIXED_BYTES)
        val column = buffer.u16()
        val span = buffer.u16()
        val kind = buffer.u8()
        val underline = buffer.u8()
        val attributes = buffer.u16()
        val foreground = buffer.int
        val background = buffer.int
        val underlineColor = buffer.int
        val length = buffer.int
        if (length < 0) throw MalformedFrameException("a run's text is too long")
        buffer.need(length)
        val text = String(bytes = buffer.array(), offset = buffer.position(), length = length, charset = Charsets.UTF_8)
        buffer.position(buffer.position() + length)
        return TerminalRun(column, span, kind, underline, attributes, foreground, background, underlineColor, text)
    }

    /** The header's length in bytes, with the row count that ends it. */
    private const val HEADER_BYTES = 40
    private const val U16 = 0xFFFF
    private const val U8 = 0xFF

    private fun ByteBuffer.need(count: Int) {
        if (remaining() < count) throw MalformedFrameException("row buffer cut short")
    }

    private fun ByteBuffer.u16(): Int = short.toInt() and U16

    private fun ByteBuffer.u8(): Int = get().toInt() and U8
}
