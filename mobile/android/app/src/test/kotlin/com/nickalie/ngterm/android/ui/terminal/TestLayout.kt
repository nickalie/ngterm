package com.nickalie.ngterm.android.ui.terminal

import android.view.KeyCharacterMap
import android.view.KeyEvent

/**
 * Hardware layouts other than US, as Android's key character maps type them, for the keys the tests press: the JVM's
 * stand-in for Android has only a US map. A key gives its base, shifted and AltGr (`ralt:`) characters; a dead key's
 * carries `KeyCharacterMap.COMBINING_ACCENT`, as `KeyEvent.getUnicodeChar` gives it.
 */
internal class TestLayout(
    private val keys: Map<Int, Triple<Int, Int, Int>>,
) : KeyLayout {
    override fun unicodeChar(
        event: KeyEvent,
        metaState: Int,
    ): Int {
        val letter = event.keyCode - KeyEvent.KEYCODE_A
        val (base, shifted, altGr) =
            keys[event.keyCode]
                ?: if (letter in 0..LAST_LETTER) {
                    Triple('a'.code + letter, 'A'.code + letter, 0)
                } else if (event.keyCode == KeyEvent.KEYCODE_SPACE) {
                    Triple(' '.code, ' '.code, 0)
                } else {
                    return 0
                }
        return when {
            metaState and KeyEvent.META_ALT_RIGHT_ON != 0 && altGr != 0 -> altGr
            metaState and KeyEvent.META_SHIFT_MASK != 0 -> shifted
            else -> base
        }
    }

    companion object {
        private const val LAST_LETTER = 25

        private fun dead(accent: Char) = KeyCharacterMap.COMBINING_ACCENT or accent.code

        private fun key(
            base: Int,
            shifted: Int,
            altGr: Int = 0,
        ) = Triple(base, shifted, altGr)

        /** Android's German layout (`keyboard_layout_german.kcm`), in part. */
        val German =
            TestLayout(
                mapOf(
                    KeyEvent.KEYCODE_Q to key('q'.code, 'Q'.code, '@'.code),
                    KeyEvent.KEYCODE_E to key('e'.code, 'E'.code, '€'.code),
                    KeyEvent.KEYCODE_Z to key('y'.code, 'Y'.code),
                    KeyEvent.KEYCODE_Y to key('z'.code, 'Z'.code),
                    KeyEvent.KEYCODE_GRAVE to key(dead('^'), '°'.code),
                    KeyEvent.KEYCODE_EQUALS to key(dead('´'), dead('`')),
                    KeyEvent.KEYCODE_MINUS to key('ß'.code, '?'.code, '\\'.code),
                    KeyEvent.KEYCODE_SLASH to key('-'.code, '_'.code),
                    KeyEvent.KEYCODE_RIGHT_BRACKET to key('+'.code, '*'.code, '~'.code),
                    KeyEvent.KEYCODE_7 to key('7'.code, '/'.code, '{'.code),
                    KeyEvent.KEYCODE_0 to key('0'.code, '='.code, '}'.code),
                ),
            )

        /** Android's Dvorak layout (`keyboard_layout_english_us_dvorak.kcm`), in part. */
        val Dvorak =
            TestLayout(
                mapOf(
                    KeyEvent.KEYCODE_MINUS to key('['.code, '{'.code),
                    KeyEvent.KEYCODE_EQUALS to key(']'.code, '}'.code),
                    KeyEvent.KEYCODE_LEFT_BRACKET to key('/'.code, '?'.code),
                    KeyEvent.KEYCODE_RIGHT_BRACKET to key('='.code, '+'.code),
                    KeyEvent.KEYCODE_APOSTROPHE to key('-'.code, '_'.code),
                ),
            )
    }
}
