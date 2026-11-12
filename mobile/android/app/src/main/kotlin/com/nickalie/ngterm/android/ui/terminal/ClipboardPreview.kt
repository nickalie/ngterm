package com.nickalie.ngterm.android.ui.terminal

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ClipboardSnapshot
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ClipboardPart
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.util.Locale

/** The tallest an image shows in the preview (Ghostty's confirmation dialog: 200 logical px). */
private val PREVIEW_IMAGE_HEIGHT = 200.dp

/** The tallest text shows before it scrolls inside its plate. */
private val PREVIEW_TEXT_HEIGHT = 160.dp

/** The most UTF-8 bytes of a text part the preview lays out; the rest is counted (desktop's `bounded_text`). */
internal const val MAX_PREVIEW_TEXT = 64 * 1024

/** The most UTF-8 bytes one UTF-16 unit takes (a surrogate pair takes four for its two). */
private const val MAX_UTF8_PER_UNIT = 3

/** The top two bits of a byte, and their value in a UTF-8 continuation byte (never a character's first byte). */
private const val UTF8_LEAD_MASK = 0xC0
private const val UTF8_CONTINUATION = 0x80

/** The largest side an image is decoded to: twice the preview's height, so it stays sharp and small. */
private const val DECODE_SIDE = 400

/** How one part of a clipboard value shows in the preview. */
internal sealed interface PreviewBody {
    /** An image to decode (shown as a no-preview note when it doesn't decode). */
    data class Image(
        val data: ByteArray,
    ) : PreviewBody {
        override fun equals(other: Any?) = other is Image && other.data.contentEquals(data)

        override fun hashCode() = data.contentHashCode()
    }

    /** Valid UTF-8, bounded to [MAX_PREVIEW_TEXT] bytes with what was cut counted. */
    data class Text(
        val text: String,
    ) : PreviewBody

    /** Data with nothing to show but its size. */
    data object NoPreview : PreviewBody

    /** A part with no bytes. */
    data object Empty : PreviewBody
}

/** What one part shows: its type, its body and its size in bytes (Ghostty's GTK confirmation, one page per type). */
internal data class PreviewPage(
    val mime: String,
    val body: PreviewBody,
    val size: Int,
)

/** Each part of [contents] as a page, in order. */
internal fun previewPages(contents: List<ClipboardPart>): List<PreviewPage> =
    contents.map { part ->
        val data = part.data
        val body =
            when {
                data.isEmpty() -> PreviewBody.Empty
                part.mime.startsWith("image/") -> PreviewBody.Image(data)
                else -> utf8(data)?.let { PreviewBody.Text(boundedText(it)) } ?: PreviewBody.NoPreview
            }
        PreviewPage(part.mime, body, data.size)
    }

private fun utf8(data: ByteArray): String? =
    try {
        Charsets.UTF_8
            .newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
            .decode(ByteBuffer.wrap(data))
            .toString()
    } catch (_: CharacterCodingException) {
        null
    }

/**
 * [text] cut at [MAX_PREVIEW_TEXT] bytes of its UTF-8, on a character's edge, with a line counting the rest in bytes:
 * desktop's `bounded_text` and iOS's `ClipboardSummary.bounded`.
 */
internal fun boundedText(text: String): String {
    // text this short is within the bound however it encodes: no need to encode it
    val bytes = if (text.length <= MAX_PREVIEW_TEXT / MAX_UTF8_PER_UNIT) null else text.encodeToByteArray()
    if (bytes == null || bytes.size <= MAX_PREVIEW_TEXT) return text
    var end = MAX_PREVIEW_TEXT
    while (end > 0 && bytes[end].toInt() and UTF8_LEAD_MASK == UTF8_CONTINUATION) end -= 1
    return "${bytes.decodeToString(0, end)}\n… ${formatSize(bytes.size - end)} more"
}

