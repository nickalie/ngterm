package com.nickalie.ngterm.android.ui.terminal

import android.content.res.Configuration
import android.graphics.Canvas
import android.graphics.Paint
import android.text.InputType
import android.text.Selection
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusEventModifierNode
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.FocusState
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.key.KeyInputModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.ObserverModifierNode
import androidx.compose.ui.node.observeReads
import androidx.compose.ui.node.requireView
import androidx.compose.ui.platform.InspectorInfo
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.platform.PlatformTextInputModifierNode
import androidx.compose.ui.platform.establishTextInputSession
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import androidx.compose.ui.input.key.KeyEvent as ComposeKeyEvent

/**
 * The input method's connection to [terminal], the terminal it was opened for: what it commits goes to that program
 * as typed text (line breaks as Enter), what it composes waits at the cursor ([TerminalState.composing]) until it
 * commits, and the keys it sends (Enter, backspace) go as keys, after the composition. Its own text is only the
 * composition: once committed nothing is left in it, so a deletion past the composition is the program's (backspace
 * before it, delete after it). While a composition waits, a key from elsewhere commits it first and restarts the
 * input method ([TerminalKeyboard.composition]).
 *
 * It types only while [terminal] is the keyboard's target and until it is [close]d: what the input method sends to a
 * connection whose terminal is not (or no longer) the one selected lands nowhere, so it never reaches another program.
 */
