package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyCharacterMap
import android.view.KeyEvent
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneKeyAction

/** What a hardware chord does in the app rather than in the program (desktop's Linux chords, `keymap/linux.rs`). */
enum class TerminalShortcut {
    /** Ctrl+= (or Ctrl++): the text a step bigger. */
    Bigger,

    /** Ctrl+-: the text a step smaller. */
    Smaller,

    /** Ctrl+0: the text back to its default size. */
    ResetSize,

    /** Ctrl+Shift+Right. */
    NextPane,

    /** Ctrl+Shift+Left. */
    PreviousPane,

    /** Ctrl+Shift+Down. */
    NextSession,

    /** Ctrl+Shift+Up. */
    PreviousSession,
}

/**
 * What a hardware keyboard's layout types: the code point [event]'s key types with [metaState] held, as
 * `KeyEvent.getUnicodeChar` gives it from the device's key character map. Tests stand in for a layout (German, Dvorak)
 * the JVM's stand-in for Android has no map of.
 */
fun interface KeyLayout {
    fun unicodeChar(
        event: KeyEvent,
        metaState: Int,
    ): Int

    companion object {
        /** The layout of the keyboard the event came from. */
        val Device = KeyLayout { event, metaState -> event.getUnicodeChar(metaState) }
    }
}

/**
 * The code points a key types, as `KeyEvent.getUnicodeChar` gives them (a dead key's carry
 * `KeyCharacterMap.COMBINING_ACCENT`): [typed] with the held modifiers but Ctrl, Alt and Meta, [base] with none, and
 * [altGr] with Alt kept (0 with no Alt held). Where the layout types a character of its own with AltGr (German
 * AltGr+Q's `@`), that is the key's text and Alt is used up by it.
 */
class KeyChars(
    val typed: Int,
    val base: Int,
    val altGr: Int = 0,
) {
    /** What the key types with Alt held when [altHeld]: AltGr's character where it has one of its own, else [typed]. */
    internal fun chosen(altHeld: Boolean): Int {
        val accentless = altGr and KeyCharacterMap.COMBINING_ACCENT_MASK
        val own = accentless != 0 && !Character.isISOControl(accentless) && altGr != typed
        return if (altHeld && own) altGr else typed
    }
}

/**
 * Android's keys as the pane's (`PaneKey`, ngterm-term's `KeyInput`): the key's position by Ghostty's name, the
 * modifiers as the pane's bits, the text the layout types before Ctrl and Alt, the character the key types with no
 * modifier (the Kitty protocol's key code), and the action.
 *
 * A key code stands for the key's position: Android's key layouts give a hardware key the code of its place on a US
 * keyboard, whatever the characters it types (those come from the key character map, as `text`).
 */
object KeyMapping {
    const val SHIFT = 1
    const val CTRL = 2
    const val ALT = 4
    const val SUPER = 8
    const val CAPS_LOCK = 16
    const val NUM_LOCK = 32

    /** The modifiers a chord is told by: the locks don't count. */
    private const val CHORD_MODS = SHIFT or CTRL or ALT or SUPER

    /** The meta state that changes what a key types into what the program reads as a chord. */
    private const val CHORD_META = KeyEvent.META_CTRL_MASK or KeyEvent.META_ALT_MASK or KeyEvent.META_META_MASK

    /** The meta state left out when asking for AltGr's character: Alt stays. */
    private const val ALT_GR_META = KeyEvent.META_CTRL_MASK or KeyEvent.META_META_MASK

    private const val FIRST_PRINTABLE = 0x20
    private const val DELETE = 0x7F
    private const val LAST_C1 = 0x9F

    // built on first use: the tables it takes in are declared below
    private val named: Map<Int, String> by lazy {
        buildMap {
            for ((index, letter) in ('a'..'z').withIndex()) put(KeyEvent.KEYCODE_A + index, "key_$letter")
            for ((index, digit) in ('0'..'9').withIndex()) {
                put(KeyEvent.KEYCODE_0 + index, "digit_$digit")
                put(KeyEvent.KEYCODE_NUMPAD_0 + index, "numpad_$digit")
            }
            for (code in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12) put(code, "f${code - KeyEvent.KEYCODE_F1 + 1}")
            putAll(KEYS)
        }
    }

