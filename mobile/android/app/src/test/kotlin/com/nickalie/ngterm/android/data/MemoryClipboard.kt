package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardReply

/** A device clipboard in memory, holding [contents] (each its own type): what the clipboard reads see in tests. */
class MemoryClipboard(
    @Volatile var contents: List<ClipboardPart> = emptyList(),
) : DeviceClipboard {
    /** Its text alone, as most tests set it; `null`: it holds nothing. */
    var text: String?
        get() = contents.firstOrNull { it.mime == "text/plain" }?.data?.decodeToString()
        set(value) {
            contents = listOfNotNull(value?.let { part("text/plain", it) })
        }

    override fun types() = contents.map { it.mime }

    /** How many times its data was read. */
    @Volatile var reads = 0
        private set

    override fun read(mimes: List<String>): List<ClipboardPart> {
        reads++
        return mimes.mapNotNull { mime -> contents.firstOrNull { it.mime == mime } }
    }
}

fun part(
    mime: String,
    text: String,
) = ClipboardPart(mime, text.toByteArray())

/** A part as it compares in a test: its type and its bytes (a `ByteArray` compares by identity). */
fun ClipboardPart.key() = mime to data.toList()

/** A reply as it compares in a test. */
fun ClipboardReply.key(): Any =
    when (this) {
        is ClipboardReply.Allow -> Triple(contents.map { it.key() }, available, remember)
        ClipboardReply.Deny -> "deny"
    }

/** An Allow handing over [contents] and [available], keeping a grant with [remember], as it compares. */
fun allowed(
    vararg contents: ClipboardPart,
    available: List<String> = emptyList(),
    remember: Boolean = false,
): Any = ClipboardReply.Allow(contents.toList(), available, remember).key()

/** Ghostty's `clipboard-multipart.py` image, a 2x2 red PNG of 73 bytes (`crates/ngterm-ghostty-vt/tests/data`). */
val RED_PNG: ByteArray =
    java.util.Base64.getDecoder().decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAIAAAD91JpzAAAAEElEQVR42mP4z8AARAwQCgAf7gP9Y167WwAAAABJRU5ErkJggg==",
    )
