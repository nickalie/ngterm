package com.nickalie.ngterm.android.ui.terminal

import android.annotation.SuppressLint
import android.content.res.Configuration
import android.icu.lang.UCharacter
import android.icu.lang.UProperty
import android.icu.lang.UScript
import android.icu.text.BreakIterator
import android.os.Build
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.annotation.RequiresApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusEventModifierNode
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.FocusState
import androidx.compose.ui.input.key.KeyInputModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.ObserverModifierNode
import androidx.compose.ui.node.observeReads
import androidx.compose.ui.node.requireView
import androidx.compose.ui.platform.InspectorInfo
import androidx.compose.ui.platform.PlatformTextInputMethodRequest
import androidx.compose.ui.platform.PlatformTextInputModifierNode
import androidx.compose.ui.platform.establishTextInputSession
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import androidx.compose.ui.input.key.KeyEvent as ComposeKeyEvent

/**
 * The input method's connection to [terminal], the terminal it was opened for: everything it types reaches that program
 * as it is typed, so a program reading each key (a shell's completion, a TUI's `/` menu) sees each letter. What it
 * commits goes as typed text (line breaks as Enter). What it composes goes as the composition grows, as far as a
 * backspace takes each character back alone in every line editor (one with no marks, joins or parts, as typed letters
 * are); from the first character a backspace may not (a letter with its marks, a conjunct, an emoji sequence) the rest
 * is held ([TerminalState.composing], shown in the [CompositionStrip]) and typed when the composition ends, so a change
 * to the composition (a conversion, a correction) is made on the program's line with backspaces only over such
 * characters, and comes out the same whether the program deletes by grapheme cluster (Claude Code, Codex), by code
 * point with its marks (readline) or by code point (a terminal's own line editing). A deletion it asks for is around
 * the composition, which stays, as in the framework's editors (backspaces before it, deletes after it); the keys it
 * sends (Enter, backspace) go as keys. A key (its own or one from elsewhere: [TerminalKeyboard.composition]), the
 * pointer or a paste reaching the program, a line break or a composed letter a sticky modifier turned into a chord ends
 * the composition and restarts the input method, so it never corrects text the program has moved past. A held part goes
 * first, where it was typed: a pointer press, drag or wheel turn or a paste types it and ends the composition before
 * reaching the program, whether the program then takes it or not (a scroll of the view too).
 *
 * It types only while [terminal] is the keyboard's target and until it is [close]d: what the input method sends to a
 * connection whose terminal is not (or no longer) the one selected lands nowhere, so it never reaches another program.
 */