    /** Ghostty's name of the key at [keyCode]'s position; `unidentified` for a key it has no name for. */
    fun physicalKey(keyCode: Int): String = named[keyCode] ?: UNIDENTIFIED

    /** The modifiers of [metaState] as the pane's bits: shift 1, ctrl 2, alt 4, super 8, caps lock 16, num lock 32. */
    fun mods(metaState: Int): Int {
        var bits = 0
        if (metaState and KeyEvent.META_SHIFT_MASK != 0) bits = bits or SHIFT
        if (metaState and KeyEvent.META_CTRL_MASK != 0) bits = bits or CTRL
        if (metaState and KeyEvent.META_ALT_MASK != 0) bits = bits or ALT
        if (metaState and KeyEvent.META_META_MASK != 0) bits = bits or SUPER
        if (metaState and KeyEvent.META_CAPS_LOCK_ON != 0) bits = bits or CAPS_LOCK
        if (metaState and KeyEvent.META_NUM_LOCK_ON != 0) bits = bits or NUM_LOCK
        return bits
    }

    /**
     * [event] as the pane's key, its characters by [layout]; `null` for what is not one key's press, repeat or release
     * (a run of characters an input method sends as one event).
     */
    fun fromEvent(
        event: KeyEvent,
        layout: KeyLayout = KeyLayout.Device,
    ): PaneKey? {
        val action =
            when (event.action) {
                KeyEvent.ACTION_DOWN -> if (event.repeatCount > 0) PaneKeyAction.REPEAT else PaneKeyAction.PRESS
                KeyEvent.ACTION_UP -> PaneKeyAction.RELEASE
                else -> return null
            }
        return key(event.keyCode, action, event.metaState, chars(event, layout))
    }

    /**
     * The accent [event]'s key holds for the next key when it is a dead key on [layout] (the spacing accent, as
     * `KeyCharacterMap.getDeadChar` takes it); 0 for any other key.
     */
    fun deadAccent(
        event: KeyEvent,
        layout: KeyLayout = KeyLayout.Device,
    ): Int {
        val chosen = chars(event, layout).chosen(mods(event.metaState) and ALT != 0)
        val dead = chosen and KeyCharacterMap.COMBINING_ACCENT != 0
        return if (dead) chosen and KeyCharacterMap.COMBINING_ACCENT_MASK else 0
    }

    // what [event]'s key types on [layout]; with Alt kept only while Alt is held (AltGr's are the layout's `ralt:`)
    private fun chars(
        event: KeyEvent,
        layout: KeyLayout,
    ): KeyChars {
        val meta = event.metaState
        val altHeld = meta and KeyEvent.META_ALT_MASK != 0
        return KeyChars(
            typed = layout.unicodeChar(event, meta and CHORD_META.inv()),
            base = layout.unicodeChar(event, 0),
            altGr = if (altHeld) layout.unicodeChar(event, meta and ALT_GR_META.inv()) else 0,
        )
    }

    /** The key at [keyCode] with [metaState] held, typing [chars]. */
    fun key(
        keyCode: Int,
        action: PaneKeyAction,
        metaState: Int,
        chars: KeyChars,
    ): PaneKey {
        val mods = mods(metaState)
        val chosen = chars.chosen(mods and ALT != 0)
        val composing = chosen and KeyCharacterMap.COMBINING_ACCENT != 0
        val text = if (composing) "" else printable(chosen)
        val usedUp = if (chosen == chars.typed) SHIFT else SHIFT or ALT
        return PaneKey(
            action = action,
            key = physicalKey(keyCode),
            mods = mods.toUByte(),
            consumed = (if (text.isNotEmpty()) mods and usedUp else 0).toUByte(),
            composing = composing,
            text = text,
            unshifted = printable(chars.base and KeyCharacterMap.COMBINING_ACCENT_MASK).ifEmpty { null },
        )
    }

    /**
     * A press of the key that types [text] (one character) on a US keyboard, with [mods] held besides the shift the
     * character needs: how a character the input method commits takes the extra-keys row's sticky Ctrl and Alt. A
     * character no US key types is an unidentified key with its text.
     */
    fun forText(
        text: String,
        mods: Int,
    ): PaneKey {
        val position = text.singleOrNull()?.let(US_KEYS::get)
        val shifted = position != null && position.unshifted != text.single()
        val shift = if (shifted) SHIFT else 0
        return PaneKey(
            action = PaneKeyAction.PRESS,
            key = position?.name ?: UNIDENTIFIED,
            mods = (mods or shift).toUByte(),
            consumed = shift.toUByte(),
            composing = false,
            text = text,
            unshifted = position?.unshifted?.toString(),
        )
    }

