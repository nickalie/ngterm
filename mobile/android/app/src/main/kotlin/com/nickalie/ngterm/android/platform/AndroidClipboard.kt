package com.nickalie.ngterm.android.platform

import android.content.ClipboardManager
import android.content.Context
import com.nickalie.ngterm.android.data.ClipboardText

/**
 * The system clipboard's text, read from any thread. Android lets only the app in front (or the keyboard) read it:
 * in the background it reads as holding nothing.
 */
class AndroidClipboard(
    private val context: Context,
) : ClipboardText {
    private val manager = context.getSystemService(ClipboardManager::class.java)

    override fun read(): String? =
        manager
            ?.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(context)
            ?.toString()
}
