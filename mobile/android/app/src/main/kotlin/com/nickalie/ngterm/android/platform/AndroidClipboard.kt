package com.nickalie.ngterm.android.platform

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import com.nickalie.ngterm.android.data.ClipboardSink
import com.nickalie.ngterm.android.data.ClipboardText

/**
 * The system clipboard's text, read from any thread and written from the main one. Android lets only the app in front
 * (or the keyboard) read it: in the background it reads as holding nothing. A write shows Android's own notice of it.
 */
class AndroidClipboard(
    private val context: Context,
) : ClipboardText,
    ClipboardSink {
    private val manager = context.getSystemService(ClipboardManager::class.java)

    override fun read(): String? =
        manager
            ?.primaryClip
            ?.takeIf { it.itemCount > 0 }
            ?.getItemAt(0)
            ?.coerceToText(context)
            ?.toString()

    override fun write(text: String?) {
        val clipboard = manager ?: return
        when {
            text != null -> clipboard.setPrimaryClip(ClipData.newPlainText(LABEL, text))
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.P -> clipboard.clearPrimaryClip()
            else -> clipboard.setPrimaryClip(ClipData.newPlainText(LABEL, ""))
        }
    }

    private companion object {
        // what the system's clipboard notice and history call a program's copy
        const val LABEL = "ngterm"
    }
}