    /** A press of the key Ghostty names [name] with [mods] held, typing nothing (Esc, Tab, the arrows, ...). */
    fun named(
        name: String,
        mods: Int,
        action: PaneKeyAction = PaneKeyAction.PRESS,
    ): PaneKey = PaneKey(action, name, mods.toUByte(), 0u, false, "", null)

    /**
     * What [keyCode] pressed with [mods] and typing [text] (its characters before Ctrl and Alt) does in the app; `null`
     * for a key that is the program's. The text size's chords go by the character the layout types (desktop's
     * `ctrl+=`, `ctrl+-`, `ctrl+0`), so they move with the layout, the numpad's by its keys; the moves by the arrows.
     */
    fun shortcut(
        keyCode: Int,
        mods: Int,
        text: String,
    ): TerminalShortcut? {
        val size = NUMPAD_CHORDS[keyCode] ?: SIZE_CHORDS[text]
        return when (mods and CHORD_MODS) {
            CTRL -> size
            CTRL or SHIFT -> NAVIGATION_CHORDS[keyCode] ?: size?.takeIf { it == TerminalShortcut.Bigger }
            else -> null
        }
    }

    private fun printable(codePoint: Int): String =
        when {
            codePoint < FIRST_PRINTABLE || codePoint in DELETE..LAST_C1 -> ""
            else -> String(Character.toChars(codePoint))
        }

    private const val UNIDENTIFIED = "unidentified"

    private val SIZE_CHORDS =
        mapOf(
            "=" to TerminalShortcut.Bigger,
            "+" to TerminalShortcut.Bigger,
            "-" to TerminalShortcut.Smaller,
            "0" to TerminalShortcut.ResetSize,
        )

    private val NUMPAD_CHORDS =
        mapOf(
            KeyEvent.KEYCODE_NUMPAD_ADD to TerminalShortcut.Bigger,
            KeyEvent.KEYCODE_NUMPAD_SUBTRACT to TerminalShortcut.Smaller,
            KeyEvent.KEYCODE_NUMPAD_0 to TerminalShortcut.ResetSize,
        )

    private val NAVIGATION_CHORDS =
        mapOf(
            KeyEvent.KEYCODE_DPAD_RIGHT to TerminalShortcut.NextPane,
            KeyEvent.KEYCODE_DPAD_LEFT to TerminalShortcut.PreviousPane,
            KeyEvent.KEYCODE_DPAD_DOWN to TerminalShortcut.NextSession,
            KeyEvent.KEYCODE_DPAD_UP to TerminalShortcut.PreviousSession,
        )

