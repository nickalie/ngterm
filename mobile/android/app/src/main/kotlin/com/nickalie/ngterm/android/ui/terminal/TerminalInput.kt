package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.animation.core.AnimationState
import androidx.compose.animation.core.animateDecay
import androidx.compose.animation.splineBasedDecay
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.PaneMouse
import com.nickalie.ngterm.core.ffi.PaneMouseAction
import com.nickalie.ngterm.core.ffi.PaneMouseButton
import com.nickalie.ngterm.core.ffi.PaneSendOutcome
import com.nickalie.ngterm.core.ffi.PaneWheel
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import kotlin.math.floor
import kotlin.math.roundToInt

/** The text sizes a pinch keeps to, in sp. */
val TerminalTextSizes = 8f..32f

// a pane with no terminal now (hidden, between attaches) or a full input queue drops pointer input: a pointer event
// is not worth queueing
private inline fun <T> guarded(call: () -> T): T? =
    try {
        call()
    } catch (_: PaneInputException) {
        null
    }

/**
 * The pointer's side of a [TerminalState]'s pane: positions turned into the pane's cells and pixels, and the mouse,
 * wheel, scroll and pinch calls the gestures make. Used on the main thread.
 */
@Stable
class TerminalInput internal constructor(
    private val state: TerminalState,
    private val scope: CoroutineScope,
) {
    /** The font the view draws with; the cells are its. */
    internal var font: TerminalFont? = null

    private var rowCarry = 0f
    private var wheelRows = 0f
    private var wheelColumns = 0f
    private var hovered: CellPoint? = null
    private var fling: Job? = null

    /** Whether the program takes pointer events: a click and a drag are then the program's. */
    val tracking: Boolean get() = state.pane.mouseTracking()

    /** The cell under [position] (view pixels), kept inside the grid; `null` before the view has a grid. */
    fun cell(position: Offset): CellPoint? {
        val font = font
        val grid = state.grid
        if (font == null || grid == null) return null
        val column = floor(position.x / font.cellWidth).toInt().coerceIn(0, grid.cols.toInt() - 1)
        val row = floor(position.y / font.cellHeight).toInt().coerceIn(0, grid.rows.toInt() - 1)
        return CellPoint(column.toUShort(), row.toUInt())
    }

    // view pixels as the pane's: it counts cells of the whole pixels it was given, the view draws them unrounded
    private fun panePixels(position: Offset): Offset {
        val font = font
        val grid = state.grid
        if (font == null || grid == null) return position
        return Offset(
            position.x / font.cellWidth * grid.cellWidth.toFloat(),
            position.y / font.cellHeight * grid.cellHeight.toFloat(),
        )
    }

    /** Reports a pointer event to the program; whether it took it (it does only while [tracking]). */
    fun mouse(
        action: PaneMouseAction,
        button: PaneMouseButton?,
        mods: UByte,
        position: Offset,
    ): Boolean {
        val at = panePixels(position)
        return guarded { state.pane.mouse(PaneMouse(action, button, mods, at.x, at.y)) } == true
    }

    /** The mouse moved with no button down: a program tracking every motion hears of each new cell. */
    fun hover(
        position: Offset,
        mods: UByte,
    ) {
        val cell = cell(position)
        if (cell != null && cell != hovered) {
            hovered = cell
            mouse(PaneMouseAction.MOTION, null, mods, position)
        }
    }

    /** Turns the wheel by whole [rows] (up is positive) and [columns] (right): the pane routes it. */
    fun wheel(
        rows: Int,
        columns: Int,
        mods: UByte,
        position: Offset,
    ) {
        if (rows == 0 && columns == 0) return
        val at = panePixels(position)
        guarded { state.pane.wheel(PaneWheel(rows, columns, mods, at.x, at.y)) }
        state.dirty()
    }

    /** A mouse wheel's or trackpad's scroll: [delta] in steps, down and right positive as Compose gives them. */
    fun scroll(
        delta: Offset,
        mods: UByte,
        position: Offset,
    ) {
        wheelRows -= delta.y
        wheelColumns += delta.x
        val rows = wheelRows.toInt()
        val columns = wheelColumns.toInt()
        wheelRows -= rows
        wheelColumns -= columns
        wheel(rows, columns, mods, position)
    }

    /** A finger dragged the screen [pixels] down (negative: up): the rows come down with it, as a wheel turned up. */
    fun drag(
        pixels: Float,
        position: Offset,
    ) {
        val cell = font?.cellHeight?.toFloat() ?: return
        rowCarry += pixels
        val rows = (rowCarry / cell).toInt()
        rowCarry -= rows * cell
        wheel(rows, 0, 0u, position)
    }

    /** Goes on scrolling after a drag let go at [velocity] pixels a second, slowing down as a list does. */
    fun fling(
        velocity: Float,
        position: Offset,
        density: Density,
    ) {
        stopFling()
        fling =
            scope.launch {
                var last = 0f
                AnimationState(0f, velocity).animateDecay(splineBasedDecay(density)) {
                    drag(value - last, position)
                    last = value
                }
            }
    }

    /** A touch stops a fling where it is, and starts its own drag from no remainder. */
    fun stopFling() {
        fling?.cancel()
        fling = null
        rowCarry = 0f
    }

    /** A pinch scaled the text by [zoom] from [base] sp: it changes in whole sp, so the pty is resized per step. */
    fun zoom(
        base: Float,
        zoom: Float,
    ) {
        val size = (base * zoom).roundToInt().toFloat().coerceIn(TerminalTextSizes)
        if (size != state.textSize.value) state.textSize = size.sp
    }
}