internal class TerminalInputConnection(
    private val host: View,
    private val keyboard: TerminalKeyboard,
    val terminal: TerminalState,
) : BaseInputConnection(host, true) {
    private var closed = false

    // whether what the input method sends now reaches the terminal
    private val live get() = !closed && keyboard.target === terminal

    // what a key from elsewhere does with the composition waiting here: commits it and tells the input method
    private val pending =
        PendingComposition {
            finishComposingText()
            host.context.getSystemService(InputMethodManager::class.java)?.restartInput(host)
        }

    private fun composed(): String = editable?.toString().orEmpty()

    // the composition, as it now stands, at the terminal's cursor and waiting for a key from elsewhere
    private fun show() {
        val text = composed()
        terminal.composing = text
        if (text.isNotEmpty()) {
            keyboard.composition = pending
        } else if (keyboard.composition === pending) {
            keyboard.composition = null
        }
    }

    private fun clear() {
        editable?.let { text ->
            removeComposingSpans(text)
            text.clear()
        }
        show()
    }

    /**
     * Ends this connection: its composition is finished into its terminal while that is still the target (dropped
     * otherwise), and nothing it is sent afterwards lands anywhere.
     */
    fun close() {
        if (closed) return
        val waiting = composed().isNotEmpty()
        finishComposingText()
        closed = true
        if (waiting) host.context.getSystemService(InputMethodManager::class.java)?.restartInput(host)
    }

    override fun closeConnection() {
        close()
        super.closeConnection()
    }

    override fun commitText(
        text: CharSequence?,
        newCursorPosition: Int,
    ): Boolean {
        if (!live) return false
        clear()
        keyboard.type(text?.toString().orEmpty(), terminal)
        return true
    }

    override fun setComposingText(
        text: CharSequence?,
        newCursorPosition: Int,
    ): Boolean {
        if (!live) return false
        super.setComposingText(text, newCursorPosition)
        show()
        return true
    }

    override fun finishComposingText(): Boolean {
        val pending = composed()
        super.finishComposingText()
        clear()
        if (pending.isNotEmpty() && live) keyboard.type(pending, terminal)
        return true
    }

    override fun deleteSurroundingText(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean {
        if (!live) return false
        var before = beforeLength.coerceAtLeast(0)
        var after = afterLength.coerceAtLeast(0)
        val text = editable
        if (!text.isNullOrEmpty()) {
            // the part of the deletion the composition holds goes from it; the rest is the program's
            val start = Selection.getSelectionStart(text).coerceIn(0, text.length)
            val end = Selection.getSelectionEnd(text).coerceIn(start, text.length)
            val localAfter = minOf(after, text.length - end)
            val localBefore = minOf(before, start)
            text.delete(end, end + localAfter)
            text.delete(start - localBefore, start)
            if (text.isEmpty()) removeComposingSpans(text)
            before -= localBefore
            after -= localAfter
            show()
        }
        repeat(before) { keyboard.press(BACKSPACE, terminal) }
        repeat(after) { keyboard.press(DELETE, terminal) }
        return true
    }

    override fun deleteSurroundingTextInCodePoints(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean = deleteSurroundingText(beforeLength, afterLength)

    override fun sendKeyEvent(event: KeyEvent): Boolean {
        if (!live) return false
        if (event.action == KeyEvent.ACTION_DOWN && composed().isNotEmpty()) finishComposingText()
        keyboard.key(event, terminal, commitFirst = false)
        return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
        if (!live) return false
        if (composed().isNotEmpty()) finishComposingText()
        keyboard.press(ENTER, terminal)
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
 * Gives a terminal the focus, with the soft keyboard or without it: a tap wants the keyboard, a mouse click only the
 * focus. Starting the input method's session opens the soft keyboard, so the terminal starts one on focus only when
 * the keyboard is wanted or a hardware keyboard is attached (which keeps the soft one closed); a later tap starts it.
 */
internal class SoftKeyboardRequest(
    private val focus: FocusRequester,
    private val view: View,
) {
    /** Whether the focus being taken wants the soft keyboard: until a mouse says otherwise, it does. */
    var wanted = true
        private set

    // the focused terminal's node, starting its session when a tap wants the keyboard
    internal var onWanted: (() -> Unit)? = null

    /** Gives the terminal the focus and, when [show], the soft keyboard. */
    fun request(show: Boolean) {
        wanted = show
        if (show) onWanted?.invoke()
        focus.requestFocus()
        if (show) view.context.getSystemService(InputMethodManager::class.java)?.showSoftInput(view, 0)
    }
}

/**
 * A terminal's keyboard input: while it has focus and is [keyboard]'s target, the input method talks to it through a
 * [TerminalInputConnection]; a hardware key goes through [keyboard] to its target (a chord runs even with none).
 * Taking the focus makes it the target only where the keyboard says so ([TerminalKeyboard.selectsOnFocus]); a
 * focused terminal the target leaves keeps the focus but lets its input method session go, and opens one again when
 * the target comes back. [request] says whether a focus wants the soft keyboard. It goes before the focus target
 * whose focus it follows.
 */
internal fun Modifier.terminalTextInput(
    keyboard: TerminalKeyboard,
    state: TerminalState,
    request: SoftKeyboardRequest,
): Modifier = this then TerminalTextInputElement(keyboard, state, request)

private data class TerminalTextInputElement(
    val keyboard: TerminalKeyboard,
    val state: TerminalState,
    val request: SoftKeyboardRequest,
) : ModifierNodeElement<TerminalTextInputNode>() {
    override fun create() = TerminalTextInputNode(keyboard, state, request)

    override fun update(node: TerminalTextInputNode) {
        node.update(keyboard, state, request)
    }

    override fun InspectorInfo.inspectableProperties() {
        name = "terminalTextInput"
    }
}

private class TerminalTextInputNode(
    var keyboard: TerminalKeyboard,
    var state: TerminalState,
    var request: SoftKeyboardRequest,
) : Modifier.Node(),
    PlatformTextInputModifierNode,
    FocusEventModifierNode,
    KeyInputModifierNode,
    ObserverModifierNode {
    private var session: Job? = null
    private var focused = false
    private val startWanted: () -> Unit = { if (focused && session == null && owns()) start() }

    private fun owns() = keyboard.target === state

    override fun onAttach() = follow()

    override fun onObservedReadsChanged() = follow()

    // the input method session goes with the target: let go when it leaves, opened again (for a focus that wanted
    // it, or a hardware keyboard) when it comes back
    private fun follow() {
        var owns = false
        observeReads { owns = owns() }
        when {
            !owns -> stop()
            focused && session == null && (request.wanted || hardwareKeyboard()) -> start()
        }
    }

    fun update(
        keyboard: TerminalKeyboard,
        state: TerminalState,
        request: SoftKeyboardRequest,
    ) {
        if (request !== this.request) {
            if (focused) this.request.onWanted = null
            this.request = request
            if (focused) request.onWanted = startWanted
        }
        if (keyboard === this.keyboard && state === this.state) return
        // the old connection is bound to the old terminal: closing it commits its composition there
        stop()
        this.keyboard = keyboard
        this.state = state
        if (focused && keyboard.selectsOnFocus) keyboard.target = state
        if (isAttached) follow()
    }

    override fun onFocusEvent(focusState: FocusState) {
        if (focusState.isFocused == focused) return
        focused = focusState.isFocused
        if (focused) {
            if (keyboard.selectsOnFocus) keyboard.target = state
            request.onWanted = startWanted
            follow()
        } else {
            if (request.onWanted === startWanted) request.onWanted = null
            stop()
            state.composing = ""
        }
    }

    private fun hardwareKeyboard(): Boolean {
        val configuration = requireView().resources.configuration
        return configuration.keyboard != Configuration.KEYBOARD_NOKEYS &&
            configuration.hardKeyboardHidden == Configuration.HARDKEYBOARDHIDDEN_NO
    }

    private fun stop() {
        keyboard.connections.release(state)
        session?.cancel()
        session = null
    }

    private fun start() {
        session?.cancel()
        val keyboard = keyboard
        val terminal = state
        session =
            coroutineScope.launch {
                establishTextInputSession {
                    val view = view
                    startInputMethod(
                        object : PlatformTextInputMethodRequest {
                            override fun createInputConnection(outAttributes: EditorInfo): InputConnection {
                                outAttributes.forTerminal()
                                val connection = TerminalInputConnection(view, keyboard, terminal)
                                keyboard.connections.bind(connection)
                                return connection
                            }
                        },
                    )
                }
            }
    }

    override fun onDetach() {
        if (request.onWanted === startWanted) request.onWanted = null
        keyboard.connections.release(state)
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