internal class TerminalInputConnection(
    private val host: View,
    private val keyboard: TerminalKeyboard,
    val terminal: TerminalState,
) : BaseInputConnection(host, true) {
    /** Whether it has ended: nothing it is sent lands anywhere. */
    var closed = false
        private set

    // whether what the input method sends now reaches the terminal
    private val live get() = !closed && keyboard.target === terminal

    // the composition as the program has it: the end of its line, where its cursor is
    private var sent = ""

    // the rest of the composition, from the first character a backspace may not take back alone
    private var held = ""

    private val pending =
        object : PendingComposition {
            override val holds get() = held.isNotEmpty()

            override fun end() = this@TerminalInputConnection.end(restart = true)
        }

    private fun composed(): String = editable?.toString().orEmpty()

    // line editors disagree on what a backspace takes beyond a lone letter, so the rest waits for the composition's end
    private fun send(
        text: String,
        whole: Boolean,
    ) {
        val ready = if (whole) text else text.substring(0, text.lettersAlone())
        val kept = sent.commonPrefixWith(ready).length
        repeat(sent.codePointCount(kept, sent.length)) { keyboard.press(BACKSPACE, terminal) }
        if (kept < ready.length) keyboard.type(ready.substring(kept), terminal)
        sent = ready
        hold(text.substring(ready.length))
    }

    private fun hold(text: String) {
        if (terminal.composing == held) terminal.composing = text
        held = text
    }

    // the composition, as it now stands, onto the program's line
    private fun update() {
        val text = composed()
        if (text == sent + held) return
        val ends = keyboard.holdsSticky || '\n' in text
        send(text, whole = ends)
        when {
            ends -> end(restart = true)
            text.isNotEmpty() -> keyboard.composition = pending
            keyboard.composition === pending -> keyboard.composition = null
        }
    }

    // the held part goes where it was typed; [restart] tells the input method to forget the composition too
    private fun end(restart: Boolean) {
        val waiting = sent.isNotEmpty() || held.isNotEmpty() || composed().isNotEmpty()
        if (live && held.isNotEmpty()) keyboard.type(held, terminal)
        hold("")
        editable?.let { text ->
            removeComposingSpans(text)
            text.clear()
        }
        sent = ""
        if (keyboard.composition === pending) keyboard.composition = null
        if (restart && waiting) host.context.getSystemService(InputMethodManager::class.java)?.restartInput(host)
    }

    /** Ends this connection: its composition stays where it was typed, and nothing sent afterwards lands anywhere. */
    fun close() {
        if (closed) return
        end(restart = true)
        closed = true
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
        send(text?.toString().orEmpty(), whole = true)
        end(restart = false)
        return true
    }

    override fun setComposingText(
        text: CharSequence?,
        newCursorPosition: Int,
    ): Boolean {
        if (!live) return false
        super.setComposingText(text, newCursorPosition)
        update()
        return true
    }

    override fun finishComposingText(): Boolean {
        super.finishComposingText()
        end(restart = false)
        return true
    }

    override fun deleteSurroundingText(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean = delete(beforeLength, afterLength)

    override fun deleteSurroundingTextInCodePoints(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean = delete(beforeLength, afterLength)

    // input methods expect the framework editors' deletion: around the composition, which stays
    private fun delete(
        beforeLength: Int,
        afterLength: Int,
    ): Boolean {
        if (!live) return false
        val around = beforeLength > 0 && sent.isNotEmpty()
        if (around) send("", whole = false)
        repeat(beforeLength.coerceAtLeast(0)) { keyboard.press(BACKSPACE, terminal) }
        if (around) send(composed(), whole = false)
        repeat(afterLength.coerceAtLeast(0)) { keyboard.press(DELETE, terminal) }
        return true
    }

    override fun sendKeyEvent(event: KeyEvent): Boolean {
        if (!live) return false
        if (event.action == KeyEvent.ACTION_DOWN) end(restart = true)
        keyboard.key(event, terminal, endFirst = false)
        return true
    }

    override fun performEditorAction(actionCode: Int): Boolean {
        if (!live) return false
        end(restart = true)
        keyboard.press(ENTER, terminal)
        return true
    }

    private companion object {
        // nothing a grapheme editor could join to what is before it, nor a mark a code point editor would leave alone
        fun Int.alone(): Boolean =
            UCharacter.getIntPropertyValue(this, UProperty.GRAPHEME_CLUSTER_BREAK) in ALONE_BREAKS &&
                UScript.getScript(this) !in CONJUNCT_SCRIPTS &&
                Character.getType(this) != Character.OTHER_SYMBOL.toInt() &&
                !(Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && pictographic())

        @RequiresApi(Build.VERSION_CODES.Q)
        fun Int.pictographic() = UCharacter.hasBinaryProperty(this, UProperty.EXTENDED_PICTOGRAPHIC)

        // held text starts a whole character, never a mark split from its letter
        fun String.lettersAlone(): Int {
            val characters = BreakIterator.getCharacterInstance().also { it.setText(this) }
            var at = 0
            while (at < length) {
                val next = characters.following(at)
                val letter = codePointAt(at)
                if (next != at + Character.charCount(letter) || !letter.alone()) return at
                at = next
            }
            return at
        }

        val ALONE_BREAKS =
            setOf(
                UCharacter.GraphemeClusterBreak.OTHER,
                UCharacter.GraphemeClusterBreak.LV,
                UCharacter.GraphemeClusterBreak.LVT,
            )

        // Indic_Conjunct_Break scripts listed by hand: the device's ICU may predate the program's Unicode
        @SuppressLint("InlinedApi")
        val CONJUNCT_SCRIPTS =
            setOf(
                UScript.BALINESE,
                UScript.BENGALI,
                UScript.CHAKMA,
                UScript.DEVANAGARI,
                UScript.DIVES_AKURU,
                UScript.GUJARATI,
                UScript.JAVANESE,
                UScript.KAWI,
                UScript.KHAROSHTHI,
                UScript.KHMER,
                UScript.LANNA,
                UScript.MALAYALAM,
                UScript.MEITEI_MAYEK,
                UScript.MYANMAR,
                UScript.ORIYA,
                UScript.SOYOMBO,
                UScript.SUNDANESE,
                UScript.TELUGU,
                UScript.TULU_TIGALARI,
                UScript.ZANABAZAR_SQUARE,
            )

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
        // the old connection is bound to the old terminal: closing it ends its composition there
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
