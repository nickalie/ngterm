package com.nickalie.ngterm.android.platform

import android.app.UiAutomation
import android.content.ClipboardManager
import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Base64
import android.view.accessibility.AccessibilityNodeInfo
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardWriteStatus
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A program's clipboard values through the device's own clipboard: Android lets only the app whose window has the
 * focus read it, so the app's activity is up and each call waits for that focus, and an image goes out and comes back
 * through the app's content URI, as another app reads it.
 */
@RunWith(AndroidJUnit4::class)
class AndroidClipboardDeviceTest {
    @get:Rule
    val compose = createAndroidComposeRule<MainActivity>()

    private val clipboard get() = AndroidClipboard(compose.activity)

    private fun <T> onMain(block: () -> T): T {
        compose.waitForIdle()
        takeFocus()
        var result: Result<T>? = null
        compose.runOnUiThread { result = runCatching(block) }
        return result!!.getOrThrow()
    }

    // a system window over the app (a slow emulator's "isn't responding" dialog) holds the focus until closed
    private fun takeFocus() {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val deadline = SystemClock.uptimeMillis() + FOCUS_TIMEOUT_MS
        while (!compose.activity.hasWindowFocus()) {
            assertTrue("the app's window never got the focus", SystemClock.uptimeMillis() < deadline)
            closeFocusHolder(automation)
            SystemClock.sleep(FOCUS_POLL_MS)
        }
    }

    // "close system dialogs" makes an "isn't responding" dialog kill its app, which may be this one: its Wait button
    // closes it. With no answer from the window manager in time, nothing is closed and the caller asks again.
    private fun closeFocusHolder(automation: UiAutomation) {
        val focus = shell(automation, FOCUSED_WINDOW).lineSequence().firstOrNull { FOCUS_LINE in it } ?: return
        if (ANR_DIALOG in focus) {
            automation.rootInActiveWindow
                ?.findAccessibilityNodeInfosByViewId(ANR_WAIT)
                ?.firstOrNull()
                ?.performAction(AccessibilityNodeInfo.ACTION_CLICK)
        } else {
            shell(automation, CLOSE_SYSTEM_DIALOGS)
        }
    }

    private fun shell(
        automation: UiAutomation,
        command: String,
    ): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(automation.executeShellCommand(command))
            .use { it.readBytes().decodeToString() }

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
        const val FOCUS_TIMEOUT_MS = 30_000L
        const val FOCUS_POLL_MS = 250L

        // each bounded, so a slow system can't hold a call far past the focus deadline: the dump by its own time
        // limit, the broadcast by not waiting for its receivers
        const val FOCUSED_WINDOW = "dumpsys -t 5 window displays"
        const val FOCUS_LINE = "mCurrentFocus="
        const val ANR_DIALOG = "Application Not Responding:"
        const val ANR_WAIT = "android:id/aerr_wait"
        const val CLOSE_SYSTEM_DIALOGS = "am broadcast --async -a android.intent.action.CLOSE_SYSTEM_DIALOGS"

        // Ghostty's clipboard-multipart.py image: a 2x2 red PNG, 73 bytes
        val RED_PNG: ByteArray =
            Base64.decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167WwAAAABJRU5ErkJggg==",
                Base64.DEFAULT,
            )
    }
}
