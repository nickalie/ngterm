package com.nickalie.ngterm.android.ui.terminal

import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.RowBufferWriter
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * A pane whose screen is [lines] (a few lines of words by default), its cursor at [cursor], and whose selection
 * follows the select calls it takes, as the real terminal's does: a word runs to the blanks around the point, a
 * range is its cells row by row, everything is every row.
 */
internal class SelectingPane(
    private val lines: List<String> = List(ROWS) { "alpha beta gamma delta $it" },
    private val cursor: FakeCursor = FakeCursor(0, 0),
) : FakePane() {
    @Volatile private var spans: List<Triple<Int, Int, Int>> = emptyList()
    private var seq = 0L

    init {
        frames = { _ ->
            val grid = sizes.lastOrNull() ?: PaneSize(COLUMNS.toUShort(), ROWS.toUShort(), 1u, 1u)
            seq++
            val writer = RowBufferWriter(grid.cols.toInt(), grid.rows.toInt(), seq = seq, cursor = cursor)
            colors.lastOrNull()?.let {
                writer.colors(
                    it.foreground.toInt(),
                    it.background.toInt(),
                    it.cursor?.toInt() ?: 0,
                )
            }
            for ((row, line) in lines
                .take(
                    grid.rows.toInt(),
                ).withIndex()) {
                writer.text(row, line.take(grid.cols.toInt()))
            }
            for ((row, first, last) in spans) writer.select(row, first, last)
            writer.bytes()
        }
    }

    override fun selectWord(at: CellPoint): Boolean {
        val line = lines.getOrElse(at.y.toInt()) { "" }
        val column = at.x.toInt()
        if (column >= line.length || line[column] == ' ') {
            super.selectWord(at)
            return false
        }
        var first = column
        while (first > 0 && line[first - 1] != ' ') first--
        var last = column
        while (last + 1 < line.length && line[last + 1] != ' ') last++
        spans = listOf(Triple(at.y.toInt(), first, last))
        return super.selectWord(at)
    }

    override fun select(
        start: CellPoint,
        end: CellPoint,
        rectangle: Boolean,
    ) {
        val (from, to) = listOf(start, end).sortedWith(compareBy({ it.y }, { it.x }))
        val columns = sizes.last().cols.toInt()
        spans =
            (from.y.toInt()..to.y.toInt()).map { row ->
                val first = if (row == from.y.toInt()) from.x.toInt() else 0
                val last = if (row == to.y.toInt()) to.x.toInt() else columns - 1
                Triple(row, first, last)
            }
        super.select(start, end, rectangle)
    }

    override fun selectAll(): Boolean {
        spans = (0 until sizes.last().rows.toInt()).map { Triple(it, 0, sizes.last().cols.toInt() - 1) }
        return super.selectAll()
    }

    override fun clearSelection() {
        spans = emptyList()
        super.clearSelection()
    }

    companion object {
        const val COLUMNS = 40
        const val ROWS = 6
    }
}
