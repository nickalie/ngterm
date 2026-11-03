package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.PointerKeyboardModifiers
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.isAltPressed
import androidx.compose.ui.input.pointer.isCtrlPressed
import androidx.compose.ui.input.pointer.isMetaPressed
import androidx.compose.ui.input.pointer.isPrimaryPressed
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.isShiftPressed
import androidx.compose.ui.input.pointer.isTertiaryPressed
import androidx.compose.ui.input.pointer.pointerInput
import com.nickalie.ngterm.core.ffi.PaneMouseAction
import com.nickalie.ngterm.core.ffi.PaneMouseButton

private const val MOD_SHIFT = 1
private const val MOD_CTRL = 2
private const val MOD_ALT = 4
private const val MOD_SUPER = 8

/** The modifiers held as the pane's bits: shift 1, ctrl 2, alt 4, super 8. */
internal fun PointerKeyboardModifiers.bits(): UByte {
    var bits = 0
    if (isShiftPressed) bits = bits or MOD_SHIFT
    if (isCtrlPressed) bits = bits or MOD_CTRL
    if (isAltPressed) bits = bits or MOD_ALT
    if (isMetaPressed) bits = bits or MOD_SUPER
    return bits.toUByte()
}

/**
 * The wheel (a mouse's, or a trackpad's two-finger scroll) scrolls the pane, which reports it to a program that asked
 * for the mouse; the mouse moving with no button down reports its motion to a program tracking it.
 */
internal fun Modifier.terminalMouse(input: TerminalInput): Modifier =
    pointerInput(input) {
        awaitPointerEventScope {
            while (true) {
                val event = awaitPointerEvent()
                val change = event.changes.firstOrNull()
                if (change != null && event.type == PointerEventType.Scroll) {
                    input.scroll(change.scrollDelta, event.keyboardModifiers.bits(), change.position)
                    change.consume()
                }
                if (change != null && isHover(event, change) && input.tracking) {
                    input.hover(change.position, event.keyboardModifiers.bits())
                }
            }
        }
    }

// the mouse came in or moved with no button down
private fun isHover(
    event: PointerEvent,
    change: PointerInputChange,
): Boolean {
    val moved = event.type == PointerEventType.Move || event.type == PointerEventType.Enter
    return moved && change.type == PointerType.Mouse && !change.pressed
}

/**
 * A mouse button pressed on the terminal: under mouse tracking (shift held overrides it) the button, the drag and
 * the release go to the program; otherwise a left click lets the selection go, a double click selects the word and a
 * triple one the line, a left drag selects, and the right button opens the menu.
 */
internal suspend fun AwaitPointerEventScope.mouseGesture(
    input: TerminalInput,
    selection: TerminalSelection,
    down: PointerInputChange,
    event: PointerEvent,
) {
    val mods = event.keyboardModifiers
    val button =
        when {
            event.buttons.isSecondaryPressed -> PaneMouseButton.RIGHT
            event.buttons.isTertiaryPressed -> PaneMouseButton.MIDDLE
            else -> PaneMouseButton.LEFT
        }
    down.consume()
    when {
        input.tracking && !mods.isShiftPressed -> {
            reportDrag(input, down, button, mods.bits())
        }

        button == PaneMouseButton.RIGHT -> {
            selection.menuAt = down.position
            awaitRelease(down)
        }

        button == PaneMouseButton.LEFT -> {
            selectDrag(input, selection, down)
        }

        else -> {
            awaitRelease(down)
        }
    }
}

private suspend fun AwaitPointerEventScope.reportDrag(
    input: TerminalInput,
    down: PointerInputChange,
    button: PaneMouseButton,
    mods: UByte,
) {
    input.mouse(PaneMouseAction.PRESS, button, mods, down.position)
    var cell = input.cell(down.position)
    var released = false
    while (!released) {
        val event = awaitPointerEvent()
        val change = event.changes.firstOrNull { it.id == down.id }
        change?.consume()
        val position = change?.position ?: down.position
        val bits = event.keyboardModifiers.bits()
        released = change == null || !stillPressed(event, button)
        if (released) {
            input.mouse(PaneMouseAction.RELEASE, button, bits, position)
        } else if (input.cell(position) != cell) {
            cell = input.cell(position)
            input.mouse(PaneMouseAction.MOTION, button, bits, position)
        }
    }
}

private fun stillPressed(
    event: PointerEvent,
    button: PaneMouseButton,
): Boolean {
    val buttons = event.buttons
    val held =
        when (button) {
            PaneMouseButton.LEFT -> buttons.isPrimaryPressed
            PaneMouseButton.RIGHT -> buttons.isSecondaryPressed
            PaneMouseButton.MIDDLE -> buttons.isTertiaryPressed
        }
    return held && event.changes.any { it.pressed }
}

private suspend fun AwaitPointerEventScope.selectDrag(
    input: TerminalInput,
    selection: TerminalSelection,
    down: PointerInputChange,
) {
    val anchor = input.cell(down.position)
    val clicks = anchor?.let { selection.mouseClick(it, down.uptimeMillis, viewConfiguration.doubleTapTimeoutMillis) }
    if (anchor != null && clicks == 1) {
        extendWhileHeld(input, selection, down, Grip(anchor, anchor))
    } else {
        awaitRelease(down)
    }
}

private suspend fun AwaitPointerEventScope.awaitRelease(down: PointerInputChange) {
    do {
        val held = awaitHeld(down)
    } while (held != null)
}
