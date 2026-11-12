package com.nickalie.ngterm.android.platform

import android.content.ClipboardManager
import android.util.Base64
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardWriteStatus
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A program's clipboard values through the device's own clipboard: Android lets only the app in front read it, so the
 * app's activity is up, and an image goes out and comes back through the app's content URI, as another app reads it.
 */
@RunWith(AndroidJUnit4::class)
class AndroidClipboardDeviceTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private val clipboard get() = AndroidClipboard(compose.activity)

    private fun <T> onMain(block: () -> T): T {
        compose.waitForIdle()
        var result: Result<T>? = null
        compose.runOnUiThread { result = runCatching(block) }
        return result!!.getOrThrow()
    }

    private fun keys(parts: List<ClipboardPart>) = parts.map { it.mime to it.data.toList() }

    @Test
    fun theMultipartValueRoundTripsByTypeAndItsImageIsReadableThroughItsUri() {
        val text = ClipboardPart("text/plain", "Ghostty multipart clipboard".toByteArray())
        val html = ClipboardPart("text/html", "<b>Ghostty</b> multipart clipboard".toByteArray())
        val png = ClipboardPart("image/png", RED_PNG)
        val binary = ClipboardPart("application/octet-stream", ByteArray(BINARY) { it.toByte() })
        assertEquals(ClipboardWriteStatus.DONE, onMain { clipboard.write(listOf(text, html, png, binary)) })
        assertEquals(listOf("text/plain", "text/html", "image/png"), onMain { clipboard.types() })
        assertEquals(
            keys(listOf(png, text, html)),
            keys(onMain { clipboard.read(listOf("image/png", "application/octet-stream", "text/plain", "text/html")) }),
        )
        val uri =
            onMain {
                compose.activity
                    .getSystemService(ClipboardManager::class.java)
                    .primaryClip
                    ?.getItemAt(0)
                    ?.uri
            }
        assertNotNull("the image is a content URI", uri)
        assertEquals("image/png", compose.activity.contentResolver.getType(uri!!))
        val bytes =
            compose.activity.contentResolver
                .openInputStream(uri)!!
                .use { it.readBytes() }
        assertArrayEquals(RED_PNG, bytes)
    }

    @Test
    fun anImageAloneStaysAnImageAndOnlyAnEmptyWriteClears() {
        val png = ClipboardPart("image/png", RED_PNG)
        assertEquals(ClipboardWriteStatus.DONE, onMain { clipboard.write(listOf(png)) })
        assertEquals(listOf("image/png"), onMain { clipboard.types() })
        assertEquals(emptyList<ClipboardPart>(), onMain { clipboard.read(listOf("text/plain")) })
        assertEquals(keys(listOf(png)), keys(onMain { clipboard.read(listOf("image/png")) }))
        assertEquals(ClipboardWriteStatus.DONE, onMain { clipboard.write(emptyList()) })
        assertEquals(emptyList<String>(), onMain { clipboard.types() })
    }

    private companion object {
        const val BINARY = 2048

        // Ghostty's clipboard-multipart.py image: a 2x2 red PNG, 73 bytes
        val RED_PNG: ByteArray =
            Base64.decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167WwAAAABJRU5ErkJggg==",
                Base64.DEFAULT,
            )
    }
}
