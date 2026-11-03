package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyEvent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneKeyAction

/** A sticky modifier of the extra-keys row, with its bit in the pane's modifiers. */
enum class StickyModifier(
    internal val bit: Int,
) {
    Ctrl(KeyMapping.CTRL),
    Alt(KeyMapping.ALT),
}

/** Where a sticky modifier stands: a tap arms it for the next key, a second tap locks it, a third lets it go. */
enum class Sticky {
    Off,
    Once,
    Locked,
}

/**
 * What the app does for the hardware chords that move between panes and sessions ([TerminalShortcut]): the session
 * view sets them. A chord with no action here is the program's, as any other key.
 */
class TerminalShortcuts(
    val nextPane: (() -> Unit)? = null,
    val previousPane: (() -> Unit)? = null,
    val nextSession: (() -> Unit)? = null,
    val previousSession: (() -> Unit)? = null,
)

/**
 * The keyboard's side of the terminal: what the soft keyboard commits and composes, the extra-keys row and a
 * hardware keyboard's keys go to the [target] pane (the terminal that last took focus), with the row's sticky Ctrl and
 * Alt held for the next key; a hardware keyboard's app chords ([TerminalShortcut]) change the target's text size or
 * run [shortcuts]. One keyboard serves every terminal on screen, so the sticky modifiers stay as the focus moves.
 * Used on the main thread.
 */
@Stable
class TerminalKeyboard(
    shortcuts: TerminalShortcuts = TerminalShortcuts(),
) {
    /** The chords' actions. */
    var shortcuts: TerminalShortcuts by mutableStateOf(shortcuts)

    /** The terminal keys go to: the one that took focus last; `null` before any did. */
    var target: TerminalState? by mutableStateOf(null)

    /** The sticky Ctrl's state. */
    var ctrl by mutableStateOf(Sticky.Off)
        private set

    /** The sticky Alt's state. */
    var alt by mutableStateOf(Sticky.Off)
        private set

    // keys whose press ran a chord: their repeats and release are the chord's too, not the program's
    private val chordKeys = mutableSetOf<Int>()

    /** Where [modifier] stands. */
    fun sticky(modifier: StickyModifier): Sticky =
        when (modifier) {
            StickyModifier.Ctrl -> ctrl
            StickyModifier.Alt -> alt
        }

    /** A tap on [modifier]'s key: off to once, once to locked, locked to off. Returns where it stands now. */
    fun toggle(modifier: StickyModifier): Sticky {
        val next =
            when (sticky(modifier)) {
                Sticky.Off -> Sticky.Once
                Sticky.Once -> Sticky.Locked
                Sticky.Locked -> Sticky.Off
            }
        set(modifier, next)
        return next
    }

    private fun set(
        modifier: StickyModifier,
        state: Sticky,
    ) {
        when (modifier) {
            StickyModifier.Ctrl -> ctrl = state
            StickyModifier.Alt -> alt = state
        }
    }

    /** The sticky modifiers' bits for a key pressed now; a modifier armed once lets go with it. */
    private fun takeSticky(): Int {
        var bits = 0
        for (modifier in StickyModifier.entries) {
            val state = sticky(modifier)
            if (state != Sticky.Off) bits = bits or modifier.bit
            if (state == Sticky.Once) set(modifier, Sticky.Off)
        }
        return bits
    }

    /**
     * A key event from a hardware keyboard, or one an input method sends (Enter, backspace): an app chord runs, any
     * other key goes to the target with the sticky modifiers. Whether it was taken: a key with no name and no text
     * (Back, the volume) is the system's.
     */
    fun key(event: KeyEvent): Boolean {
        val target = target
        val key = KeyMapping.fromEvent(event)
        return when {
            target == null -> {
                false
            }

            chord(event, target) -> {
                true
            }

            key == null || key.isSystems() -> {
                false
            }

            else -> {
                val sticky =
                    if (key.action == PaneKeyAction.PRESS &&
                        !KeyEvent.isModifierKey(event.keyCode)
                    ) {
                        takeSticky()
                    } else {
                        0
                    }
                target.key(key.copy(mods = (key.mods.toInt() or sticky).toUByte()))
                true
            }
        }
    }

    private fun chord(
        event: KeyEvent,
        target: TerminalState,
    ): Boolean =
        when (event.action) {
            KeyEvent.ACTION_UP -> chordKeys.remove(event.keyCode)
            KeyEvent.ACTION_DOWN -> runChord(event, target)
            else -> false
        }

    // a size step repeats while its chord is held; a move happens once per press; the held chord's key is the app's
    private fun runChord(
        event: KeyEvent,
        target: TerminalState,
    ): Boolean {
        val code = event.keyCode
        val held = code in chordKeys
        val run =
            KeyMapping
                .shortcut(code, KeyMapping.mods(event.metaState))
                ?.takeIf { !held || it in SIZE_STEPS }
                ?.let { shortcuts.action(it, target) }
        run?.invoke()
        if (run != null) chordKeys += code
        return run != null || held
    }

    /**
     * Text the input method committed: line breaks go as Enter, the rest as typed text; with a sticky modifier held,
     * the first character goes as the key that types it, with the modifier.
     */
    fun type(text: String) {
        val target = target ?: return
        var rest = text
        val held = ctrl != Sticky.Off || alt != Sticky.Off
        if (held && rest.isNotEmpty() && !rest.startsWith('\n')) {
            val first = rest.substring(0, rest.offsetByCodePoints(0, 1))
            target.pressKey(KeyMapping.forText(first, takeSticky()))
            rest = rest.substring(first.length)
        }
        val lines = rest.split('\n')
        lines.forEachIndexed { index, line ->
            if (index > 0) press(ENTER)
            if (line.isNotEmpty()) target.type(line)
        }
    }

    /** The input method's composition, drawn at the target's cursor until it commits; empty for none. */
    fun compose(text: String) {
        target?.composing = text
    }

    /** Presses and lets go of the key Ghostty names [name], with the sticky modifiers (the extra keys, Enter). */
    fun press(name: String) {
        val target = target ?: return
        target.pressKey(KeyMapping.named(name, takeSticky()))
    }

    private companion object {
        const val ENTER = "enter"
        val SIZE_STEPS = setOf(TerminalShortcut.Bigger, TerminalShortcut.Smaller, TerminalShortcut.ResetSize)
    }
}

