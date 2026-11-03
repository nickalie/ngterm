package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyCharacterMap
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
 * An input method's composition in progress: a key from elsewhere (the extra-keys row, a hardware key) commits it
 * first, so the program gets the text before the key, in the order it was typed.
 */
internal fun interface PendingComposition {
    fun commit()
}

/**
 * The keyboard's side of the terminal: what the soft keyboard commits and composes, the extra-keys row and a
 * hardware keyboard's keys go to the [target] pane (the terminal that last took focus), with the row's sticky Ctrl and
 * Alt held for the next key; a hardware keyboard's app chords ([TerminalShortcut]) change the target's text size or
 * run [shortcuts], and its dead keys hold their accent at the cursor for the next key. One keyboard serves every
 * terminal on screen, so the sticky modifiers stay as the focus moves. [layout] is what the hardware keys type (the
 * device's, or a test's). Used on the main thread.
 */
@Stable
class TerminalKeyboard(
    shortcuts: TerminalShortcuts = TerminalShortcuts(),
    private val layout: KeyLayout = KeyLayout.Device,
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

    private val chords = HeldChords()
    private val pending = PendingInput(layout)

    /** The input method's composition waiting to commit, which a key from elsewhere commits first. */
    internal var composition: PendingComposition?
        get() = pending.composition
        set(value) {
            pending.composition = value
        }

    /** Where [modifier] stands. */
    fun sticky(modifier: StickyModifier): Sticky =
        when (modifier) {
            StickyModifier.Ctrl -> ctrl
            StickyModifier.Alt -> alt
        }

    /**
     * A tap on [modifier]'s key: off to once, once to locked, locked to off. What waits (the input method's
     * composition, a held accent) was typed before the tap, so it goes first, under the modifiers as they stood.
     * Returns where it stands now.
     */
    fun toggle(modifier: StickyModifier): Sticky {
        target?.let(pending::commitBefore) ?: pending.commitComposition()
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
     * A key event from a hardware keyboard: an app chord runs, any other key goes to the target with the sticky
     * modifiers, after the input method's composition. Whether it was taken: a key with no name and no text (Back,
     * the volume) is the system's.
     */
    fun key(event: KeyEvent): Boolean = key(event, target, commitFirst = true)

    /**
     * [event] for [to]: as [key], from the input method's connection to [to] when [commitFirst] is false (it has
     * committed its own composition).
     */
    internal fun key(
        event: KeyEvent,
        to: TerminalState?,
        commitFirst: Boolean,
    ): Boolean {
        val target = to ?: return false
        val key = KeyMapping.fromEvent(event, layout)
        return when {
            chords.take(event, key?.text.orEmpty(), target, shortcuts) -> {
                true
            }

            key == null || key.isSystems() -> {
                false
            }

            key.action != PaneKeyAction.PRESS || KeyEvent.isModifierKey(event.keyCode) -> {
                target.key(key)
                true
            }

            else -> {
                if (commitFirst) pending.commitComposition()
                pending.pressed(event, key, target, ::takeSticky)
                true
            }
        }
    }

    /** Text from the extra-keys row: as [type] for the target, after the input method's composition. */
    fun type(text: String) {
        val target = target ?: return
        pending.commitBefore(target)
        type(text, target)
    }

    /**
     * Text the input method committed to [to]: line breaks go as Enter, the rest as typed text; with a sticky
     * modifier held, the first character goes as the key that types it, with the modifier.
     */
    internal fun type(
        text: String,
        to: TerminalState,
    ) {
        var rest = text
        val held = ctrl != Sticky.Off || alt != Sticky.Off
        if (held && rest.isNotEmpty() && !rest.startsWith('\n')) {
            val first = rest.substring(0, rest.offsetByCodePoints(0, 1))
            to.pressKey(KeyMapping.forText(first, takeSticky()))
            rest = rest.substring(first.length)
        }
        val lines = rest.split('\n')
        lines.forEachIndexed { index, line ->
            if (index > 0) press(ENTER, to)
            if (line.isNotEmpty()) to.type(line)
        }
    }

    /**
     * Presses and lets go of the key Ghostty names [name] on the target, with the sticky modifiers (the extra keys),
     * after the input method's composition.
     */
    fun press(name: String) {
        val target = target ?: return
        pending.commitBefore(target)
        press(name, target)
    }

    /** Presses and lets go of the key Ghostty names [name] on [to], with the sticky modifiers. */
    internal fun press(
        name: String,
        to: TerminalState,
    ) {
        to.pressKey(KeyMapping.named(name, takeSticky()))
    }

    private companion object {
        const val ENTER = "enter"
    }
}

