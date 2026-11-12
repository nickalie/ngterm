package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Bitmap
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.ClipboardSnapshot
import com.nickalie.ngterm.android.data.RED_PNG
import com.nickalie.ngterm.android.data.part
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ClipboardPart
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode
import java.io.ByteArrayOutputStream

/** What the clipboard read's preview shows of each part: its pages, bodies and sizes, and the states around them. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ClipboardPreviewTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(snapshot: ClipboardSnapshot?) {
        compose.setContent { NgtermTheme { ClipboardPreview(snapshot) } }
        compose.waitForIdle()
    }

    @Test
    fun eachPartIsAPageByItsKind() {
        val pages =
            previewPages(
                listOf(
                    part("text/plain", "héllo"),
                    ClipboardPart("image/png", RED_PNG),
                    ClipboardPart("application/octet-stream", byteArrayOf(-1, -2, 0)),
                    ClipboardPart("text/html", ByteArray(0)),
                ),
            )
        assertEquals(listOf("text/plain", "image/png", "application/octet-stream", "text/html"), pages.map { it.mime })
        assertEquals(PreviewBody.Text("héllo"), pages[0].body)
        assertEquals(PreviewBody.Image(RED_PNG), pages[1].body)
        assertEquals(PreviewBody.NoPreview, pages[2].body)
        assertEquals(PreviewBody.Empty, pages[3].body)
        assertEquals(listOf(6, 73, 3, 0), pages.map { it.size })
    }

    @Test
    fun longTextIsCutOnACharactersEdgeAndTheRestCounted() {
        val kept = "a".repeat(MAX_PREVIEW_TEXT - 1)
        val shown = boundedText(kept + "😀" + "b".repeat(1500))
        assertTrue(shown.startsWith(kept + "\n"))
        assertTrue(shown, shown.endsWith("… 1.5 kB more"))
        assertEquals("short", boundedText("short"))
    }

    @Test
    fun theBoundCountsUtf8BytesNotCharacters() {
        // two bytes a character: the bound keeps half as many characters as bytes, and counts the rest in bytes
        val text = "é".repeat(MAX_PREVIEW_TEXT)
        val shown = boundedText(text)
        assertEquals("é".repeat(MAX_PREVIEW_TEXT / 2) + "\n… 65.5 kB more", shown)
        // a three-byte character that would straddle the bound is left out whole
        val straddling = boundedText("a".repeat(MAX_PREVIEW_TEXT - 1) + "€" + "b")
        assertEquals("a".repeat(MAX_PREVIEW_TEXT - 1) + "\n… 4 bytes more", straddling)
        val exact = "a".repeat(MAX_PREVIEW_TEXT - 2) + "é"
        assertEquals("exactly the bound is kept whole", exact, boundedText(exact))
    }

    @Test
    fun allowAndAlwaysWaitForThePreview() {
        val question = mutableStateOf<ClipboardSnapshot?>(null)
        var answered = 0
        compose.setContent {
            NgtermTheme {
                ClipboardReadQuestion(
                    canRemember = false,
                    machine = "build-01",
                    session = "docs-site",
                    preview = question.value,
                    onAllow = { answered += 1 },
                    onAlways = { answered += 1 },
                    onDeny = {},
                )
            }
        }
        compose.onNodeWithText("Allow").assertIsNotEnabled().performClick()
        compose.onNodeWithText("Always allow on build-01").assertIsNotEnabled().performClick()
        compose.onNodeWithText("Deny").assertIsEnabled()
        assertEquals("nothing is handed over before it is shown", 0, answered)

        question.value = ClipboardSnapshot(listOf(part("text/plain", "copied")), emptyList())
        compose.waitForIdle()
        compose.onNodeWithText("Allow").assertIsEnabled().performClick()
        compose.onNodeWithText("Always allow on build-01").assertIsEnabled()
        assertEquals(1, answered)
    }

    @Test
    fun sizesReadAsGhosttysDialogSaysThem() {
        assertEquals(
            listOf("1 byte", "73 bytes", "999 bytes", "2.0 kB", "1.5 MB", "2.1 GB"),
            listOf(1, 73, 999, 2048, 1_500_000, Int.MAX_VALUE).map(::formatSize),
        )
    }

    @Test
    fun aSnapshotStillBeingReadSaysSo() {
        show(null)
        compose.onNodeWithText("Reading your clipboard…").assertExists()
    }

    @Test
    fun noneOfTheTypesSaysWhatTheProgramStillLearns() {
        show(ClipboardSnapshot(emptyList(), listOf("text/plain")))
        compose
            .onNodeWithText("It sees only which kinds your clipboard holds.", substring = true)
            .assertExists()
    }

    @Test
    fun onePageSaysItsTypeWithItsSizeAndABrokenImageHasNoPreview() {
        show(ClipboardSnapshot(listOf(ClipboardPart("image/png", byteArrayOf(1, 2, 3))), emptyList()))
        compose.onNodeWithText("No preview for this kind of content").assertExists()
        compose.onNodeWithText("image/png, 3 bytes").assertExists()
    }

    @Test
    fun aLargeImageIsDecodedSmallerAndShown() {
        val bitmap = Bitmap.createBitmap(1200, 900, Bitmap.Config.ARGB_8888)
        val png =
            ByteArrayOutputStream().use { out ->
                bitmap.compress(Bitmap.CompressFormat.PNG, 100, out).also { out.flush() }.let { out.toByteArray() }
            }
        show(ClipboardSnapshot(listOf(ClipboardPart("image/png", png)), emptyList()))
        compose.onNodeWithContentDescription("Copied image, image/png").assertExists()
    }
}