/** A byte count as Ghostty's dialog says it (glib's `g_format_size`): "1 byte", "73 bytes", "2.0 kB", "1.5 MB". */
internal fun formatSize(size: Int): String =
    when {
        size == 1 -> {
            "1 byte"
        }

        size < KILO -> {
            "$size bytes"
        }

        else -> {
            var value = size.toDouble() / KILO
            var unit = 0
            while (value >= KILO && unit < UNITS.lastIndex) {
                value /= KILO
                unit += 1
            }
            String.format(Locale.ROOT, "%.1f %s", value, UNITS[unit])
        }
    }

private const val KILO = 1000
private val UNITS = listOf("kB", "MB", "GB")

/**
 * What a program's read would hand over, under the question: a page per type, picked with a chip each (when there is
 * more than one), on a `surfaceContainerLow` plate. An image shows at most [PREVIEW_IMAGE_HEIGHT] tall, text in mono,
 * other data as a note with its size, an empty part as such; each page says its size under the plate. A [snapshot]
 * still being read shows a quiet line, one with none of the types says so.
 */
@Composable
internal fun ClipboardPreview(
    snapshot: ClipboardSnapshot?,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
        when {
            snapshot == null -> {
                Quiet(stringResource(R.string.clipboard_preview_reading))
            }

            snapshot.contents.isEmpty() -> {
                Quiet(
                    stringResource(
                        if (snapshot.available.isEmpty()) {
                            R.string.clipboard_preview_none
                        } else {
                            R.string.clipboard_preview_none_listed
                        },
                    ),
                )
            }

            else -> {
                Pages(previewPages(snapshot.contents))
            }
        }
    }
}

@Composable
private fun Pages(pages: List<PreviewPage>) {
    val spacing = NgtermTheme.spacing
    var selected by remember(pages) { mutableIntStateOf(0) }
    Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
        if (pages.size > 1) {
            Row(
                modifier = Modifier.horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(spacing.sm),
            ) {
                pages.forEachIndexed { index, page ->
                    FilterChip(
                        selected = index == selected,
                        onClick = { selected = index },
                        label = { Text(page.mime, style = NgtermTheme.mono.small) },
                    )
                }
            }
        }
        val page = pages[selected.coerceIn(0, pages.lastIndex)]
        Box(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.small)
                    .padding(spacing.md),
        ) {
            Body(page)
        }
        val caption =
            if (pages.size > 1) {
                formatSize(page.size)
            } else {
                stringResource(R.string.clipboard_preview_caption, page.mime, formatSize(page.size))
            }
        Text(
            text = caption,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun Body(page: PreviewPage) {
    when (val body = page.body) {
        is PreviewBody.Image -> {
            val bitmap = remember(body) { decode(body.data) }
            if (bitmap == null) {
                Quiet(stringResource(R.string.clipboard_preview_no_preview))
            } else {
                val description = stringResource(R.string.clipboard_preview_image, page.mime)
                Image(
                    bitmap = bitmap.asImageBitmap(),
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    alignment = Alignment.CenterStart,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .heightIn(max = PREVIEW_IMAGE_HEIGHT)
                            .semantics { contentDescription = description },
                )
            }
        }

        is PreviewBody.Text -> {
            Text(
                text = body.text,
                style = NgtermTheme.mono.small,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.heightIn(max = PREVIEW_TEXT_HEIGHT).verticalScroll(rememberScrollState()),
            )
        }

        PreviewBody.NoPreview -> {
            Quiet(stringResource(R.string.clipboard_preview_no_preview))
        }

        PreviewBody.Empty -> {
            Quiet(stringResource(R.string.clipboard_preview_empty))
        }
    }
}

@Composable
private fun Quiet(text: String) {
    Text(text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

// an untrusted image, decoded no larger than DECODE_SIDE on its longer side; null when it isn't one Android decodes
private fun decode(data: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(data, 0, data.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= DECODE_SIDE) sample *= 2
    val options = BitmapFactory.Options().apply { inSampleSize = sample }
    return BitmapFactory.decodeByteArray(data, 0, data.size, options)
}
