package com.nickalie.ngterm.android.ui.chat

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.pm.ProviderInfo
import android.content.res.AssetFileDescriptor
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.provider.OpenableColumns
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.shadows.ShadowContentResolver
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.InputStream

/** Document providers may omit metadata, revoke access or fail partway through a read. */
@RunWith(AndroidJUnit4::class)
class PickedAttachmentTest {
    private val resolver = ApplicationProvider.getApplicationContext<Context>().contentResolver
    private val uri = Uri.parse("content://attachments/document/folder%2Fnotes.txt")

    @Test
    fun theProvidersDisplayNameAndSizeAreUsedAndItsCursorIsClosed() {
        val provider = MetadataProvider()
        ShadowContentResolver.registerProviderInternal("attachments", provider)
        val cursor = metadata("report.txt", 42L)
        provider.cursor = cursor
        assertEquals(PickedFile("report.txt", 42), pickedFile(resolver, uri))
        assertTrue(cursor.isClosed)
    }

    @Test
    fun absentOrBlankNamesFallBackToTheLastPartOfTheUriAndUnknownSizesStayUnknown() {
        val provider = MetadataProvider()
        ShadowContentResolver.registerProviderInternal("attachments", provider)
        listOf(null, "", "  ").forEach { name ->
            val cursor = metadata(name, null)
            provider.cursor = cursor
            assertEquals(PickedFile("notes.txt", null), pickedFile(resolver, uri))
            assertTrue(cursor.isClosed)
        }
        provider.cursor = metadata("empty.txt", 0)
        assertEquals(PickedFile("empty.txt", 0), pickedFile(resolver, uri))
    }

    @Test
    fun anAbsentOrEmptyResultAndRevokedPermissionStillGiveTheFileAFallbackName() {
        val provider = MetadataProvider()
        ShadowContentResolver.registerProviderInternal("attachments", provider)
        assertEquals(PickedFile("notes.txt", null), pickedFile(resolver, uri))
        val empty = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE))
        provider.cursor = empty
        assertEquals(PickedFile("notes.txt", null), pickedFile(resolver, uri))
        assertTrue(empty.isClosed)
        provider.denied = true
        assertEquals(PickedFile("notes.txt", null), pickedFile(resolver, uri))
        assertEquals(PickedFile("upload", null), pickedFile(resolver, Uri.parse("content://attachments")))
        assertEquals(PickedFile("upload", null), pickedFile(resolver, Uri.parse("content://attachments/")))
    }

    @Test
    fun readingReturnsTheBytesAndClosesTheStreamIncludingForAnEmptyFile() {
        listOf(byteArrayOf(1, 2, 3), byteArrayOf()).forEach { bytes ->
            val input = TrackedInput(bytes)
            shadowOf(resolver).registerInputStream(uri, input)
            assertArrayEquals(bytes, readPicked(resolver, uri))
            assertTrue(input.closed)
        }
    }

    @Test
    fun readingStopsOneBytePastTheUploadLimitSoAnUnknownSizeCanBeRefused() {
        val input = CountingInput(UPLOAD_CAP + 20)
        shadowOf(resolver).registerInputStream(uri, input)
        val bytes = readPicked(resolver, uri)
        assertEquals((UPLOAD_CAP + 1).toInt(), bytes.size)
        assertEquals(UPLOAD_CAP + 1, input.read)
        assertTrue(input.closed)
    }

    @Test
    fun aReadFailureClosesTheStreamAndIsReturnedToTheUploadFlow() {
        val input =
            object : InputStream() {
                var closed = false

                override fun read(): Int = throw IOException("permission lost")

                override fun close() {
                    closed = true
                }
            }
        shadowOf(resolver).registerInputStream(uri, input)
        val error = assertThrows(IOException::class.java) { readPicked(resolver, uri) }
        assertEquals("permission lost", error.message)
        assertTrue(input.closed)
    }

    @Test
    fun aProviderThatCannotOpenItsStreamReturnsAnIoFailure() {
        val provider = MetadataProvider()
        provider.attachInfo(
            ApplicationProvider.getApplicationContext(),
            ProviderInfo().apply {
                authority =
                    "attachments"
            },
        )
        ShadowContentResolver.registerProviderInternal("attachments", provider)
        assertThrows(IOException::class.java) { readPicked(resolver, uri) }
    }

    private fun metadata(
        name: String?,
        size: Long?,
    ) = MatrixCursor(arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)).apply {
        addRow(arrayOf<Any?>(name, size))
    }

    private class TrackedInput(
        bytes: ByteArray,
    ) : ByteArrayInputStream(bytes) {
        var closed = false

        override fun close() {
            closed = true
            super.close()
        }
    }

    private class CountingInput(
        private val size: Long,
    ) : InputStream() {
        var read = 0L
        var closed = false

        override fun read(): Int = if (read++ < size) 0 else -1

        override fun read(
            buffer: ByteArray,
            offset: Int,
            length: Int,
        ): Int {
            if (read == size) return -1
            val count = minOf(length.toLong(), size - read).toInt()
            buffer.fill(0, offset, offset + count)
            read += count
            return count
        }

        override fun close() {
            closed = true
        }
    }

    private class MetadataProvider : ContentProvider() {
        var cursor: Cursor? = null
        var denied = false

        override fun onCreate() = true

        override fun query(
            uri: Uri,
            projection: Array<out String>?,
            selection: String?,
            selectionArgs: Array<out String>?,
            sortOrder: String?,
        ): Cursor? {
            if (denied) throw SecurityException("revoked")
            return cursor
        }

        override fun openAssetFile(
            uri: Uri,
            mode: String,
        ): AssetFileDescriptor? = null

        override fun getType(uri: Uri): String? = null

        override fun insert(
            uri: Uri,
            values: ContentValues?,
        ): Uri? = null

        override fun delete(
            uri: Uri,
            selection: String?,
            selectionArgs: Array<out String>?,
        ) = 0

        override fun update(
            uri: Uri,
            values: ContentValues?,
            selection: String?,
            selectionArgs: Array<out String>?,
        ) = 0
    }
}
