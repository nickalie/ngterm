package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.input.pointer.util.addPointerInputChange
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.core.ffi.CellPoint
import com.nickalie.ngterm.core.ffi.PaneMouseAction
import com.nickalie.ngterm.core.ffi.PaneMouseButton
import kotlin.math.abs

/** How near a selection handle's knob a finger lands to take it. */
private val HandleReach = 24.dp

/**
 * The terminal's pointer input, through [input] and [selection]:
 * - a finger: a drag scrolls (the pane reports it to a program that asked for the mouse, as wheel turns) and flings
 *   on; a long press selects the word with handles and a toolbar, and a drag from it extends the selection; a drag
 *   on a handle ([handles]) moves that end; two fingers pinch the text size; a tap clicks under mouse tracking and
 *   otherwise lets a touch selection go;
 * - a mouse or trackpad: as [terminalMouse] says.
 */
internal fun Modifier.terminalGestures(
    input: TerminalInput,
    selection: TerminalSelection,
    haptics: HapticFeedback,
    handles: () -> HandleSpots?,
    textSize: () -> Float,
): Modifier =
    pointerInput(input, selection) {
        awaitEachGesture {
            val down = awaitFirstDown(requireUnconsumed = false)
            if (down.type == PointerType.Mouse) {
                mouseGesture(input, selection, down, currentEvent)
            } else {
                input.stopFling()
                val handle = handles()?.nearest(down.position, HandleReach.toPx())
                if (handle != null) {
                    down.consume()
                    // the finger holds the knob under the row: it moves the cell the knob hangs from
                    adjust(input, selection, down, Grip(handle.fixed, handle.cell, handle.cellCenter - down.position))
                } else {
                    touchGesture(input, selection, down, haptics, textSize())
                }
            }
        }
    }.terminalMouse(input)

/** What a finger turned out to be doing before the long-press timeout. */
private enum class Touch { Tap, Drag, Pinch }

private suspend fun AwaitPointerEventScope.touchGesture(
    input: TerminalInput,
    selection: TerminalSelection,
    down: PointerInputChange,
    haptics: HapticFeedback,
    textSize: Float,
) {
    // every move counts toward a fling's speed, those before the drag was told apart too
    val velocity = VelocityTracker()
    velocity.addPointerInputChange(down)
    when (withTimeoutOrNull(viewConfiguration.longPressTimeoutMillis) { classify(down, velocity) }) {
        null -> {
            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
            val anchor = input.cell(down.position)
            if (anchor != null) {
                selection.selectWordAt(anchor)
                adjust(input, selection, down, Grip(anchor, anchor))
            }
        }

        Touch.Tap -> {
            if (input.tracking) {
                input.mouse(PaneMouseAction.PRESS, PaneMouseButton.LEFT, 0u, down.position)
                input.mouse(PaneMouseAction.RELEASE, PaneMouseButton.LEFT, 0u, down.position)
            } else if (selection.touchSelection) {
                selection.clearSelection()
            }
        }

        Touch.Drag -> {
            scrollDrag(input, down, velocity, textSize)
        }

        Touch.Pinch -> {
            pinch(input, textSize)
        }
    }
}

private suspend fun AwaitPointerEventScope.classify(
    down: PointerInputChange,
    velocity: VelocityTracker,
): Touch {
    val slop = viewConfiguration.touchSlop
    var touch: Touch? = null
    while (touch == null) {
        val event = awaitPointerEvent()
        val change = event.changes.firstOrNull { it.id == down.id }
        change?.let(velocity::addPointerInputChange)
        touch =
            when {
                event.changes.count { it.pressed } > 1 -> Touch.Pinch
                change == null || !change.pressed -> Touch.Tap
                (change.position - down.position).getDistance() > slop -> Touch.Drag
                else -> null
            }
        if (touch == Touch.Tap) change?.consume()
    }
    return touch
}

private suspend fun AwaitPointerEventScope.scrollDrag(
    input: TerminalInput,
    down: PointerInputChange,
    velocity: VelocityTracker,
    textSize: Float,
) {
    var last = down.position
    var pinched = false
    var lifted = false
    while (!pinched && !lifted) {
        val event = awaitPointerEvent()
        val change = event.changes.firstOrNull { it.id == down.id }
        pinched = event.changes.count { it.pressed } > 1
        if (change == null) {
            lifted = true
        } else if (!pinched) {
            velocity.addPointerInputChange(change)
            input.drag(change.position.y - last.y, change.position)
            last = change.position
            change.consume()
            lifted = !change.pressed
        }
    }
    if (pinched) {
        pinch(input, textSize)
    } else {
        val fling = velocity.calculateVelocity().y
        if (abs(fling) > viewConfiguration.minimumFlingVelocity) input.fling(fling, last, this)
    }
}

private suspend fun AwaitPointerEventScope.pinch(
    input: TerminalInput,
    base: Float,
) {
    var zoom = 1f
    do {
        val event = awaitPointerEvent()
        val held = event.changes.any { it.pressed }
        if (held) {
            zoom *= event.calculateZoom()
            input.zoom(base, zoom)
        }
        event.changes.forEach { it.consume() }
    } while (held)
}

/**
 * [extendWhileHeld] by a finger, with [TerminalSelection.adjusting] set meanwhile, so the toolbar keeps out of its
 * way.
 */
private suspend fun AwaitPointerEventScope.adjust(
    input: TerminalInput,
    selection: TerminalSelection,
    down: PointerInputChange,
    grip: Grip,
) {
    selection.adjusting = true
    try {
        extendWhileHeld(input, selection, down, grip)
    } finally {
        selection.adjusting = false
    }
}

/**
 * How a pointer holds the selection: [moving] is the end it moves, from the cell under the pointer plus [offset];
 * [fixed] is the other end.
 */
internal class Grip(
    val fixed: CellPoint,
    val moving: CellPoint,
    val offset: Offset = Offset.Zero,
)

/** Moves one end of the selection with [down]'s pointer, as [grip] holds it, until the pointer lifts. */
internal suspend fun AwaitPointerEventScope.extendWhileHeld(
    input: TerminalInput,
    selection: TerminalSelection,
    down: PointerInputChange,
    grip: Grip,
) {
    var last = grip.moving
    while (true) {
        val change = awaitHeld(down) ?: break
        val cell = input.cell(change.position + grip.offset)
        if (cell != null && cell != last) {
            last = cell
            selection.select(grip.fixed, cell)
        }
    }
}

/** The next event's change of [down]'s pointer, consumed, while it stays pressed; `null` once it has lifted. */
internal suspend fun AwaitPointerEventScope.awaitHeld(down: PointerInputChange): PointerInputChange? {
    val change = awaitPointerEvent().changes.firstOrNull { it.id == down.id }
    change?.consume()
    return change?.takeIf { it.pressed }
}
