package com.nickalie.ngterm.android.ui.chat

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import java.io.ByteArrayOutputStream
import java.io.IOException

/**
 * Attach, inside the pill: the paperclip opens a menu of the system's pickers (Photo or video, through the photo
 * picker; File, through the documents UI), neither needing a storage permission; what the user picks goes to
 * [onPick]. It holds while an upload is on its way ([enabled]).
 */
@Composable
internal fun AttachButton(
    enabled: Boolean,
    onPick: (Uri) -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    val media =
        rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri -> uri?.let(onPick) }
    val document =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri -> uri?.let(onPick) }
    val label = stringResource(R.string.chat_attach)
    Box(modifier) {
        IconButton(
            onClick = { open = true },
            enabled = enabled,
            modifier = Modifier.semantics { contentDescription = label },
        ) {
            Icon(painterResource(R.drawable.ic_attach), contentDescription = null)
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.chat_attach_media)) },
                onClick = {
                    open = false
                    media.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
                },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.chat_attach_file)) },
                onClick = {
                    open = false
                    document.launch(arrayOf("*/*"))
                },
            )
        }
    }
}

/**
 * The attached file over the pill: on its way, its name, how much has gone and Cancel, over a thin bar; stopped
 * short, why (in the Error tone, announced politely) and Dismiss.
 */
@Composable
internal fun UploadLine(
    upload: ChatUpload,
    onCancel: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(modifier.fillMaxWidth().padding(horizontal = spacing.xs)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Icon(
                painterResource(R.drawable.ic_attach),
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(16.dp),
            )
            when (upload) {
                is ChatUpload.Sending -> Sending(upload, onCancel, Modifier.weight(1f))
                is ChatUpload.Failed -> Failed(upload, onDismiss, Modifier.weight(1f))
            }
        }
        if (upload is ChatUpload.Sending) {
            val share = if (upload.total > 0) upload.sent.toFloat() / upload.total else 0f
            LinearProgressIndicator(progress = { share }, modifier = Modifier.fillMaxWidth())
        }
    }
}

@Composable
private fun Sending(
    upload: ChatUpload.Sending,
    onCancel: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val percent = if (upload.total > 0) (upload.sent * PERCENT / upload.total).toInt() else 0
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Text(
            stringResource(R.string.chat_upload_sending, upload.name),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurface,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        Text(
            stringResource(R.string.chat_upload_percent, percent),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        LineButton(stringResource(R.string.chat_upload_cancel), onCancel)
    }
}

// a share as a percentage
private const val PERCENT = 100

@Composable
private fun Failed(
    upload: ChatUpload.Failed,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val why =
        when (val problem = upload.problem) {
            UploadProblem.TooLarge -> stringResource(R.string.chat_upload_too_large)
            UploadProblem.Unreadable -> stringResource(R.string.chat_upload_unreadable)
            is UploadProblem.Refused -> problem.reason
        }
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Text(
            stringResource(R.string.chat_upload_failed, upload.name, why),
            style = MaterialTheme.typography.bodySmall,
            color = NgtermTheme.status.error,
            modifier = Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite },
        )
        LineButton(stringResource(R.string.action_dismiss), onDismiss)
    }
}

/** A file the user picked: its [name] as the picker gives it, and its [size] in bytes when known. */
data class PickedFile(
    val name: String,
    val size: Long?,
)

/** What [resolver] says of the picked [uri]: its display name (its last path segment without one) and size. */
fun pickedFile(
    resolver: ContentResolver,
    uri: Uri,
): PickedFile {
    val columns = arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
    val fallback =
        uri.lastPathSegment
            ?.substringAfterLast('/')
            .orEmpty()
            .ifEmpty { "upload" }
    return try {
        resolver.query(uri, columns, null, null, null)?.use { cursor ->
            if (!cursor.moveToFirst()) return@use null
            val name = cursor.getString(0)?.takeIf(String::isNotBlank) ?: fallback
            val size = if (cursor.isNull(1)) null else cursor.getLong(1)
            PickedFile(name, size)
        } ?: PickedFile(fallback, null)
    } catch (_: SecurityException) {
        PickedFile(fallback, null)
    }
}

/**
 * The picked [uri]'s bytes, read through [resolver]: no more than one past [UPLOAD_CAP], so a file whose size wasn't
 * known still can't fill the memory before it is refused. Off the main thread.
 *
 * @throws IOException when the file can't be opened or read.
 */
fun readPicked(
    resolver: ContentResolver,
    uri: Uri,
): ByteArray {
    val input = resolver.openInputStream(uri) ?: throw IOException("no stream for $uri")
    return input.use { stream ->
        val out = ByteArrayOutputStream()
        val buffer = ByteArray(READ_CHUNK)
        while (out.size() <= UPLOAD_CAP) {
            val read = stream.read(buffer)
            if (read < 0) break
            out.write(buffer, 0, read)
        }
        out.toByteArray()
    }
}

// how much of a picked file one read takes
private const val READ_CHUNK = 64 * 1024

// a 48dp Close at the line's end, named for what it does
@Composable
private fun LineButton(
    label: String,
    onClick: () -> Unit,
) {
    IconButton(onClick = onClick, modifier = Modifier.semantics { contentDescription = label }) {
        Icon(painterResource(R.drawable.ic_close), contentDescription = null, modifier = Modifier.size(18.dp))
    }
}