    /** The keys past the letters, digits, numpad digits and F1-F12, by Ghostty's name. */
    private val KEYS =
        mapOf(
            KeyEvent.KEYCODE_GRAVE to "backquote",
            KeyEvent.KEYCODE_BACKSLASH to "backslash",
            KeyEvent.KEYCODE_LEFT_BRACKET to "bracket_left",
            KeyEvent.KEYCODE_RIGHT_BRACKET to "bracket_right",
            KeyEvent.KEYCODE_COMMA to "comma",
            KeyEvent.KEYCODE_EQUALS to "equal",
            KeyEvent.KEYCODE_MINUS to "minus",
            KeyEvent.KEYCODE_PERIOD to "period",
            KeyEvent.KEYCODE_APOSTROPHE to "quote",
            KeyEvent.KEYCODE_SEMICOLON to "semicolon",
            KeyEvent.KEYCODE_SLASH to "slash",
            KeyEvent.KEYCODE_YEN to "intl_yen",
            KeyEvent.KEYCODE_RO to "intl_ro",
            KeyEvent.KEYCODE_ALT_LEFT to "alt_left",
            KeyEvent.KEYCODE_ALT_RIGHT to "alt_right",
            KeyEvent.KEYCODE_DEL to "backspace",
            KeyEvent.KEYCODE_CAPS_LOCK to "caps_lock",
            KeyEvent.KEYCODE_MENU to "context_menu",
            KeyEvent.KEYCODE_CTRL_LEFT to "control_left",
            KeyEvent.KEYCODE_CTRL_RIGHT to "control_right",
            KeyEvent.KEYCODE_ENTER to "enter",
            KeyEvent.KEYCODE_META_LEFT to "meta_left",
            KeyEvent.KEYCODE_META_RIGHT to "meta_right",
            KeyEvent.KEYCODE_SHIFT_LEFT to "shift_left",
            KeyEvent.KEYCODE_SHIFT_RIGHT to "shift_right",
            KeyEvent.KEYCODE_SPACE to "space",
            KeyEvent.KEYCODE_TAB to "tab",
            KeyEvent.KEYCODE_HENKAN to "convert",
            KeyEvent.KEYCODE_KATAKANA_HIRAGANA to "kana_mode",
            KeyEvent.KEYCODE_MUHENKAN to "non_convert",
            KeyEvent.KEYCODE_FORWARD_DEL to "delete",
            KeyEvent.KEYCODE_MOVE_END to "end",
            KeyEvent.KEYCODE_HELP to "help",
            KeyEvent.KEYCODE_MOVE_HOME to "home",
            KeyEvent.KEYCODE_INSERT to "insert",
            KeyEvent.KEYCODE_PAGE_DOWN to "page_down",
            KeyEvent.KEYCODE_PAGE_UP to "page_up",
            KeyEvent.KEYCODE_DPAD_DOWN to "arrow_down",
            KeyEvent.KEYCODE_DPAD_LEFT to "arrow_left",
            KeyEvent.KEYCODE_DPAD_RIGHT to "arrow_right",
            KeyEvent.KEYCODE_DPAD_UP to "arrow_up",
            KeyEvent.KEYCODE_NUM_LOCK to "num_lock",
            KeyEvent.KEYCODE_NUMPAD_ADD to "numpad_add",
            KeyEvent.KEYCODE_NUMPAD_COMMA to "numpad_comma",
            KeyEvent.KEYCODE_NUMPAD_DOT to "numpad_decimal",
            KeyEvent.KEYCODE_NUMPAD_DIVIDE to "numpad_divide",
            KeyEvent.KEYCODE_NUMPAD_ENTER to "numpad_enter",
            KeyEvent.KEYCODE_NUMPAD_EQUALS to "numpad_equal",
            KeyEvent.KEYCODE_NUMPAD_MULTIPLY to "numpad_multiply",
            KeyEvent.KEYCODE_NUMPAD_LEFT_PAREN to "numpad_paren_left",
            KeyEvent.KEYCODE_NUMPAD_RIGHT_PAREN to "numpad_paren_right",
            KeyEvent.KEYCODE_NUMPAD_SUBTRACT to "numpad_subtract",
            KeyEvent.KEYCODE_ESCAPE to "escape",
            KeyEvent.KEYCODE_SYSRQ to "print_screen",
            KeyEvent.KEYCODE_SCROLL_LOCK to "scroll_lock",
            KeyEvent.KEYCODE_BREAK to "pause",
            KeyEvent.KEYCODE_FUNCTION to "fn",
            KeyEvent.KEYCODE_COPY to "copy",
            KeyEvent.KEYCODE_CUT to "cut",
            KeyEvent.KEYCODE_PASTE to "paste",
        )

    /** A US key: its name and the character it types unshifted. */
    private class UsKey(
        val name: String,
        val unshifted: Char,
    )

    /** Every character a US keyboard types, by the key that types it. */
    private val US_KEYS: Map<Char, UsKey> =
        buildMap {
            for (letter in 'a'..'z') {
                val key = UsKey("key_$letter", letter)
                put(letter, key)
                put(letter.uppercaseChar(), key)
            }
            val unshifted = "0123456789`-=[]\\;',./ "
            val shifted = ")!@#$%^&*(~_+{}|:\"<>? "
            val names =
                ('0'..'9').map { "digit_$it" } +
                    listOf(
                        "backquote",
                        "minus",
                        "equal",
                        "bracket_left",
                        "bracket_right",
                        "backslash",
                        "semicolon",
                        "quote",
                        "comma",
                        "period",
                        "slash",
                        "space",
                    )
            names.forEachIndexed { index, name ->
                val key = UsKey(name, unshifted[index])
                put(unshifted[index], key)
                put(shifted[index], key)
            }
        }
}
