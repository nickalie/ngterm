package com.nickalie.ngterm.android.platform

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.Build
import android.os.ParcelFileDescriptor
import android.os.ParcelFileDescriptor.MODE_READ_ONLY
import android.provider.OpenableColumns
import android.text.Html
import android.webkit.MimeTypeMap
import com.nickalie.ngterm.android.data.ClipboardWriter
import com.nickalie.ngterm.android.data.DeviceClipboard
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardWriteStatus
import java.io.File
import java.io.FileNotFoundException
import java.io.IOException
import java.io.InputStream

/**
 * The system clipboard per MIME type, read from any thread and written from the main one. It keeps text
 * (`text/plain`), HTML (`text/html`, with its text beside it, as Android's clips carry HTML) and images: an image is a
 * file in the app's cache served through [ClipboardFiles] (a content URI, as Android's clipboard requires), replaced by
 * the next write. Other types aren't kept. Android lets only the app in front (or the keyboard) read the clipboard: in
 * the background it reads as holding nothing. A write shows Android's own notice of it.
 */
class AndroidClipboard(
    private val context: Context,
) : DeviceClipboard,
    ClipboardWriter {
    private val manager = context.getSystemService(ClipboardManager::class.java)
    private val files get() = File(context.cacheDir, FILES_DIR)

    override fun types(): List<String> {
        val clip = manager?.primaryClip?.takeIf { it.itemCount > 0 } ?: return emptyList()
        val item = clip.getItemAt(0)
        val types = mutableListOf<String>()
        if (text(item) != null) types += TEXT
        if (item.htmlText != null) types += HTML
        item.uri?.let(::contentType)?.let { types += it }
        val description = clip.description
        for (index in 0 until description.mimeTypeCount) {
            val type = description.getMimeType(index)
            // the description's text types are the item's text, listed once above; a URI's own type is above too
            if (!type.startsWith("text/") && type !in types && !type.contains('*')) types += type
        }
        return types.distinct()
    }

    override fun read(mimes: List<String>): List<ClipboardPart> {
        val clip = manager?.primaryClip?.takeIf { it.itemCount > 0 } ?: return emptyList()
        val item = clip.getItemAt(0)
        return mimes.distinct().mapNotNull { mime ->
            when (mime) {
                TEXT -> {
                    text(item)?.let { ClipboardPart(mime, it.toByteArray()) }
                }

                HTML -> {
                    item.htmlText?.let { ClipboardPart(mime, it.toByteArray()) }
                }

                else -> {
                    item.uri
                        ?.takeIf { contentType(it) == mime }
                        ?.let { bytes(it) }
                        ?.let { ClipboardPart(mime, it) }
                }
            }
        }
    }

    override fun write(contents: List<ClipboardPart>): ClipboardWriteStatus {
        val clipboard = manager ?: return ClipboardWriteStatus.UNSUPPORTED
        val text = contents.firstOrNull { it.mime == TEXT }?.data?.decodeToString()
        val html = contents.firstOrNull { it.mime == HTML }?.data?.decodeToString()
        val image = contents.firstOrNull { it.mime.startsWith(IMAGE) && it.mime.length > IMAGE.length }
        var stored: File? = null
        val status =
            when {
                contents.isEmpty() -> {
                    written { clear(clipboard) }
                }

                text == null && html == null && image == null -> {
                    ClipboardWriteStatus.UNSUPPORTED
                }

                else -> {
                    written {
                        stored = image?.let(::store)
                        clipboard.setPrimaryClip(clip(text, html, image?.mime, stored))
                    }
                }
            }
        // the last write's image stays until its successor is on the clipboard: a failed write leaves it readable
        if (status == ClipboardWriteStatus.DONE) {
            files.listFiles()?.filter { it != stored }?.forEach(::remove)
        } else {
            stored?.let(::remove)
        }
        return status
    }

    // DONE once [write] has run; FAILED when it threw. The system clipboard reports a failed binder call (a clip too
    // large for the transaction, a dead system server) as a plain RuntimeException, so every one is caught.
    private inline fun written(write: () -> Unit): ClipboardWriteStatus =
        try {
            write()
            ClipboardWriteStatus.DONE
        } catch (_: IOException) {
            ClipboardWriteStatus.FAILED
        } catch (_: RuntimeException) {
            ClipboardWriteStatus.FAILED
        }

    // the clip of what Android's clipboard can hold of a write: its text, its HTML and its first image (stored)
    private fun clip(
        text: String?,
        html: String?,
        imageType: String?,
        image: File?,
    ): ClipData {
        val types =
            buildList {
                if (text != null || html != null) add(ClipDescription.MIMETYPE_TEXT_PLAIN)
                if (html != null) add(ClipDescription.MIMETYPE_TEXT_HTML)
                if (imageType != null) add(imageType)
            }
        // Android's HTML item carries its text too: the HTML's own when the program wrote none
        val shown = text ?: html?.let { Html.fromHtml(it, Html.FROM_HTML_MODE_COMPACT).toString() }
        val uri =
            image?.let {
                Uri
                    .Builder()
                    .scheme("content")
                    .authority(authority(context))
                    .appendPath(it.name)
                    .build()
            }
        return ClipData(ClipDescription(LABEL, types.toTypedArray()), ClipData.Item(shown, html, null, uri))
    }

    // a new file holding the image, beside the last write's until this one is on the clipboard; none when it fails
    private fun store(image: ClipboardPart): File {
        val dir = files
        dir.mkdirs()
        val extension = MimeTypeMap.getSingleton().getExtensionFromMimeType(image.mime) ?: "bin"
        val file = File(dir, "clip-${System.nanoTime()}.$extension")
        try {
            file.writeBytes(image.data)
        } catch (failed: IOException) {
            file.delete()
            throw failed
        }
        types[file.name] = image.mime
        return file
    }

    private fun clear(clipboard: ClipboardManager) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            clipboard.clearPrimaryClip()
        } else {
            clipboard.setPrimaryClip(ClipData.newPlainText(LABEL, ""))
        }
    }

    // an image file, and the type it was written with
    private fun remove(file: File) {
        file.delete()
        types.remove(file.name)
    }

    // a clip's text: its own, else a link that isn't content (a content URI is data, read by its type)
    private fun text(item: ClipData.Item): String? =
        item.text?.toString() ?: item.uri?.takeIf { it.scheme != "content" }?.toString()

    private fun contentType(uri: Uri): String? =
        if (uri.scheme == "content") {
            ourType(uri) ?: runCatching { context.contentResolver.getType(uri) }.getOrNull()
        } else {
            null
        }

    // an image this process wrote keeps the type it was written with (an extension can't name every type)
    private fun ourType(uri: Uri): String? =
        uri.takeIf { it.authority == authority(context) }?.lastPathSegment?.let(::typeOf)

    private fun bytes(uri: Uri): ByteArray? =
        try {
            context.contentResolver.openInputStream(uri)?.use { it.readBounded() }
        } catch (_: IOException) {
            null
        } catch (_: SecurityException) {
            null
        }

    // the stream's bytes, or null past MAX_READ
    private fun InputStream.readBounded(): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(BUFFER)
        while (true) {
            val read = read(buffer)
            if (read < 0) return out.toByteArray()
            if (out.size() + read > MAX_READ) return null
            out.write(buffer, 0, read)
        }
    }

    companion object {
        // what the system's clipboard notice and history call a program's copy
        const val LABEL = "ngterm"
        const val TEXT = "text/plain"
        const val HTML = "text/html"
        private const val IMAGE = "image/"
        private const val FILES_DIR = "clipboard"
        private const val BUFFER = 64 * 1024

        /** The most bytes one representation another app put on the clipboard is read with. */
        const val MAX_READ = 32 * 1024 * 1024

        // the MIME type of each image file this process wrote, by its name
        private val types = java.util.concurrent.ConcurrentHashMap<String, String>()

        /** The authority of [ClipboardFiles]: the app's id, so a debug and a release build never share it. */
        fun authority(context: Context) = "${context.packageName}.clipboard"

        /** The image file named [name] that the last write kept, if it is still there. */
        internal fun file(
            context: Context,
            name: String,
        ): File? {
            if (name.isEmpty() || name.contains('/') || name.startsWith('.')) return null
            return File(File(context.cacheDir, FILES_DIR), name).takeIf { it.isFile }
        }

        /** The MIME type of the image file named [name]: as written, else by its extension. */
        internal fun typeOf(name: String): String? =
            types[name]
                ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', ""))
    }
}

