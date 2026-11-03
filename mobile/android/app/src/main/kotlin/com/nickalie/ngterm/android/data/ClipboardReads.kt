package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringSetPreferencesKey
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.updateAndGet
import kotlinx.coroutines.launch

/** The device's clipboard as text: what it holds now, `null` when it holds no text (or the app may not read it). */
fun interface ClipboardText {
    fun read(): String?
}

/**
 * The machines whose programs may read the clipboard without asking ("Always allow on build-01"), kept in [store]
 * (Preferences DataStore) by machine id. Settings lists and revokes them.
 */
class ClipboardTrust(
    private val store: DataStore<Preferences>,
) {
    /** The trusted machines' ids. */
    val machines: Flow<Set<String>> = store.data.map { it[TRUSTED].orEmpty() }

    /** Lets [machineId]'s programs read the clipboard without asking. */
    suspend fun allow(machineId: String) {
        store.edit { it[TRUSTED] = it[TRUSTED].orEmpty() + machineId }
    }

    /** Asks again before [machineId]'s programs read the clipboard. */
    suspend fun revoke(machineId: String) {
        store.edit { it[TRUSTED] = it[TRUSTED].orEmpty() - machineId }
    }

    private companion object {
        val TRUSTED = stringSetPreferencesKey("clipboard_read_machines")
    }
}

/** A program's read of the clipboard (OSC 52) waiting for the user's answer: [listOnly] asks only what it holds. */
data class ClipboardRead(
    val pane: PaneRef,
    val readId: ULong,
    val listOnly: Boolean,
)

/**
 * The clipboard reads of shown panes' programs (Ghostty's `clipboard-read = ask`): each waits in [pending] for the
 * user's Allow, Always or Deny, unless its machine is in [trust], which answers it at once. A read the core
 * dismisses (its pane hidden, detached, lost or exited: the core denied it) leaves [pending]. The core's own
 * thread delivers the requests and dismissals in order, and both are taken on it, so a dismissal never overtakes its
 * request.
 */
class ClipboardReads(
    private val scope: CoroutineScope,
    private val connections: Connections,
    private val trust: ClipboardTrust,
    private val clipboard: ClipboardText,
) {
    private val pendingState = MutableStateFlow<List<ClipboardRead>>(emptyList())

    // the trusted machines, read on the core's thread: the store's, and each Always at once, until the store has it.
    // Until the store's first value lands, every read asks (the safe side).
    private val trustedState = MutableStateFlow<Set<String>>(emptySet())
    private val unsaved = MutableStateFlow<Set<String>>(emptySet())

    /** The machines whose reads are answered unasked, as this class last heard. */
    internal val trusted: StateFlow<Set<String>> = trustedState.asStateFlow()

    /** The reads waiting for an answer, oldest first: the prompt shows the first. */
    val pending: StateFlow<List<ClipboardRead>> = pendingState.asStateFlow()

    init {
        scope.launch {
            trust.machines.collect { stored ->
                val waiting = unsaved.updateAndGet { it - stored }
                trustedState.value = stored + waiting
            }
        }
        connections.listen(::onEvent)
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.ClipboardReadRequest -> {
                val read = ClipboardRead(event.pane, event.readId, event.listOnly)
                if (event.pane.machineId in trusted.value) {
                    reply(read, ClipboardReply.Allow(clipboard.read()))
                } else {
                    pendingState.update { it + read }
                }
            }

            is MobileEvent.ClipboardReadDismiss -> {
                pendingState.update { reads -> reads.filterNot { it.pane == event.pane && it.readId == event.readId } }
            }

            else -> {}
        }
    }

    /** Hands [read]'s program the clipboard's text, this once. */
    fun allow(read: ClipboardRead) = answer(read, ClipboardReply.Allow(clipboard.read()))

    /**
     * Hands [read]'s program the clipboard's text, and lets its machine's programs read it from now on unasked: the
     * machine's other reads already waiting are answered with it.
     */
    fun allowAlways(read: ClipboardRead) {
        val machine = read.pane.machineId
        unsaved.update { it + machine }
        trustedState.update { it + machine }
        scope.launch { trust.allow(machine) }
        var answered = emptyList<ClipboardRead>()
        pendingState.update { reads ->
            val (same, others) = reads.partition { it.pane.machineId == machine }
            answered = same
            others
        }
        val text = clipboard.read()
        for (each in (listOf(read) + answered).distinct()) reply(each, ClipboardReply.Allow(text))
    }

    /** Refuses [read]: its program reads an empty clipboard. */
    fun deny(read: ClipboardRead) = answer(read, ClipboardReply.Deny)

    private fun answer(
        read: ClipboardRead,
        reply: ClipboardReply,
    ) {
        pendingState.update { it - read }
        reply(read, reply)
    }

    private fun reply(
        read: ClipboardRead,
        reply: ClipboardReply,
    ) {
        try {
            connections.core.answerClipboardRead(
                read.pane.machineId,
                read.pane.sessionId,
                read.pane.paneId,
                read.readId,
                reply,
            )
        } catch (_: MobileException) {
            // the core has stopped: no program is left waiting
        }
    }
}
