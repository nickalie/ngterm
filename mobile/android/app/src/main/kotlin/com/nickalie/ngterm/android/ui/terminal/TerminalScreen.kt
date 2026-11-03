package com.nickalie.ngterm.android.ui.terminal

/**
 * The screen a [TerminalView] draws: the rows it holds from the frames read so far, each replaced as a frame brings
 * it again, with the latest frame's header and selection. Used on the main thread only.
 */
class TerminalScreen {
    /** The latest frame's header; `null` before the first. */
    var header: TerminalHeader? = null
        private set

    /** The selection's rows, whole, as the latest frame listed them. */
    var selection: List<SelectedSpan> = emptyList()
        private set

    private var rows: Array<TerminalRow?> = emptyArray()

    /** The seq to ask the next frame since: the latest frame's, or 0 before the first. */
    val seq: Long get() = header?.seq ?: 0L

    /** The grid's columns, 0 before the first frame. */
    val columns: Int get() = header?.columns ?: 0

    /** The grid's rows, 0 before the first frame. */
    val rowCount: Int get() = rows.size

    /** The row at [index] as last sent; `null` for a row with nothing drawn on it (never sent since a full frame). */
    fun row(index: Int): TerminalRow? = rows.getOrNull(index)

    /**
     * Takes [update]: a full frame, or one whose grid differs, starts from blank rows; the rows it brings replace the
     * ones held. A row past the grid is dropped.
     */
    fun apply(update: FrameUpdate) {
        val next = update.header
        if (next.full || next.rows != rows.size || next.columns != header?.columns) {
            rows = arrayOfNulls(next.rows)
        }
        for (row in update.rows) {
            if (row.index in rows.indices) rows[row.index] = row
        }
        header = next
        selection = update.selection
    }

    /** Row [index]'s text as drawn, its blank cells spaces, trailing blanks trimmed (for tests and accessibility). */
    fun text(index: Int): String {
        val row = row(index) ?: return ""
        val line = StringBuilder()
        var column = 0
        for (run in row.runs) {
            repeat(run.column - column) { line.append(' ') }
            line.append(run.text)
            column = run.column + run.span
        }
        return line.trimEnd().toString()
    }
}
