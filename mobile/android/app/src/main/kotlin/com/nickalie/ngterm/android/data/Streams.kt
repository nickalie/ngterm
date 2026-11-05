package com.nickalie.ngterm.android.data

import java.io.ByteArrayOutputStream
import java.io.InputStream

/**
 * Up to [limit] bytes of the stream, never more: each read asks for no more than what is left, so a stream longer
 * than [limit] is read [limit] bytes deep (`InputStream.readNBytes` is API 33).
 */
internal fun InputStream.readAtMost(limit: Int): ByteArray {
    val out = ByteArrayOutputStream()
    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
    while (out.size() < limit) {
        val read = read(buffer, 0, minOf(buffer.size, limit - out.size()))
        if (read < 0) break
        out.write(buffer, 0, read)
    }
    return out.toByteArray()
}
