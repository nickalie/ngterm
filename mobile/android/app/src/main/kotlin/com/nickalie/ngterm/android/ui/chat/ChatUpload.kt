package com.nickalie.ngterm.android.ui.chat

import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import com.nickalie.ngterm.android.data.reason
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.UploadProgress
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.IOException

/** The most bytes a file the composer attaches may hold: the origin's cap (`ngtermctl agent upload`). */
const val UPLOAD_CAP: Long = 32L * 1024 * 1024

/** A file the composer attaches, on its way to the machine ([Sending]) or stopped short of it ([Failed]). */
sealed interface ChatUpload {
    /** The file's name, as the user picked it. */
    val name: String

    /**
     * [sent] of [total] bytes have gone; [saving] once the core has told every one gone: the machine has the whole
     * file then and stores it, and a cancel could no longer take it back, so none is offered.
     */
    data class Sending(
        override val name: String,
        val sent: Long,
        val total: Long,
        val saving: Boolean = false,
    ) : ChatUpload

    /** Why the file didn't reach the machine, until the user dismisses it. */
    data class Failed(
        override val name: String,
        val problem: UploadProblem,
    ) : ChatUpload
}

/** Why an upload stored nothing. */
sealed interface UploadProblem {
    /** Over [UPLOAD_CAP]. */
    data object TooLarge : UploadProblem

    /** The phone couldn't read the file it was given. */
    data object Unreadable : UploadProblem

    /** The machine or the link said no: in its words. */
    data class Refused(
        val reason: String,
    ) : UploadProblem
}

/**
 * A pane's chat's uploads, one at a time, into [chat]'s [ChatUiState.upload]: the file's bytes read off the main
 * thread, sent to [machineId] through [core], and the stored path handed to [onStored] (the composer puts it into the
 * prompt). Its work runs in [scope]; [cancel] stops the one on its way while bytes of it are still to go, and the
 * machine keeps nothing of it. Once every byte has gone ([ChatUpload.Sending.saving]) the machine stores the file
 * whatever the phone does, so [cancel] holds off and the upload runs to its path.
 */
internal class ChatUploads(
    private val core: MobileCoreInterface,
    private val machineId: String,
    private val chat: MutableStateFlow<ChatUiState>,
    private val scope: CoroutineScope,
    private val onStored: (String) -> Unit,
) {
    private var job: Job? = null

    /** Sends the file [name] of [size] bytes (`null`: unknown) that [read] gives, unless one is on its way. */
    fun attach(
        name: String,
        size: Long?,
        read: suspend () -> ByteArray,
    ) {
        if (job?.isActive == true) return
        if (size != null && size > UPLOAD_CAP) return failed(name, UploadProblem.TooLarge)
        chat.update { it.copy(upload = ChatUpload.Sending(name, 0, size ?: 0)) }
        job = scope.launch { send(name, read) }
    }

    /** Stops the upload on its way, if any, unless it is [saving][ChatUpload.Sending.saving]: too late then. */
    fun cancel() {
        if ((chat.value.upload as? ChatUpload.Sending)?.saving == true) return
        job?.cancel()
        job = null
        chat.update { if (it.upload is ChatUpload.Sending) it.copy(upload = null) else it }
    }

    /** The failure was seen: it goes. */
    fun shown() = chat.update { if (it.upload is ChatUpload.Failed) it.copy(upload = null) else it }

    private suspend fun send(
        name: String,
        read: suspend () -> ByteArray,
    ) {
        val bytes = readOrNull(read)
        val outcome =
            when {
                bytes == null -> Outcome.Stopped(UploadProblem.Unreadable)
                bytes.size > UPLOAD_CAP -> Outcome.Stopped(UploadProblem.TooLarge)
                else -> store(name, bytes)
            }
        when (outcome) {
            is Outcome.Stored -> {
                chat.update { it.copy(upload = null) }
                onStored(outcome.path)
            }

            is Outcome.Stopped -> {
                failed(name, outcome.problem)
            }
        }
    }

    // the file's bytes, off the main thread; `null` when the phone can't read them
    private suspend fun readOrNull(read: suspend () -> ByteArray): ByteArray? =
        try {
            withContext(Dispatchers.IO) { read() }
        } catch (_: IOException) {
            null
        } catch (_: SecurityException) {
            null
        }

    // the bytes, sent; a cancel goes on up, so the core drops the upload
    private suspend fun store(
        name: String,
        bytes: ByteArray,
    ): Outcome =
        try {
            Outcome.Stored(core.uploadFile(machineId, name, bytes, Progress(name)))
        } catch (error: CancellationException) {
            throw error
        } catch (error: MobileException) {
            Outcome.Stopped(UploadProblem.Refused(error.reason()))
        }

    // how an upload ended
    private sealed interface Outcome {
        data class Stored(
            val path: String,
        ) : Outcome

        data class Stopped(
            val problem: UploadProblem,
        ) : Outcome
    }

    private fun failed(
        name: String,
        problem: UploadProblem,
    ) = chat.update { it.copy(upload = ChatUpload.Failed(name, problem)) }

    // the core's word on how much has gone, for the upload [name] still showing
    private inner class Progress(
        private val name: String,
    ) : UploadProgress {
        override fun sent(
            sent: ULong,
            total: ULong,
        ) = chat.update {
            val now = it.upload as? ChatUpload.Sending
            if (now?.name == name) {
                it.copy(upload = now.copy(sent = sent.toLong(), total = total.toLong(), saving = sent >= total))
            } else {
                it
            }
        }
    }
}

/**
 * Puts an attached file's [path] at the composer's cursor (in place of a selection), a space between it and the words
 * on either side, the cursor after it.
 */
fun ChatComposer.insertPath(path: String) {
    val text = draft.text
    val start = draft.selection.min.coerceIn(0, text.length)
    val end = draft.selection.max.coerceIn(0, text.length)
    val before = text.substring(0, start)
    val after = text.substring(end)
    val lead = if (before.isEmpty() || before.last().isWhitespace()) "" else " "
    val inserted = "$lead$path "
    edit(TextFieldValue(before + inserted + after.trimStart(' '), TextRange(before.length + inserted.length)))
}

/**
 * [path] as it goes into a prompt: as is, or in double quotes when it holds a space, so the agent reads it as one
 * path.
 */
fun promptPath(path: String): String = if (path.any(Char::isWhitespace)) "\"$path\"" else path
