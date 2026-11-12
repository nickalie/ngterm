package com.nickalie.ngterm.android.update

import androidx.test.ext.junit.runners.AndroidJUnit4
import com.sun.net.httpserver.HttpServer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.cancel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import java.io.File
import java.io.IOException
import java.net.InetSocketAddress

/** The platform HTTP client against a local server: GitHub's headers, bounded reads and exact-size downloads. */
@RunWith(AndroidJUnit4::class)
class UrlConnectionHttpTest {
    @get:Rule
    val dir = TemporaryFolder()

    private val server = HttpServer.create(InetSocketAddress("127.0.0.1", 0), 0)
    private val http = UrlConnectionHttp("ngterm-android/0.2.3")
    private val headers = mutableMapOf<String, String?>()
    private val big = ByteArray(200 * 1024) { (it % 251).toByte() }

    private fun url(path: String) = "http://127.0.0.1:${server.address.port}$path"

    @Before
    fun serve() {
        server.createContext("/") { exchange ->
            headers["User-Agent"] = exchange.requestHeaders.getFirst("User-Agent")
            headers["Accept"] = exchange.requestHeaders.getFirst("Accept")
            val body =
                when (exchange.requestURI.path) {
                    "/list" -> "[]".toByteArray()
                    "/big" -> big
                    else -> null
                }
            exchange.sendResponseHeaders(if (body == null) 404 else 200, body?.size?.toLong() ?: -1)
            exchange.responseBody.use { out -> body?.let(out::write) }
        }
        server.start()
    }

    @After
    fun stop() = server.stop(0)

    @Test
    fun aFetchReadsTheBodyWithGitHubsHeadersWithinItsBound() {
        runBlocking {
            assertArrayEquals("[]".toByteArray(), http.fetch(url("/list"), METADATA_LIMIT))
            assertEquals("ngterm-android/0.2.3", headers["User-Agent"])
            assertEquals("application/vnd.github+json", headers["Accept"])
            assertArrayEquals(big, http.fetch(url("/big"), big.size.toLong()))
        }
        assertThrows(IOException::class.java) { runBlocking { http.fetch(url("/big"), big.size - 1L) } }
        val missing = assertThrows(IOException::class.java) { runBlocking { http.fetch(url("/nope"), 10) } }
        assertEquals("GitHub answered 404", missing.message)
    }

    @Test
    fun aDownloadComesToExactlyItsSizeReportingProgress() {
        val into = File(dir.root, "a.apk")
        val seen = mutableListOf<Long>()
        runBlocking { http.download(url("/big"), into, big.size.toLong()) { seen += it } }
        assertArrayEquals(big, into.readBytes())
        assertEquals(big.size.toLong(), seen.last())
        assertEquals(seen.sorted(), seen)

        val short =
            assertThrows(IOException::class.java) { runBlocking { http.download(url("/big"), into, big.size + 1L) {} } }
        assertEquals("the download ended early", short.message)
        val long = assertThrows(IOException::class.java) { runBlocking { http.download(url("/big"), into, 10) {} } }
        assertEquals("the download is larger than the release says", long.message)
    }

    @Test
    fun aCancelledDownloadStopsBetweenChunks() {
        val into = File(dir.root, "a.apk")
        var chunks = 0
        assertThrows(CancellationException::class.java) {
            runBlocking {
                coroutineScope {
                    http.download(url("/big"), into, big.size.toLong()) {
                        chunks++
                        cancel()
                    }
                }
            }
        }
        assertEquals(1, chunks)
    }
}
