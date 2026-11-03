package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.offset
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.round
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.core.ffi.CellPoint
import kotlin.math.roundToInt

/** A selection handle's knob: its radius. */
internal val KnobRadius = 11.dp

/** A selection handle's stem: its width. */
private val StemWidth = 2.dp

/** The gap between the selection and its toolbar. */
private val ToolbarGap = 8.dp

/** One end of a selection, as a finger on its handle moves it: the end's [cell], the [fixed] other end. */
internal class Handle(
    val cell: CellPoint,
    val fixed: CellPoint,
    /** The end cell's middle, in the view's pixels: a finger on the knob moves this point. */
    val cellCenter: Offset,
    /** The knob's middle, in the view's pixels. */
    val knob: Offset,
    /** The knob's square corner is its top left (it hangs right of its stem), not its top right. */
    val squareLeft: Boolean,
    /** The stem: a line on the selection's edge (its left and right are the same), as tall as the end's row. */
    val stem: Rect,
)

/**
 * Where a selection's two handles stand, in the view's pixels. A handle is a stem on the selection's edge, as tall as
 * its row, and a teardrop knob hanging under the row: the start's to the left of its stem, the end's to the right,
 * each with its square corner at the stem's foot, so the two point at the text they hold between them.
 */
internal class HandleSpots(
    val start: Handle,
    val end: Handle,
    /** The selection's top and its knobs' bottom: the toolbar keeps clear of both. */
    val top: Float,
    val bottom: Float,
    /** Where the toolbar centers: the selection's middle on one row, the view's on several. */
    val centerX: Float?,
) {
    /** The handle whose knob is within [reach] of [position], the nearer of the two; `null` for neither. */
    fun nearest(
        position: Offset,
        reach: Float,
    ): Handle? =
        listOf(start, end)
            .map { it to (it.knob - position).getDistance() }
            .filter { it.second <= reach }
            .minByOrNull { it.second }
            ?.first
}

/**
 * The handles of [selection] (a frame's rows, whole) drawn in [font]'s cells, with knobs of [knob] pixels, in a view
 * [width] pixels wide: a knob that would leave the view hangs on the other side of its stem instead.
 */
internal fun handleSpots(
    selection: List<SelectedSpan>,
    font: TerminalFont,
    knob: Float,
    width: Float,
): HandleSpots? {
    val first = selection.minByOrNull { it.row } ?: return null
    val last = selection.maxBy { it.row }
    val cellHeight = font.cellHeight.toFloat()
    val startCell = CellPoint(first.first.toUShort(), first.row.toUInt())
    val endCell = CellPoint(last.last.toUShort(), last.row.toUInt())
    val startX = font.left(first.first)
    val endX = font.left(last.last + 1)
    val startFoot = font.top(first.row) + cellHeight
    val endFoot = font.top(last.row) + cellHeight
    val startRight = startX - 2 * knob < 0
    val endLeft = endX + 2 * knob > width
    val center = { column: Int, row: Int ->
        Offset(font.left(column) + font.cellWidth / 2, font.top(row) + cellHeight / 2)
    }
    return HandleSpots(
        start =
            Handle(
                cell = startCell,
                fixed = endCell,
                cellCenter = center(first.first, first.row),
                knob = Offset(if (startRight) startX + knob else startX - knob, startFoot + knob),
                squareLeft = startRight,
                stem = Rect(startX, font.top(first.row), startX, startFoot),
            ),
        end =
            Handle(
                cell = endCell,
                fixed = startCell,
                cellCenter = center(last.last, last.row),
                knob = Offset(if (endLeft) endX - knob else endX + knob, endFoot + knob),
                squareLeft = !endLeft,
                stem = Rect(endX, font.top(last.row), endX, endFoot),
            ),
        top = font.top(first.row),
        bottom = endFoot + 2 * knob,
        centerX = if (first.row == last.row) (startX + endX) / 2 else null,
    )
}

/** Draws [spots]' stems and knobs in [color] (the palette's cursor color: the terminal's own copper). */
internal fun DrawScope.drawHandles(
    spots: HandleSpots,
    color: Color,
) {
    val stem = StemWidth.toPx()
    val knob = KnobRadius.toPx()
    // a stem, and a circle with the quarter at the stem squared off: the start's top right, the end's top left (or the
    // other way round where a knob hangs on the other side of its stem, at the view's edge)
    for (handle in listOf(spots.start, spots.end)) {
        drawRect(color, Offset(handle.stem.left - stem / 2, handle.stem.top), Size(stem, handle.stem.height))
        drawCircle(color, knob, handle.knob)
        val left = if (handle.squareLeft) handle.knob.x - knob else handle.knob.x
        drawRect(color, Offset(left, handle.knob.y - knob), Size(knob, knob))
    }
}

/**
 * The touch selection's toolbar: Copy, Paste and Select all on a tonal strip, placed over the selection, or under
 * its handles when there is no room above, centered on it and kept inside the view.
 */
@Composable
internal fun SelectionToolbar(
    spots: HandleSpots,
    onCopy: () -> Unit,
    onPaste: () -> Unit,
    onSelectAll: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val gap = with(LocalDensity.current) { ToolbarGap.toPx() }
    Layout(
        modifier = modifier,
        content = {
            Surface(
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
            ) {
                Row {
                    TextButton(onClick = onCopy) { Text(stringResource(R.string.action_copy)) }
                    TextButton(onClick = onPaste) { Text(stringResource(R.string.terminal_paste)) }
                    TextButton(onClick = onSelectAll) { Text(stringResource(R.string.terminal_select_all)) }
                }
            }
        },
    ) { measurables, constraints ->
        val bar = measurables.single().measure(constraints.copy(minWidth = 0, minHeight = 0))
        val width = constraints.maxWidth
        val height = constraints.maxHeight
        val above = spots.top - gap - bar.height
        val below = spots.bottom + gap
        val y =
            when {
                above >= 0 -> above
                below + bar.height <= height -> below
                else -> (height - bar.height) / 2f
            }
        val centerX = spots.centerX ?: (width / 2f)
        val x = (centerX - bar.width / 2f).coerceIn(gap, (width - bar.width - gap).coerceAtLeast(gap))
        layout(width, height) { bar.place(x.roundToInt(), y.roundToInt()) }
    }
}

/**
 * The right-click menu at [at] (the view's pixels): Copy while there is a selection, Paste and Select all.
 */
@Composable
internal fun TerminalMenu(
    at: Offset,
    hasSelection: Boolean,
    onCopy: () -> Unit,
    onPaste: () -> Unit,
    onSelectAll: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Box(modifier.offset { at.round() }) {
        DropdownMenu(
            expanded = true,
            onDismissRequest = onDismiss,
            shape = MaterialTheme.shapes.medium,
            containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        ) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.action_copy)) },
                onClick = onCopy,
                enabled = hasSelection,
            )
            DropdownMenuItem(text = { Text(stringResource(R.string.terminal_paste)) }, onClick = onPaste)
            DropdownMenuItem(text = { Text(stringResource(R.string.terminal_select_all)) }, onClick = onSelectAll)
        }
    }
}