/** A hardware keyboard's app chords ([TerminalShortcut]) as they are pressed, held and let go. */
private class HeldChords {
    // keys whose press ran a chord: their repeats and release are the chord's too, not the program's
    private val keys = mutableSetOf<Int>()

    /**
     * Whether [event] (its key typing [text]) is a chord's, running its action on [target]: a size step repeats while
     * its chord is held, a move happens once per press, and the held chord's key is the app's until it lets go.
     */
    fun take(
        event: KeyEvent,
        text: String,
        target: TerminalState,
        shortcuts: TerminalShortcuts,
    ): Boolean {
        val code = event.keyCode
        val held = code in keys
        val run =
            KeyMapping
                .shortcut(code, KeyMapping.mods(event.metaState), text)
                ?.takeIf { event.action == KeyEvent.ACTION_DOWN && (!held || it in SIZE_STEPS) }
                ?.let { shortcuts.action(it, target) }
        run?.invoke()
        if (run != null) keys += code
        return when (event.action) {
            KeyEvent.ACTION_UP -> keys.remove(code)
            KeyEvent.ACTION_DOWN -> run != null || held
            else -> false
        }
    }

    private companion object {
        val SIZE_STEPS = setOf(TerminalShortcut.Bigger, TerminalShortcut.Smaller, TerminalShortcut.ResetSize)
    }
}

/**
 * What waits for the next key: the input method's [composition], and a hardware dead key's accent, shown at the
 * cursor of the terminal it was typed into. The next hardware key combines with the accent ([pressed]); a key from
 * elsewhere commits both first ([commitBefore]).
 */
private class PendingInput(
    private val layout: KeyLayout,
) {
    var composition: PendingComposition? = null

    private var accent = 0
    private var accentTarget: TerminalState? = null

    /**
     * A hardware key's press for [target], [sticky] giving the sticky modifiers: a dead key holds its accent (a held
     * one goes first, unless it is the same: twice types it once); another key combines with a held accent, or types
     * it before its own when the two don't combine or the key is a chord.
     */
    fun pressed(
        event: KeyEvent,
        key: PaneKey,
        target: TerminalState,
        sticky: () -> Int,
    ) {
        val held = takeAccent(target)
        val dead = KeyMapping.deadAccent(event, layout)
        if (dead != 0) {
            if (held != 0) target.type(accentText(held))
            if (held != dead) holdAccent(dead, target)
            target.key(key)
            return
        }
        val mods = key.mods.toInt() or sticky()
        val unconsumed = mods and key.consumed.toInt().inv() and CHORD_BITS
        val combined =
            key.text
                .takeIf { held != 0 && unconsumed == 0 && it.codePointCount(0, it.length) == 1 }
                ?.let { KeyCharacterMap.getDeadChar(held, it.codePointAt(0)) } ?: 0
        if (held != 0 && combined == 0) target.type(accentText(held))
        val text = if (combined != 0) accentText(combined) else key.text
        target.key(key.copy(mods = mods.toUByte(), text = text))
    }

    /** A key from elsewhere than the hardware keyboard is coming to [target]: the composition and accent go first. */
    fun commitBefore(target: TerminalState) {
        commitComposition()
        val held = takeAccent(target)
        if (held != 0) target.type(accentText(held))
    }

    /** Commits the input method's composition, if one waits. */
    fun commitComposition() {
        val waiting = composition ?: return
        composition = null
        waiting.commit()
    }

    private fun holdAccent(
        accent: Int,
        target: TerminalState,
    ) {
        this.accent = accent
        accentTarget = target
        target.composing = accentText(accent)
    }

    // the accent held for [target] (0 for none), let go of; one held for another terminal is dropped
    private fun takeAccent(target: TerminalState): Int {
        val held = accent
        val heldFor = accentTarget ?: return 0
        accent = 0
        accentTarget = null
        if (heldFor.composing == accentText(held)) heldFor.composing = ""
        return if (heldFor === target) held else 0
    }

    private companion object {
        // the modifiers that make a key a chord, which a held accent doesn't combine with
        const val CHORD_BITS = KeyMapping.CTRL or KeyMapping.ALT or KeyMapping.SUPER
    }
}

/** [codePoint] as a string. */
private fun accentText(codePoint: Int) = String(Character.toChars(codePoint))

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