/**
 * The selection's side of a [TerminalState]'s pane: a touch selection with its handles and toolbar, the right-click
 * menu, a paste waiting for its confirmation, and the selection and clipboard calls they make. Used on the main
 * thread.
 */
@Stable
class TerminalSelection internal constructor(
    private val state: TerminalState,
) {
    /** A long press made the selection: its handles and its toolbar show while it has any. */
    var touchSelection by mutableStateOf(false)
        private set

    /** A finger is moving the selection: the toolbar keeps out of the way. */
    var adjusting by mutableStateOf(false)
        internal set

    /** Where the right-click menu opens, in the view's pixels; `null` while it is closed. */
    var menuAt by mutableStateOf<Offset?>(null)
        internal set

    /** Text the pane refused to paste as unsafe, waiting for the user's yes; `null` with none. */
    var unsafePaste by mutableStateOf<String?>(null)
        private set

    // a touch selection is let go once a frame shows it gone (copied, cleared by output), not before its first frame
    private var selectionSeen = false
    private var lastClick = Click(CellPoint(0u, 0u), 0L, 0)

    private data class Click(
        val cell: CellPoint,
        val at: Long,
        val count: Int,
    )

    /** Selects from [start] to [end] (either order). */
    fun select(
        start: CellPoint,
        end: CellPoint,
    ) {
        guarded { state.pane.select(start, end, false) }
        state.dirty()
    }

    /** Starts a touch selection at [cell]: the word there, or the cell itself where there is none. */
    fun selectWordAt(cell: CellPoint) {
        menuAt = null
        val word = guarded { state.pane.selectWord(cell) }
        if (word == false) guarded { state.pane.select(cell, cell, false) }
        if (word != null) {
            touchSelection = true
            selectionSeen = false
        }
        state.dirty()
    }

    /**
     * A mouse click at [cell] at [time] ms; returns how many clicks in a row it makes: the first lets the selection
     * go, a second selects the word, a third the line.
     */
    fun mouseClick(
        cell: CellPoint,
        time: Long,
        doubleClickMillis: Long,
    ): Int {
        val again = cell == lastClick.cell && time - lastClick.at <= doubleClickMillis
        val count = if (again) lastClick.count + 1 else 1
        lastClick = Click(cell, time, count)
        touchSelection = false
        when ((count - 1) % CLICK_KINDS) {
            0 -> guarded { state.pane.clearSelection() }
            1 -> guarded { state.pane.selectWord(cell) }
            else -> guarded { state.pane.selectLine(cell) }
        }
        state.dirty()
        return count
    }

    /** Selects everything on screen and in the scrollback. */
    fun selectAll() {
        menuAt = null
        guarded { state.pane.selectAll() }
        state.dirty()
    }

    /** Drops the selection, and with it the handles and the toolbar. */
    fun clearSelection() {
        touchSelection = false
        guarded { state.pane.clearSelection() }
        state.dirty()
    }

    /** The selection's text as a copy takes it, and the selection is let go; `null` with none. */
    fun copy(): String? {
        menuAt = null
        val text = guarded { state.pane.selectionText() }
        clearSelection()
        return text
    }

    /**
     * Pastes [text] into the program. Text that can run a command (a line break where the program would take it as
     * Enter) waits in [unsafePaste] for the user's yes unless [allowUnsafe].
     */
    fun paste(
        text: String,
        allowUnsafe: Boolean = false,
    ) {
        menuAt = null
        unsafePaste = null
        if (touchSelection) clearSelection()
        val outcome = guarded { state.pane.paste(text, allowUnsafe) }
        if (outcome == PaneSendOutcome.UNSAFE) unsafePaste = text
    }

    /** The user said no to [unsafePaste]. */
    fun cancelPaste() {
        unsafePaste = null
    }

    /** Closes the right-click menu. */
    fun closeMenu() {
        menuAt = null
    }

    /** A frame came with [selection]: a touch selection whose frames no longer show it is over. */
    internal fun shown(selection: List<SelectedSpan>) {
        if (selection.isNotEmpty()) {
            selectionSeen = true
        } else if (touchSelection && selectionSeen && !adjusting) {
            touchSelection = false
        }
    }

    private companion object {
        // a single, a double (word) and a triple (line) click, then round again
        const val CLICK_KINDS = 3
    }
}