/** A key with no name and no text (Back, the volume): the system's, not the program's. */
private fun PaneKey.isSystems() = key == "unidentified" && text.isEmpty() && !composing

/** Presses [key] and lets it go. */
private fun TerminalState.pressKey(key: PaneKey) {
    key(key)
    key(key.copy(action = PaneKeyAction.RELEASE))
}

/** What [shortcut] does for [target]: the text size's chords always, the moves when the session view set them. */
private fun TerminalShortcuts.action(
    shortcut: TerminalShortcut,
    target: TerminalState,
): (() -> Unit)? =
    when (shortcut) {
        TerminalShortcut.Bigger -> ({ target.stepTextSize(1) })
        TerminalShortcut.Smaller -> ({ target.stepTextSize(-1) })
        TerminalShortcut.ResetSize -> ({ target.textSize = TerminalTextSize })
        TerminalShortcut.NextPane -> nextPane
        TerminalShortcut.PreviousPane -> previousPane
        TerminalShortcut.NextSession -> nextSession
        TerminalShortcut.PreviousSession -> previousSession
    }

/** A [TerminalKeyboard] kept across compositions. */
@Composable
fun rememberTerminalKeyboard(): TerminalKeyboard = remember { TerminalKeyboard() }

/** The text a step of [steps] whole sp bigger (negative: smaller), kept within [TerminalTextSizes]. */
internal fun TerminalState.stepTextSize(steps: Int) {
    textSize = (textSize.value + steps).coerceIn(TerminalTextSizes).sp
}
