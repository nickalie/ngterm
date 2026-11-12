package com.nickalie.ngterm.android.platform

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.RED_PNG
import com.nickalie.ngterm.core.ffi.ClipboardPart
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Test
import org.junit.runner.RunWith
import java.io.FileNotFoundException

/** The content URIs of the clipboard's image: read-only, the one file only, its name and size. */
@RunWith(AndroidJUnit4::class)
class ClipboardFilesTest {
    private val context = ApplicationProvider.getApplicationContext<Context>()
    private val resolver get() = context.contentResolver

    private fun uri(name: String) =
        Uri
            .Builder()
            .scheme("content")
            .authority(AndroidClipboard.authority(context))
            .appendPath(name)
            .build()

    // the image a write keeps, and its URI
    private fun written(): Uri {
        AndroidClipboard(context).write(listOf(ClipboardPart("image/png", RED_PNG)))
        val name =
            java.io
                .File(context.cacheDir, "clipboard")
                .listFiles()!!
                .single()
                .name
        return uri(name)
    }

    @Test
    fun theImageHasItsTypeNameAndSize() {
        val uri = written()
        assertEquals("image/png", resolver.getType(uri))
        resolver.query(uri, null, null, null, null)!!.use { cursor ->
            cursor.moveToFirst()
            assertEquals(
                uri.lastPathSegment,
                cursor.getString(cursor.getColumnIndexOrThrow(OpenableColumns.DISPLAY_NAME)),
            )
            assertEquals(73L, cursor.getLong(cursor.getColumnIndexOrThrow(OpenableColumns.SIZE)))
        }
        resolver.query(uri, arrayOf(OpenableColumns.SIZE, "other"), null, null, null)!!.use { cursor ->
            cursor.moveToFirst()
            assertEquals(73L, cursor.getLong(0))
            assertEquals(true, cursor.isNull(1))
        }
    }

    @Test
    fun onlyTheKeptFileIsServedAndOnlyForReading() {
        val uri = written()
        assertThrows(SecurityException::class.java) { resolver.openFileDescriptor(uri, "w") }
        assertThrows(FileNotFoundException::class.java) { resolver.openFileDescriptor(uri("clip-gone.png"), "r") }
        assertNull(resolver.query(uri("clip-gone.png"), null, null, null, null))
        assertNull(resolver.query(uri(".hidden"), null, null, null, null))
        val nested = uri.buildUpon().appendPath("more").build()
        assertNull(resolver.query(nested, null, null, null, null))
        assertNull(resolver.getType(nested))
    }

    @Test
    fun nothingIsAddedChangedOrRemovedThroughIt() {
        val provider = ClipboardFiles()
        val uri = written()
        assertNull(provider.insert(uri, ContentValues()))
        assertEquals(0, provider.update(uri, ContentValues(), null, null))
        assertEquals(0, provider.delete(uri, null, null))
    }
}
