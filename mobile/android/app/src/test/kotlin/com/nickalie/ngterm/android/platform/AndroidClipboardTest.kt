package com.nickalie.ngterm.android.platform

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.ContextWrapper
import android.net.Uri
import android.os.TransactionTooLargeException
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.RED_PNG
import com.nickalie.ngterm.android.data.key
import com.nickalie.ngterm.android.data.part
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardWriteStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.Implementation
import org.robolectric.annotation.Implements
import org.robolectric.shadows.ShadowClipboardManager

/** The terminal's clipboard boundary per MIME type: text, HTML and images in, the same out, and clears. */
@RunWith(AndroidJUnit4::class)
class AndroidClipboardTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val manager get() = context.getSystemService(ClipboardManager::class.java)
    private val clipboard = AndroidClipboard(context)

    private fun text(clipboard: AndroidClipboard) =
        clipboard
            .read(listOf("text/plain"))
            .singleOrNull()
            ?.data
            ?.decodeToString()

    private fun keys(parts: List<ClipboardPart>) = parts.map { it.key() }

    @Test
    fun copyingTextCanBeReadBackAndOnlyTheFirstItemIsPasted() {
        assertEquals(ClipboardWriteStatus.DONE, clipboard.write(listOf(part("text/plain", "λ terminal\ntext"))))
        assertEquals("ngterm", manager.primaryClipDescription?.label)
        assertEquals("λ terminal\ntext", text(clipboard))
        assertEquals(listOf("text/plain"), clipboard.types())
        manager.setPrimaryClip(ClipData.newPlainText("other", "first").apply { addItem(ClipData.Item("second")) })
        assertEquals("first", text(clipboard))
        clipboard.write(listOf(part("text/plain", "")))
        assertEquals("", text(clipboard))
    }

    @Test
    fun textHtmlAndAnImageAreKeptAndReadBackByTypeInTheRequestedOrder() {
        val parts =
            listOf(
                part("text/plain", "Ghostty multipart clipboard"),
                part("text/html", "<b>Ghostty</b> multipart clipboard"),
                ClipboardPart("image/png", RED_PNG),
                ClipboardPart("application/octet-stream", ByteArray(2048) { it.toByte() }),
            )
        assertEquals(ClipboardWriteStatus.DONE, clipboard.write(parts))
        val description = manager.primaryClipDescription!!
        assertEquals(
            listOf("text/plain", "text/html", "image/png"),
            (0 until description.mimeTypeCount).map(description::getMimeType),
        )
        assertEquals(listOf("text/plain", "text/html", "image/png"), clipboard.types())
        assertEquals(
            "each type it keeps, in the order asked; types it doesn't keep are skipped",
            keys(listOf(parts[2], parts[0], parts[1])),
            keys(clipboard.read(listOf("image/png", "application/octet-stream", "text/plain", "text/html"))),
        )
        val uri = manager.primaryClip!!.getItemAt(0).uri
        assertEquals("content", uri.scheme)
        assertTrue(
            context.contentResolver
                .openInputStream(uri)!!
                .use { it.readBytes() }
                .contentEquals(RED_PNG),
        )
    }

    @Test
    fun anImageAloneIsAWriteNotAClearAndTheNextWriteReplacesItsFile() {
        assertEquals(ClipboardWriteStatus.DONE, clipboard.write(listOf(ClipboardPart("image/png", RED_PNG))))
        assertEquals(listOf("image/png"), clipboard.types())
        assertNull("an image has no text", text(clipboard))
        assertEquals(keys(listOf(ClipboardPart("image/png", RED_PNG))), keys(clipboard.read(listOf("image/png"))))
        val dir = java.io.File(context.cacheDir, "clipboard")
        assertEquals(1, dir.listFiles()!!.size)
        clipboard.write(listOf(ClipboardPart("image/jpeg", byteArrayOf(1, 2))))
        assertEquals("only the last image is kept", 1, dir.listFiles()!!.size)
        clipboard.write(emptyList())
        assertEquals(0, dir.listFiles()!!.size)
    }

    @Test
    fun anImageThatCantBeStoredFailsAndTheLastImageStaysReadable() {
        clipboard.write(listOf(ClipboardPart("image/png", RED_PNG)))
        val dir = java.io.File(context.cacheDir, "clipboard")
        dir.setWritable(false)
        val status =
            try {
                clipboard.write(listOf(ClipboardPart("image/jpeg", byteArrayOf(1, 2))))
            } finally {
                dir.setWritable(true)
            }
        assertEquals(ClipboardWriteStatus.FAILED, status)
        assertEquals(
            "the clipboard still holds the last image, and its file is still there",
            keys(listOf(ClipboardPart("image/png", RED_PNG))),
            keys(clipboard.read(listOf("image/png"))),
        )
        assertEquals(1, dir.listFiles()!!.size)
    }

    @Test
    @Config(shadows = [FailingClipboardManager::class])
    fun aClipTheSystemRefusesFailsTheWriteAndKeepsTheLastImage() {
        FailingClipboardManager.failing = false
        clipboard.write(listOf(ClipboardPart("image/png", RED_PNG)))
        FailingClipboardManager.failing = true
        val status =
            try {
                clipboard.write(listOf(part("text/plain", "too large"), ClipboardPart("image/jpeg", byteArrayOf(1, 2))))
            } finally {
                FailingClipboardManager.failing = false
            }
        assertEquals("a binder failure is a failed write, not a crash", ClipboardWriteStatus.FAILED, status)
        val dir = java.io.File(context.cacheDir, "clipboard")
        assertEquals("the refused clip's image is removed, the last one kept", 1, dir.listFiles()!!.size)
        assertEquals(
            keys(listOf(ClipboardPart("image/png", RED_PNG))),
            keys(clipboard.read(listOf("image/png"))),
        )
        FailingClipboardManager.failing = true
        try {
            assertEquals(ClipboardWriteStatus.FAILED, clipboard.write(emptyList()))
        } finally {
            FailingClipboardManager.failing = false
        }
        assertEquals("a refused clear keeps the image too", 1, dir.listFiles()!!.size)
    }

    @Test
    fun htmlAloneCarriesItsOwnTextAsAndroidRequires() {
        clipboard.write(listOf(part("text/html", "<p>rich <b>text</b></p>")))
        assertEquals("rich text", text(clipboard)?.trim())
        assertEquals(
            "<p>rich <b>text</b></p>",
            clipboard
                .read(listOf("text/html"))
                .single()
                .data
                .decodeToString(),
        )
    }

    @Test
    fun aValueWithNoTypeAndroidKeepsIsUnsupportedAndLeavesTheClipboard() {
        clipboard.write(listOf(part("text/plain", "kept")))
        assertEquals(
            ClipboardWriteStatus.UNSUPPORTED,
            clipboard.write(listOf(ClipboardPart("application/octet-stream", byteArrayOf(0, 1)))),
        )
        assertEquals("kept", text(clipboard))
    }

    @Test
    fun anEmptyClipboardReadsAsAbsentAndClearingRemovesCopiedText() {
        manager.clearPrimaryClip()
        assertEquals(emptyList<String>(), clipboard.types())
        assertEquals(emptyList<ClipboardPart>(), clipboard.read(listOf("text/plain")))
        clipboard.write(listOf(part("text/plain", "private text")))
        assertEquals(ClipboardWriteStatus.DONE, clipboard.write(emptyList()))
        assertNull(manager.primaryClip)
        assertNull(text(clipboard))
    }

    @Test
    @Config(sdk = [26])
    fun clearingOnAndroid8ReplacesTheTextWithAnEmptyClip() {
        clipboard.write(listOf(part("text/plain", "private text")))
        clipboard.write(emptyList())
        assertEquals("", text(clipboard))
        assertEquals("", manager.primaryClip?.getItemAt(0)?.text)
    }

    @Test
    fun aLinkClipIsReadAsText() {
        manager.setPrimaryClip(ClipData.newRawUri("link", Uri.parse("https://example.com/terminal")))
        assertEquals("https://example.com/terminal", text(clipboard))
    }

    @Test
    fun anotherAppsHtmlIsListedAndRead() {
        manager.setPrimaryClip(ClipData.newHtmlText("browser", "hello", "<i>hello</i>"))
        assertEquals(listOf(ClipDescription.MIMETYPE_TEXT_PLAIN, "text/html"), clipboard.types())
        assertEquals(
            "<i>hello</i>",
            clipboard
                .read(listOf("text/html"))
                .single()
                .data
                .decodeToString(),
        )
    }

    @Test
    fun anotherAppsTypesAreListedOnceAndAnUnreadableUriIsSkipped() {
        val description = ClipDescription("other", arrayOf("text/plain", "image/*", "application/pdf"))
        val item = ClipData.Item("caption", null, null, Uri.parse("content://elsewhere.invalid/doc"))
        manager.setPrimaryClip(ClipData(description, item))
        assertEquals(listOf("text/plain", "application/pdf"), clipboard.types())
        assertEquals(
            "the URI's own type can't be learned, so it serves nothing",
            emptyList<ClipboardPart>(),
            clipboard.read(listOf("application/pdf", "image/png")),
        )
        assertEquals("caption", text(clipboard))
    }

    @Test
    fun anImageTypeWithNoSubtypeIsNotKeptAndOnlyTheFirstImageIs() {
        assertEquals(ClipboardWriteStatus.UNSUPPORTED, clipboard.write(listOf(ClipboardPart("image/", RED_PNG))))
        clipboard.write(listOf(ClipboardPart("image/png", RED_PNG), ClipboardPart("image/jpeg", byteArrayOf(1))))
        assertEquals(listOf("image/png"), clipboard.types())
        assertEquals(emptyList<ClipboardPart>(), clipboard.read(listOf("image/jpeg")))
    }

    @Test
    fun aDeviceWithoutAClipboardReadsNothingAndRefusesWrites() {
        val absent =
            object : ContextWrapper(context) {
                override fun getSystemService(name: String): Any? =
                    if (name == Context.CLIPBOARD_SERVICE) null else super.getSystemService(name)
            }
        val clipboard = AndroidClipboard(absent)
        assertEquals(ClipboardWriteStatus.UNSUPPORTED, clipboard.write(listOf(part("text/plain", "ignored"))))
        assertEquals(ClipboardWriteStatus.UNSUPPORTED, clipboard.write(emptyList()))
        assertEquals(emptyList<ClipboardPart>(), clipboard.read(listOf("text/plain")))
        assertEquals(emptyList<String>(), clipboard.types())
    }
}

/**
 * Android's clipboard that refuses clips while [failing], as the system clipboard does when a clip outgrows the binder
 * transaction: `RemoteException.rethrowFromSystemServer` throws a plain RuntimeException.
 */
@Implements(ClipboardManager::class)
@Suppress("TooGenericExceptionThrown") // the very exception the system clipboard throws
class FailingClipboardManager : ShadowClipboardManager() {
    @Implementation
    override fun setPrimaryClip(clip: ClipData) {
        if (failing) throw RuntimeException(TransactionTooLargeException("data parcel size 2097152 bytes"))
        super.setPrimaryClip(clip)
    }

    @Implementation
    override fun clearPrimaryClip() {
        if (failing) throw RuntimeException(TransactionTooLargeException())
        super.clearPrimaryClip()
    }

    companion object {
        var failing = false
    }
}
