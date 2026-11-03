package com.nickalie.ngterm.core.fake

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * A run of a [RowBufferWriter] row: [text] from [column], [span] columns (one per code point by default, two in a
 * wide run). Colors are `0xAARRGGBB`, `0` for the terminal's default; [kind], [underline] and [attributes] are the
 * layout's bits (crates/ngterm-mobile/src/rows.rs).
 */
data class FakeRun(
    val column: Int,
    val text: String,
    val kind: Int = 0,
    val span: Int = defaultSpan(text, kind),
    val underline: Int = 0,
    val attributes: Int = 0,
    val foreground: Int = 0,
    val background: Int = 0,
    val underlineColor: Int = 0,
) {
    companion object {
        const val WIDE = 1
        const val CLUSTER = 2

        const val BOLD = 1
        const val ITALIC = 2
        const val FAINT = 4
        const val INVERSE = 16
        const val INVISIBLE = 32
        const val STRIKETHROUGH = 64
        const val OVERLINE = 128

        private fun defaultSpan(
            text: String,
            kind: Int,
        ): Int {
            val glyphs = if (kind and CLUSTER != 0) 1 else text.codePointCount(0, text.length)
            return if (kind and WIDE != 0) glyphs * 2 else glyphs
        }
    }
}

/** The cursor of a [RowBufferWriter] frame: style 0 block, 1 bar, 2 underline, 3 hollow block. */
data class FakeCursor(
    val column: Int,
    val row: Int,
    val style: Int = 0,
    val visible: Boolean = true,
    val blinking: Boolean = false,
    val wideTail: Boolean = false,
)

/**
 * Packs a frame as `Pane.frame` does (crates/ngterm-mobile/src/rows.rs, version 1), for fakes that hand a view
 * frames with no native library behind them. The app's reader test holds it to the Rust writer's golden fixture.
 */
class RowBufferWriter(
    private val columns: Int,
    private val rows: Int,
    private val seq: Long = 1,
    private val full: Boolean = true,
    private val cursor: FakeCursor? = FakeCursor(0, 0),
    private val stale: Boolean = false,
) {
    private var foreground = GHOSTTY_FOREGROUND
    private var background = GHOSTTY_BACKGROUND
    private var cursorColor = 0
    private val records = sortedMapOf<Int, Pair<Boolean, List<FakeRun>>>()
    private val selection = sortedMapOf<Int, Pair<Int, Int>>()

    /** The default text and ground, and the cursor's color (`0`: it follows the cell); Ghostty's, none by default. */
    fun colors(
        foreground: Int,
        background: Int,
        cursor: Int = 0,
    ) = apply {
        this.foreground = foreground
        this.background = background
        cursorColor = cursor
    }

    /** Row [index] with [runs], left to right; [wrapped] when its text continues on the next row. */
    fun row(
        index: Int,
        vararg runs: FakeRun,
        wrapped: Boolean = false,
    ) = apply { records[index] = wrapped to runs.toList() }

    /** Row [index] as one run of default-colored [text]. */
    fun text(
        index: Int,
        text: String,
    ) = row(index, FakeRun(0, text))

    /** Selects row [index] from [first] to [last], both included. */
    fun select(
        index: Int,
        first: Int,
        last: Int,
    ) = apply { selection[index] = first to last }

    /** The packed frame. */
    fun bytes(): ByteArray {
        val out = ByteArrayOutputStream()
        val header = buffer(HEADER_BYTES)
        header.put("NGRB".toByteArray(Charsets.US_ASCII))
        header.putShort(1)
        header.putShort(((if (full) 1 else 0) or (if (stale) 2 else 0)).toShort())
        header.putLong(seq)
        header.putShort(columns.toShort())
        header.putShort(rows.toShort())
        header.putInt(foreground)
        header.putInt(background)
        header.putInt(cursorColor)
        header.putShort((cursor?.column ?: 0).toShort())
        header.putShort((cursor?.row ?: 0).toShort())
        header.put((cursor?.style ?: 0).toByte())
        header.put(cursorFlags().toByte())
        header.putShort(records.size.toShort())
        out.write(header.array())
        for ((index, record) in records) {
            val (wrapped, runs) = record
            val row = buffer(ROW_BYTES)
            row.putShort(index.toShort())
            row.put((if (wrapped) 1 else 0).toByte())
            row.putShort(runs.size.toShort())
            out.write(row.array())
            for (run in runs) out.write(run(run))
        }
        val spans = buffer(2 + selection.size * SPAN_BYTES)
        spans.putShort(selection.size.toShort())
        for ((index, span) in selection) {
            spans.putShort(index.toShort())
            spans.putShort(span.first.toShort())
            spans.putShort(span.second.toShort())
        }
        out.write(spans.array())
        return out.toByteArray()
    }

    private fun cursorFlags(): Int {
        val cursor = cursor ?: return 0
        var flags = IN_VIEW
        if (cursor.visible) flags = flags or VISIBLE
        if (cursor.blinking) flags = flags or BLINKING
        if (cursor.wideTail) flags = flags or WIDE_TAIL
        return flags
    }

    private fun run(run: FakeRun): ByteArray {
        val text = run.text.toByteArray(Charsets.UTF_8)
        val out = buffer(RUN_BYTES + text.size)
        out.putShort(run.column.toShort())
        out.putShort(run.span.toShort())
        out.put(run.kind.toByte())
        out.put(run.underline.toByte())
        out.putShort(run.attributes.toShort())
        out.putInt(run.foreground)
        out.putInt(run.background)
        out.putInt(run.underlineColor)
        out.putInt(text.size)
        out.put(text)
        return out.array()
    }

    private fun buffer(size: Int) = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN)

    companion object {
        /** Ghostty's default text and ground, the terminal's colors before the view gives it a palette. */
        const val GHOSTTY_FOREGROUND = 0xFFFFFFFF.toInt()
        const val GHOSTTY_BACKGROUND = 0xFF282C34.toInt()

        private const val HEADER_BYTES = 40
        private const val ROW_BYTES = 5
        private const val RUN_BYTES = 24
        private const val SPAN_BYTES = 6
        private const val IN_VIEW = 1
        private const val VISIBLE = 2
        private const val BLINKING = 4
        private const val WIDE_TAIL = 16
    }
}
