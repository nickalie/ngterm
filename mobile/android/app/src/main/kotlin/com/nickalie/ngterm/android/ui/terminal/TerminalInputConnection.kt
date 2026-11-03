package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Canvas
import android.graphics.Paint
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusEventModifierNode
import androidx.compose.ui.focus.FocusState
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.key.KeyInputModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.platform.InspectorInfo
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.platform.PlatformTextInputModifierNode
import androidx.compose.ui.platform.establishTextInputSession
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import androidx.compose.ui.input.key.KeyEvent as ComposeKeyEvent

/**
 * The input method's connection to a terminal: what it commits goes to the program as typed text (line breaks as
 * Enter), what it composes waits at the cursor ([TerminalKeyboard.compose]) until it commits, and the keys it sends
 * (Enter, backspace) go as keys. Its own text is only the composition: once committed nothing is left in it, so a
 * deletion with nothing composed is a backspace for the program.
 */
internal class TerminalInputConnection(
    view: View,
    private val keyboard: TerminalKeyboard,
) : BaseInputConnection(view, true) {
    private fun composed(): String = editable?.toString().orEmpty()

    private fun clear() {
        editable?.let { text ->
            removeComposingSpans(text)
            text.clear()
        }
        keyboard.compose("")
    }

    override fun commitText(
        text: CharSequence?,
        newCursorPosition: Int,
    ): Boolean {
        clear()
        keyboard.type(text?.toString().orEmpty())
        return true
    }

    override fun setComposingText(
        text: CharSequence?,
        newCursorPosition: Int,
    ): Boolean {
        super.setComposingText(text, newCursorPosition)
        keyboard.compose(composed())
        return true
    }

    override fun finishComposingText(): Boolean {
        val pending = composed()
        super.finishComposingText()
        clear()
        if (pending.isNotEmpty()) keyboard.type(pending)
        return true
    }

    override fun deleteSurroundingText(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean {
        if (composed().isNotEmpty()) {
            super.deleteSurroundingText(beforeLength, afterLength)
            keyboard.compose(composed())
        } else {
            repeat(beforeLength) { keyboard.press(BACKSPACE) }
            repeat(afterLength) { keyboard.press(DELETE) }
        }
        return true
    }

    override fun deleteSurroundingTextInCodePoints(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean = deleteSurroundingText(beforeLength, afterLength)

    override fun sendKeyEvent(event: KeyEvent): Boolean {
        keyboard.key(event)
        return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
        keyboard.press(ENTER)
        return true
    }

    private companion object {
        const val BACKSPACE = "backspace"
        const val DELETE = "delete"
        const val ENTER = "enter"
    }
}

/**
 * What a terminal asks of the input method: plain text with no suggestions, autocorrect, automatic capitals or
 * learning (it types commands and passwords), Enter as a key rather than an action, and no full-screen editor.
 */
internal fun EditorInfo.forTerminal() {
    inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    imeOptions =
        EditorInfo.IME_ACTION_NONE or
        EditorInfo.IME_FLAG_NO_ENTER_ACTION or
        EditorInfo.IME_FLAG_NO_EXTRACT_UI or
        EditorInfo.IME_FLAG_NO_FULLSCREEN or
        EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
    initialSelStart = 0
    initialSelEnd = 0
    initialCapsMode = 0
}

/**
 * A terminal's keyboard input: while it has focus, the input method talks to it through a [TerminalInputConnection]
 * and [keyboard] sends to [state]; a hardware key goes through [keyboard] as well. It goes before the focus target
 * whose focus it follows.
 */
internal fun Modifier.terminalTextInput(
    keyboard: TerminalKeyboard,
    state: TerminalState,
): Modifier = this then TerminalTextInputElement(keyboard, state)

private data class TerminalTextInputElement(
    val keyboard: TerminalKeyboard,
    val state: TerminalState,
) : ModifierNodeElement<TerminalTextInputNode>() {
    override fun create() = TerminalTextInputNode(keyboard, state)

    override fun update(node: TerminalTextInputNode) {
        node.update(keyboard, state)
    }

    override fun InspectorInfo.inspectableProperties() {
        name = "terminalTextInput"
    }
}

private class TerminalTextInputNode(
    var keyboard: TerminalKeyboard,
    var state: TerminalState,
) : Modifier.Node(),
    PlatformTextInputModifierNode,
    FocusEventModifierNode,
    KeyInputModifierNode {
    private var session: Job? = null
    private var focused = false

    fun update(
        keyboard: TerminalKeyboard,
        state: TerminalState,
    ) {
        if (keyboard === this.keyboard && state === this.state) return
        this.keyboard = keyboard
        this.state = state
        if (focused) {
            keyboard.target = state
            restart()
        }
    }

    override fun onFocusEvent(focusState: FocusState) {
        if (focusState.isFocused == focused) return
        focused = focusState.isFocused
        if (focused) {
            keyboard.target = state
            restart()
        } else {
            session?.cancel()
            session = null
            state.composing = ""
        }
    }

    private fun restart() {
        session?.cancel()
        val keyboard = keyboard
        session =
            coroutineScope.launch {
                establishTextInputSession {
                    val view = view
                    startInputMethod(
                        object : PlatformTextInputMethodRequest {
                            override fun createInputConnection(outAttributes: EditorInfo): InputConnection {
                                outAttributes.forTerminal()
                                return TerminalInputConnection(view, keyboard)
                            }
                        },
                    )
                }
            }
    }

    override fun onDetach() {
        session = null
        focused = false
    }

    override fun onKeyEvent(event: ComposeKeyEvent): Boolean = keyboard.key(event.nativeKeyEvent)

    override fun onPreKeyEvent(event: ComposeKeyEvent): Boolean = false
}

/**
 * Draws the input method's composition at a terminal's cursor: on the terminal's ground in its text color, underlined
 * in the cursor's color, moved left as far as it needs to fit the view.
 */
internal class CompositionPainter(
    private val font: TerminalFont,
    /** The underline's thickness in pixels. */
    private val underline: Float,
) {
    private val fill = Paint()

    /** Draws [text] at [state]'s cursor on [canvas], [width] pixels wide, in [palette]'s colors. */
    fun draw(
        canvas: Canvas,
        text: String,
        state: TerminalState,
        palette: TerminalPalette,
        width: Float,
    ) {
        if (text.isEmpty()) return
        val cursor = state.screen.header?.cursor ?: return
        val paint = font.paint(0)
        val textWidth = paint.measureText(text)
        val left = font.left(cursor.column).coerceAtMost(width - textWidth).coerceAtLeast(0f)
        val top = font.top(cursor.row)
        val bottom = top + font.cellHeight
        fill.color = palette.background.toArgb()
        canvas.drawRect(left, top, left + textWidth, bottom, fill)
        paint.color = palette.foreground.toArgb()
        canvas.drawText(text, left, top + font.baseline, paint)
        fill.color = palette.cursor.toArgb()
        canvas.drawRect(left, bottom - underline, left + textWidth, bottom, fill)
    }
}
