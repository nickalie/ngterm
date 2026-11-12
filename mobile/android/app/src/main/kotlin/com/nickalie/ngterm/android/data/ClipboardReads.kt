package com.nickalie.ngterm.android.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringSetPreferencesKey
import com.nickalie.ngterm.core.ffi.ClipboardPart
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PaneRef
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.flow.updateAndGet
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.concurrent.ConcurrentHashMap
import kotlin.coroutines.CoroutineContext

/**
 * The device's clipboard per MIME type, as a program's reads see it (text as `text/plain`): which types it holds, and
 * each of [read]'s types it holds, in that order; none when it holds none (or the app may not read it).
 */
interface DeviceClipboard {
    fun types(): List<String>

    fun read(mimes: List<String>): List<ClipboardPart>
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

/**
 * A program's read of the clipboard waiting for the user's answer: the [mimes] it wants (text as `text/plain`) and,
 * with [list], the types the clipboard holds; [canRemember] when an Allow may keep a grant for the program in its
 * terminal (the Kitty protocol's password). [listOnly] reads ask only which types it holds.
 */
data class ClipboardRead(
    val pane: PaneRef,
    val readId: ULong,
    val listOnly: Boolean,
    val mimes: List<String> = listOf("text/plain"),
    val list: Boolean = false,
    val canRemember: Boolean = false,
)

/** What the clipboard holds of a read's types: each one it holds, and the types it holds when the read asked. */
data class ClipboardSnapshot(
    val contents: List<ClipboardPart>,
    val available: List<String>,
)

/**
 * The clipboard reads of shown panes' programs (Ghostty's `clipboard-read = ask`): each waits in [pending] for the
 * user's Allow, Always or Deny, unless its machine is in [trust] or the program holds a grant in its terminal, which
 * answer it at once. A read that discloses no data (a listing of the types, or one asking for no type) is answered at
 * once too. The prompt shows what the read would hand over ([preview]); its Allow hands over exactly that. A read the
 * core dismisses (its pane hidden, detached, lost or exited: the core denied it) leaves [pending]. The core's own
 * thread delivers the requests and dismissals in order, and both are taken on it, so a dismissal never overtakes its
 * request. The clipboard is never read on that thread (another app's image is a content provider's stream, which may
 * take its time and would stop every machine's traffic with it): an unasked read's snapshot is taken on [io], one
 * read at a time in the order they came, and a read dismissed meanwhile goes unanswered (one the clipboard fails
 * to hand over is refused).
 */
class ClipboardReads(
    private val scope: CoroutineScope,
    private val connections: Connections,
    private val trust: ClipboardTrust,
    private val clipboard: DeviceClipboard,
    private val io: CoroutineContext = Dispatchers.IO,
) {
    private val pendingState = MutableStateFlow<List<ClipboardRead>>(emptyList())

    // what each read's prompt showed, kept so the prompt shows the same snapshot again when it comes back
    private val shown = ConcurrentHashMap<ClipboardRead, ClipboardSnapshot>()

    // the trusted machines, read on the core's thread: the store's, and each Always at once, until the store has it.
    // Until the store's first value lands, every read asks (the safe side).
    private val trustedState = MutableStateFlow<Set<String>>(emptySet())
    private val unsaved = MutableStateFlow<Set<String>>(emptySet())

    // the reads answered unasked whose snapshot is still to be taken, in the order they came; a dismissal takes a read
    // out of [unanswered], and only a read still in it is answered
    private val unasked = Channel<ClipboardRead>(Channel.UNLIMITED)
    private val unanswered: MutableSet<ClipboardRead> = ConcurrentHashMap.newKeySet()

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
        // the reads [answerUnasked] queued, answered in the order they came, each snapshot taken on [io]
        scope.launch {
            for (read in unasked) {
                withContext(io) {
                    if (read in unanswered) {
                        val reply = unaskedReply { snapshot(read) }
                        if (unanswered.remove(read)) reply(read, reply)
                    }
                }
            }
        }
        connections.listen(::onEvent)
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.ClipboardReadRequest -> {
                val read =
                    ClipboardRead(event.pane, event.readId, event.listOnly, event.mimes, event.list, event.canRemember)
                val discloses = !read.listOnly && read.mimes.isNotEmpty()
                if (!discloses || event.granted || event.pane.machineId in trusted.value) {
                    answerUnasked(read)
                } else {
                    pendingState.update { it + read }
                }
            }

            is MobileEvent.ClipboardReadDismiss -> {
                pendingState.update { reads ->
                    reads.filterNot { it.pane == event.pane && it.readId == event.readId }
                }
                shown.keys.removeAll { it.pane == event.pane && it.readId == event.readId }
                unanswered.removeAll { it.pane == event.pane && it.readId == event.readId }
            }

            else -> {}
        }
    }

    /**
     * What [read]'s Allow would hand over, read once (off the main thread) and kept for that Allow; null when the
     * clipboard failed to read (a provider's or the system clipboard's RuntimeException), which refuses [read] as an
     * unasked read's failure does: a failed read is not an empty clipboard.
     */
    suspend fun preview(read: ClipboardRead): ClipboardSnapshot? =
        shown[read] ?: try {
            withContext(io) { snapshot(read) }.also { if (read in pending.value) shown[read] = it }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: RuntimeException) {
            if (read in pending.value) deny(read)
            null
        }

    /**
     * Hands [read]'s program [shown], what its prompt showed ([preview]), this once: never a fresh read the user didn't
     * see. [remember] keeps a grant for the program in its terminal, where the read can keep one.
     */
    fun allow(
        read: ClipboardRead,
        shown: ClipboardSnapshot,
        remember: Boolean = false,
    ) {
        this.shown.remove(read)
        answer(read, shown.allowed(remember && read.canRemember))
    }

    /**
     * Allows [read] as [allow] does, and lets its machine's programs read the clipboard from now on unasked: the
     * machine's other reads already waiting are answered too, as unasked reads are.
     */
    fun allowAlways(
        read: ClipboardRead,
        shown: ClipboardSnapshot,
        remember: Boolean = false,
    ) {
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
        this.shown.remove(read)
        reply(read, shown.allowed(remember && read.canRemember))
        for (each in answered - read) {
            this.shown.remove(each)
            answerUnasked(each)
        }
    }

    /** The machines whose programs read the clipboard unasked, as stored: what Settings lists. */
    val alwaysAllowed: Flow<Set<String>> get() = trust.machines

    /** Asks again before [machineId]'s programs read the clipboard (Settings' Ask again). */
    suspend fun revoke(machineId: String) {
        unsaved.update { it - machineId }
        trustedState.update { it - machineId }
        trust.revoke(machineId)
    }

    /** Refuses [read]: its program reads an empty clipboard (OSC 52) or is told it was refused. */
    fun deny(read: ClipboardRead) {
        shown.remove(read)
        answer(read, ClipboardReply.Deny)
    }

    // answers [read] with a snapshot taken off the caller's thread (the core's, or the main thread)
    private fun answerUnasked(read: ClipboardRead) {
        unanswered += read
        unasked.trySend(read)
    }

    private fun snapshot(read: ClipboardRead) =
        ClipboardSnapshot(
            contents = if (read.listOnly || read.mimes.isEmpty()) emptyList() else clipboard.read(read.mimes),
            available = if (read.list) clipboard.types().take(MAX_LISTED) else emptyList(),
        )

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

    private companion object {
        // the most types a listing names (Ghostty's clipboard_response.zig)
        const val MAX_LISTED = 16
    }
}

/** The Allow that hands over this snapshot, keeping a grant with [remember]. */
private fun ClipboardSnapshot.allowed(remember: Boolean) = ClipboardReply.Allow(contents, available, remember)

// an unasked read's Allow of what [take] reads; a provider's or the system clipboard's failure refuses this read, not
// every later one
private inline fun unaskedReply(take: () -> ClipboardSnapshot): ClipboardReply =
    try {
        take().allowed(remember = false)
    } catch (_: RuntimeException) {
        ClipboardReply.Deny
    }
