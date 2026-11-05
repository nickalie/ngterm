package com.nickalie.ngterm.android.data

import org.junit.Assert.assertEquals
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.InputStream

/** The bounded read: no byte past its limit, whatever each read of the stream would hand over. */
class StreamsTest {
    // a stream of [size] bytes that counts what it gave
    private class Counted(
        private val size: Int,
    ) : InputStream() {
        var given = 0
            private set

        override fun read(): Int = if (given < size) 1.also { given++ } else -1

        override fun read(
            b: ByteArray,
            off: Int,
            len: Int,
        ): Int {
            if (given >= size) return -1
            val count = minOf(len, size - given)
            given += count
            return count
        }
    }

    @Test
    fun aLongerStreamIsReadNoFurtherThanTheLimit() {
        val limit = 3 * DEFAULT_BUFFER_SIZE + 1
        val stream = Counted(limit * 4)
        assertEquals(limit, stream.readAtMost(limit).size)
        assertEquals("nothing read past the limit", limit, stream.given)
    }

    @Test
    fun aShorterStreamIsReadWhole() {
        val bytes = ByteArray(10) { it.toByte() }
        assertEquals(bytes.toList(), ByteArrayInputStream(bytes).readAtMost(100).toList())
    }
}
