package com.nickalie.ngterm.android.update

import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.IOException
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.URL

/** The update's network reads: the release list and the checksums whole, the APK into a file. */
interface UpdateHttp {
    /** The body of [url], at most [limit] bytes. */
    suspend fun fetch(
        url: String,
        limit: Long,
    ): ByteArray

    /** Streams [url] into [into], which must come to exactly [size] bytes, telling [progress] the bytes so far. */
    suspend fun download(
        url: String,
        into: File,
        size: Long,
        progress: (Long) -> Unit,
    )
}

/**
 * [UpdateHttp] over the platform's [HttpURLConnection], as desktop's updater asks GitHub (its headers, a bounded
 * read). Call it off the main thread; a cancelled coroutine stops a read between chunks.
 */
class UrlConnectionHttp(
    private val userAgent: String,
) : UpdateHttp {
    override suspend fun fetch(
        url: String,
        limit: Long,
    ): ByteArray =
        open(url).use { body ->
            val bytes = ByteArrayOutputStream()
            val buffer = ByteArray(CHUNK)
            while (true) {
                val read = body.read(buffer)
                if (read < 0) break
                bytes.write(buffer, 0, read)
                if (bytes.size() > limit) throw IOException("$url is larger than expected")
            }
            bytes.toByteArray()
        }

    override suspend fun download(
        url: String,
        into: File,
        size: Long,
        progress: (Long) -> Unit,
    ) {
        open(url).use { body ->
            into.outputStream().use { out ->
                val buffer = ByteArray(CHUNK)
                var total = 0L
                while (true) {
                    currentCoroutineContext().ensureActive()
                    val read = body.read(buffer)
                    if (read < 0) break
                    total += read
                    if (total > size) throw IOException("the download is larger than the release says")
                    out.write(buffer, 0, read)
                    progress(total)
                }
                if (total != size) throw IOException("the download ended early")
            }
        }
    }

    private fun open(url: String): InputStream {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.connectTimeout = TIMEOUT_MS
        connection.readTimeout = TIMEOUT_MS
        connection.setRequestProperty("User-Agent", userAgent)
        connection.setRequestProperty("Accept", "application/vnd.github+json")
        connection.setRequestProperty("X-GitHub-Api-Version", "2022-11-28")
        val status = connection.responseCode
        if (status != HttpURLConnection.HTTP_OK) {
            connection.disconnect()
            throw IOException("GitHub answered $status")
        }
        return connection.inputStream
    }

    private companion object {
        const val CHUNK = 64 * 1024
        const val TIMEOUT_MS = 30_000
    }
}
