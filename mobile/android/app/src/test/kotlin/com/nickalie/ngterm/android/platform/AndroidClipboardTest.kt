package com.nickalie.ngterm.android.platform

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.net.Uri
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** The terminal's clipboard boundary, including clear on both sides of Android 9. */
@RunWith(AndroidJUnit4::class)
class AndroidClipboardTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val manager get() = context.getSystemService(ClipboardManager::class.java)

    @Test
    fun copyingTextCanBeReadBackAndOnlyTheFirstItemIsPasted() {
        val clipboard = AndroidClipboard(context)
        clipboard.write("λ terminal\ntext")
        assertEquals("ngterm", manager.primaryClipDescription?.label)
        assertEquals("λ terminal\ntext", clipboard.read())
        manager.setPrimaryClip(ClipData.newPlainText("other", "first").apply { addItem(ClipData.Item("second")) })
        assertEquals("first", clipboard.read())
        clipboard.write("")
        assertEquals("", clipboard.read())
    }

    @Test
    fun anEmptyClipboardReadsAsAbsentAndClearingRemovesCopiedText() {
        val clipboard = AndroidClipboard(context)
        manager.clearPrimaryClip()
        assertNull(clipboard.read())
        clipboard.write("private text")
        clipboard.write(null)
        assertNull(manager.primaryClip)
        assertNull(clipboard.read())
    }

    @Test
    @Config(sdk = [26])
    fun clearingOnAndroid8ReplacesTheTextWithAnEmptyClip() {
        val clipboard = AndroidClipboard(context)
        clipboard.write("private text")
        clipboard.write(null)
        assertEquals("", clipboard.read())
        assertEquals("", manager.primaryClip?.getItemAt(0)?.text)
    }

    @Test
    fun aUriClipIsCoercedToText() {
        manager.setPrimaryClip(ClipData.newRawUri("link", Uri.parse("https://example.com/terminal")))
        assertEquals("https://example.com/terminal", AndroidClipboard(context).read())
    }

    @Test
    fun aDeviceWithoutAClipboardReadsNothingAndAcceptsWrites() {
        val absent =
            object : ContextWrapper(context) {
                override fun getSystemService(name: String): Any? =
                    if (name == Context.CLIPBOARD_SERVICE) null else super.getSystemService(name)
            }
        val clipboard = AndroidClipboard(absent)
        clipboard.write("ignored")
        clipboard.write(null)
        assertNull(clipboard.read())
    }
}