/**
 * The content URIs of the images [AndroidClipboard] puts on the clipboard: read-only, one file at a time, nothing
 * listed. The clipboard grants each app that reads the clip its own read permission; the provider isn't exported.
 */
class ClipboardFiles : ContentProvider() {
    override fun onCreate() = true

    override fun getType(uri: Uri): String? = name(uri)?.let(AndroidClipboard::typeOf)

    override fun openFile(
        uri: Uri,
        mode: String,
    ): ParcelFileDescriptor {
        if (mode != "r") throw SecurityException("clipboard images are read-only")
        val file = name(uri)?.let { name -> context?.let { AndroidClipboard.file(it, name) } }
        return ParcelFileDescriptor.open(file ?: throw FileNotFoundException(uri.toString()), MODE_READ_ONLY)
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor? {
        val name = name(uri)
        val file = name?.let { context?.let { context -> AndroidClipboard.file(context, it) } } ?: return null
        val columns = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        val row =
            columns.map { column ->
                when (column) {
                    OpenableColumns.DISPLAY_NAME -> name
                    OpenableColumns.SIZE -> file.length()
                    else -> null
                }
            }
        return MatrixCursor(columns, 1).apply { addRow(row) }
    }

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

    private fun name(uri: Uri) = uri.pathSegments.singleOrNull()
}
